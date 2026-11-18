//! Lightweight multi-core scene-distance analysis for GTA SA.
//!
//! GTA, RenderWare, physics, and Direct3D remain on the game thread. The
//! SetupMapEntityVisibility hook copies only scalar distances and visibility
//! result codes into a thread-local frame buffer. Workers classify those scalar
//! samples off-thread; no GTA pointer or engine object crosses the boundary.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows::Win32::System::Threading::{
    GetCurrentProcessorNumber, GetCurrentProcessId, GetCurrentThreadId,
};

const MAX_SAMPLES_PER_FRAME: usize = 16_384;
const MAX_WORKERS: usize = 7;
const STATUS_REFRESH_INTERVAL: Duration = Duration::from_millis(250);

thread_local! {
    // Keep capacity between frames. The frame's vector is handed to workers
    // only after capture has finished for that Present boundary.
    static FRAME_CAPTURE: RefCell<FrameCapture> =
        RefCell::new(FrameCapture::with_capacity(2_048));
}

struct FrameCapture {
    distances: Vec<f32>,
    map_visibility_calls: u64,
    visibility_results: [u64; 4],
    valid_distances: u64,
    invalid_distances: u64,
    sample_cap_hits: u64,
    capture_thread_id: u32,
    last_distance: f32,
    max_distance: f32,
}

impl FrameCapture {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            distances: Vec::with_capacity(capacity),
            map_visibility_calls: 0,
            visibility_results: [0; 4],
            valid_distances: 0,
            invalid_distances: 0,
            sample_cap_hits: 0,
            capture_thread_id: 0,
            last_distance: 0.0,
            max_distance: 0.0,
        }
    }
}

struct Job {
    distances: Arc<Vec<f32>>,
    start: usize,
    end: usize,
}

struct WorkerStats {
    last_processor: AtomicU32,
    thread_id: AtomicU32,
    processors_seen: AtomicU64,
    jobs: AtomicU64,
    entities: AtomicU64,
    elapsed_ns: AtomicU64,
    near_entities: AtomicU64,
    mid_entities: AtomicU64,
    far_entities: AtomicU64,
}

impl WorkerStats {
    fn new() -> Self {
        Self {
            last_processor: AtomicU32::new(u32::MAX),
            thread_id: AtomicU32::new(0),
            processors_seen: AtomicU64::new(0),
            jobs: AtomicU64::new(0),
            entities: AtomicU64::new(0),
            elapsed_ns: AtomicU64::new(0),
            near_entities: AtomicU64::new(0),
            mid_entities: AtomicU64::new(0),
            far_entities: AtomicU64::new(0),
        }
    }
}

struct Worker {
    sender: SyncSender<Job>,
    stats: Arc<WorkerStats>,
}

struct Pool {
    workers: Vec<Worker>,
    last_frame_entities: AtomicUsize,
    frames_submitted: AtomicU64,
    chunks_submitted: AtomicU64,
    chunks_dropped: AtomicU64,
    present_calls: AtomicU64,
    empty_frames: AtomicU64,
    frames_with_samples: AtomicU64,
    max_samples_per_frame: AtomicUsize,
    map_visibility_calls: AtomicU64,
    visibility_results: [AtomicU64; 4],
    valid_distances: AtomicU64,
    invalid_distances: AtomicU64,
    sample_cap_hits: AtomicU64,
    total_samples_captured: AtomicU64,
    last_capture_thread: AtomicU32,
    last_present_thread: AtomicU32,
    last_distance_bits: AtomicU32,
    max_distance_bits: AtomicU32,
}

static POOL: OnceLock<Pool> = OnceLock::new();

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub worker_count: usize,
    /// Number of workers that have completed at least one job this session.
    pub active_workers: usize,
    pub cpu_cores: String,
    pub worker_jobs: String,
    pub last_frame_entities: usize,
    pub processed_entities: u64,
    pub frames_submitted: u64,
    pub chunks_submitted: u64,
    pub chunks_dropped: u64,
    pub worker_time_ms: u64,
    pub near_entities: u64,
    pub mid_entities: u64,
    pub far_entities: u64,
    pub map_visibility_calls: u64,
    pub visibility_results: [u64; 4],
    pub valid_distances: u64,
    pub invalid_distances: u64,
    pub sample_cap_hits: u64,
    pub total_samples_captured: u64,
    pub present_calls: u64,
    pub empty_frames: u64,
    pub frames_with_samples: u64,
    pub max_samples_per_frame: usize,
    pub last_capture_thread: u32,
    pub last_present_thread: u32,
    pub last_distance: f32,
    pub max_distance: f32,
}

#[derive(Default)]
struct StatusCache {
    updated_at: Option<Instant>,
    value: Status,
}

static STATUS_CACHE: OnceLock<Mutex<StatusCache>> = OnceLock::new();

pub fn initialize() {
    if POOL.get().is_some() {
        return;
    }

    let mut system_info = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut system_info) };
    let system_processors = system_info.dwNumberOfProcessors.max(1) as usize;
    let available_processors = thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(system_processors);
    let requested_workers = available_processors
        .saturating_sub(1)
        .clamp(1, MAX_WORKERS);

    let process_id = unsafe { GetCurrentProcessId() };
    let mut workers = Vec::with_capacity(requested_workers);

    for index in 0..requested_workers {
        let (sender, receiver) = mpsc::sync_channel::<Job>(1);
        let stats = Arc::new(WorkerStats::new());
        let worker_stats = Arc::clone(&stats);
        let name = format!("GTA-MC-Scene-{index}");

        match thread::Builder::new().name(name).spawn(move || {
            let worker_tid = unsafe { GetCurrentThreadId() };
            let initial_cpu = unsafe { GetCurrentProcessorNumber() };
            worker_stats.thread_id.store(worker_tid, Ordering::Release);
            crate::config::log(&format!(
                "Scene worker started: index={index}, tid={worker_tid}, initial_cpu={initial_cpu}"
            ));

            while let Ok(job) = receiver.recv() {
                let started = Instant::now();
                let processor = unsafe { GetCurrentProcessorNumber() };
                let mut near = 0_u64;
                let mut mid = 0_u64;
                let mut far = 0_u64;

                for distance in &job.distances[job.start..job.end] {
                    match distance_band(*distance) {
                        0 => near += 1,
                        1 => mid += 1,
                        _ => far += 1,
                    }
                }

                let count = (job.end - job.start) as u64;
                let elapsed = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                worker_stats.last_processor.store(processor, Ordering::Release);
                if processor < 64 {
                    worker_stats.processors_seen.fetch_or(1_u64 << processor, Ordering::Relaxed);
                }
                worker_stats.jobs.fetch_add(1, Ordering::Relaxed);
                worker_stats.entities.fetch_add(count, Ordering::Relaxed);
                worker_stats.elapsed_ns.fetch_add(elapsed, Ordering::Relaxed);
                worker_stats.near_entities.fetch_add(near, Ordering::Relaxed);
                worker_stats.mid_entities.fetch_add(mid, Ordering::Relaxed);
                worker_stats.far_entities.fetch_add(far, Ordering::Relaxed);
            }
        }) {
            Ok(_thread) => workers.push(Worker { sender, stats }),
            Err(error) => {
                crate::config::log(&format!(
                    "Scene worker startup failed: index={index}, error={error}"
                ));
                break;
            }
        }
    }

    if workers.is_empty() {
        crate::config::log("Multicore scene analysis unavailable: no worker threads started");
        return;
    }

    let count = workers.len();
    let _ = POOL.set(Pool {
        workers,
        last_frame_entities: AtomicUsize::new(0),
        frames_submitted: AtomicU64::new(0),
        chunks_submitted: AtomicU64::new(0),
        chunks_dropped: AtomicU64::new(0),
        present_calls: AtomicU64::new(0),
        empty_frames: AtomicU64::new(0),
        frames_with_samples: AtomicU64::new(0),
        max_samples_per_frame: AtomicUsize::new(0),
        map_visibility_calls: AtomicU64::new(0),
        visibility_results: std::array::from_fn(|_| AtomicU64::new(0)),
        valid_distances: AtomicU64::new(0),
        invalid_distances: AtomicU64::new(0),
        sample_cap_hits: AtomicU64::new(0),
        total_samples_captured: AtomicU64::new(0),
        last_capture_thread: AtomicU32::new(0),
        last_present_thread: AtomicU32::new(0),
        last_distance_bits: AtomicU32::new(0),
        max_distance_bits: AtomicU32::new(0),
    });

    crate::config::log(&format!(
        "Multicore scene analysis ready: workers={count}/{requested_workers}, logical_processors={system_processors}, process_id={process_id}, one_logical_processor_reserved=true"
    ));
}

/// Called after GTA's original SetupMapEntityVisibility function returns.
/// The hook copies only scalar values into a thread-local frame buffer. No
/// per-entity atomics, system calls, file I/O, or worker synchronization occur.
pub fn record_map_visibility_sample(distance: f32, result: i32) {
    FRAME_CAPTURE.with(|capture| {
        let mut capture = capture.borrow_mut();
        if capture.map_visibility_calls == 0 {
            capture.capture_thread_id = unsafe { GetCurrentThreadId() };
        }
        capture.map_visibility_calls += 1;

        if (0..4).contains(&result) {
            capture.visibility_results[result as usize] += 1;
        }

        if !distance.is_finite() || distance < 0.0 {
            capture.invalid_distances += 1;
            return;
        }

        capture.valid_distances += 1;
        capture.last_distance = distance;
        capture.max_distance = capture.max_distance.max(distance);

        if capture.distances.len() < MAX_SAMPLES_PER_FRAME {
            capture.distances.push(distance);
        } else {
            capture.sample_cap_hits += 1;
        }
    });
}

/// Called at Present. Frame counters are merged in batches and worker
/// submission is non-blocking: a busy worker drops its chunk instead of
/// stalling GTA's render thread.
pub fn submit_frame() {
    let Some(pool) = POOL.get() else {
        return;
    };

    let present_thread = unsafe { GetCurrentThreadId() };
    let (
        samples,
        map_visibility_calls,
        visibility_results,
        valid_distances,
        invalid_distances,
        sample_cap_hits,
        capture_thread,
        last_distance,
        max_distance,
    ) = FRAME_CAPTURE.with(|slot| {
        let mut capture = slot.borrow_mut();
        let samples = capture.distances.split_off(0);
        let frame = (
            samples,
            capture.map_visibility_calls,
            capture.visibility_results,
            capture.valid_distances,
            capture.invalid_distances,
            capture.sample_cap_hits,
            capture.capture_thread_id,
            capture.last_distance,
            capture.max_distance,
        );

        capture.map_visibility_calls = 0;
        capture.visibility_results = [0; 4];
        capture.valid_distances = 0;
        capture.invalid_distances = 0;
        capture.sample_cap_hits = 0;
        capture.capture_thread_id = 0;
        capture.last_distance = 0.0;
        capture.max_distance = 0.0;
        frame
    });

    pool.present_calls.fetch_add(1, Ordering::Relaxed);
    pool.last_present_thread.store(present_thread, Ordering::Release);
    if capture_thread != 0 {
        pool.last_capture_thread.store(capture_thread, Ordering::Release);
    }

    pool.map_visibility_calls.fetch_add(map_visibility_calls, Ordering::Relaxed);
    for (index, count) in visibility_results.iter().enumerate() {
        pool.visibility_results[index].fetch_add(*count, Ordering::Relaxed);
    }
    pool.valid_distances.fetch_add(valid_distances, Ordering::Relaxed);
    pool.invalid_distances.fetch_add(invalid_distances, Ordering::Relaxed);
    pool.sample_cap_hits.fetch_add(sample_cap_hits, Ordering::Relaxed);
    pool.total_samples_captured.fetch_add(samples.len() as u64, Ordering::Relaxed);

    if valid_distances > 0 {
        pool.last_distance_bits.store(last_distance.to_bits(), Ordering::Release);
        pool.max_distance_bits.fetch_max(max_distance.to_bits(), Ordering::Relaxed);
    }

    pool.last_frame_entities.store(samples.len(), Ordering::Relaxed);
    pool.max_samples_per_frame.fetch_max(samples.len(), Ordering::Relaxed);

    if samples.is_empty() {
        pool.empty_frames.fetch_add(1, Ordering::Relaxed);
        return;
    }

    pool.frames_with_samples.fetch_add(1, Ordering::Relaxed);
    pool.frames_submitted.fetch_add(1, Ordering::Relaxed);

    let worker_count = pool.workers.len();
    let chunk_size = samples.len().div_ceil(worker_count);
    let distances = Arc::new(samples);

    for (index, worker) in pool.workers.iter().enumerate() {
        let start = index * chunk_size;
        let end = (start + chunk_size).min(distances.len());
        if start >= end {
            continue;
        }

        let job = Job {
            distances: Arc::clone(&distances),
            start,
            end,
        };
        match worker.sender.try_send(job) {
            Ok(()) => {
                pool.chunks_submitted.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
                pool.chunks_dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

fn distance_band(distance: f32) -> u8 {
    if distance < 100.0 {
        0
    } else if distance < 350.0 {
        1
    } else {
        2
    }
}

pub fn status() -> Status {
    let Some(pool) = POOL.get() else {
        return Status::default();
    };

    let cache = STATUS_CACHE.get_or_init(|| Mutex::new(StatusCache::default()));
    let now = Instant::now();
    let mut cached = cache.lock().unwrap_or_else(|error| error.into_inner());
    if cached.updated_at.is_some_and(|updated| {
        now.saturating_duration_since(updated) < STATUS_REFRESH_INTERVAL
    }) {
        return cached.value.clone();
    }

    let mut active_workers = 0_usize;
    let mut processors_mask = 0_u64;
    let mut processed_entities = 0_u64;
    let mut worker_time_ns = 0_u64;
    let mut near_entities = 0_u64;
    let mut mid_entities = 0_u64;
    let mut far_entities = 0_u64;
    let mut worker_jobs = Vec::with_capacity(pool.workers.len());

    for (index, worker) in pool.workers.iter().enumerate() {
        let jobs = worker.stats.jobs.load(Ordering::Relaxed);
        if jobs > 0 {
            active_workers += 1;
        }
        processors_mask |= worker.stats.processors_seen.load(Ordering::Relaxed);
        let processor = worker.stats.last_processor.load(Ordering::Acquire);
        let thread_id = worker.stats.thread_id.load(Ordering::Acquire);
        let processor_label = if processor == u32::MAX {
            "PENDING".to_owned()
        } else {
            processor.to_string()
        };
        worker_jobs.push(format!("W{index}:J{jobs}@CPU{processor_label}#T{thread_id}"));
        processed_entities += worker.stats.entities.load(Ordering::Relaxed);
        worker_time_ns += worker.stats.elapsed_ns.load(Ordering::Relaxed);
        near_entities += worker.stats.near_entities.load(Ordering::Relaxed);
        mid_entities += worker.stats.mid_entities.load(Ordering::Relaxed);
        far_entities += worker.stats.far_entities.load(Ordering::Relaxed);
    }

    let processors = (0..64)
        .filter(|processor| processors_mask & (1_u64 << *processor) != 0)
        .map(|processor| processor.to_string())
        .collect::<Vec<_>>();

    let value = Status {
        worker_count: pool.workers.len(),
        active_workers,
        cpu_cores: if processors.is_empty() {
            "pending".to_owned()
        } else {
            processors.join(",")
        },
        worker_jobs: worker_jobs.join(";"),
        last_frame_entities: pool.last_frame_entities.load(Ordering::Relaxed),
        processed_entities,
        frames_submitted: pool.frames_submitted.load(Ordering::Relaxed),
        chunks_submitted: pool.chunks_submitted.load(Ordering::Relaxed),
        chunks_dropped: pool.chunks_dropped.load(Ordering::Relaxed),
        worker_time_ms: worker_time_ns / 1_000_000,
        near_entities,
        mid_entities,
        far_entities,
        map_visibility_calls: pool.map_visibility_calls.load(Ordering::Relaxed),
        visibility_results: std::array::from_fn(|index| {
            pool.visibility_results[index].load(Ordering::Relaxed)
        }),
        valid_distances: pool.valid_distances.load(Ordering::Relaxed),
        invalid_distances: pool.invalid_distances.load(Ordering::Relaxed),
        sample_cap_hits: pool.sample_cap_hits.load(Ordering::Relaxed),
        total_samples_captured: pool.total_samples_captured.load(Ordering::Relaxed),
        present_calls: pool.present_calls.load(Ordering::Relaxed),
        empty_frames: pool.empty_frames.load(Ordering::Relaxed),
        frames_with_samples: pool.frames_with_samples.load(Ordering::Relaxed),
        max_samples_per_frame: pool.max_samples_per_frame.load(Ordering::Relaxed),
        last_capture_thread: pool.last_capture_thread.load(Ordering::Acquire),
        last_present_thread: pool.last_present_thread.load(Ordering::Acquire),
        last_distance: f32::from_bits(pool.last_distance_bits.load(Ordering::Acquire)),
        max_distance: f32::from_bits(pool.max_distance_bits.load(Ordering::Acquire)),
    };

    cached.updated_at = Some(now);
    cached.value = value.clone();
    value
}

#[cfg(test)]
mod tests {
    use super::distance_band;

    #[test]
    fn distance_bands_are_non_overlapping() {
        for distance in [0.0_f32, 99.9, 100.0, 349.9, 350.0, 2_500.0] {
            assert!(distance_band(distance) <= 2);
        }
    }

    #[test]
    fn distance_band_boundaries_are_stable() {
        assert_eq!(distance_band(99.999), 0);
        assert_eq!(distance_band(100.0), 1);
        assert_eq!(distance_band(349.999), 1);
        assert_eq!(distance_band(350.0), 2);
    }

    #[test]
    fn positive_float_bits_preserve_numeric_order() {
        let values = [0.0_f32, 0.1, 1.0, 100.0, 1_000.0];
        for pair in values.windows(2) {
            assert!(pair[0].to_bits() < pair[1].to_bits());
        }
    }
}

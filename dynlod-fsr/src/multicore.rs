//! Experimental multi-core scene preparation with bounded diagnostics.
//!
//! RenderWare and Direct3D calls stay on GTA's render thread. The
//! CRenderer::AddEntityToRenderList hook records only scalar distances;
//! persistent workers classify the visible-entity render-list batch off-thread.
//! Workers never dereference GTA pointers or touch RenderWare/D3D objects.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Instant;

use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows::Win32::System::Threading::{
    GetCurrentProcessorNumber, GetCurrentProcessId, GetCurrentThreadId,
};

const MAX_SAMPLES_PER_FRAME: usize = 16_384;
const DIAGNOSTIC_LOG_INTERVAL_SECONDS: u64 = 5;

thread_local! {
    static FRAME_DISTANCES: RefCell<Vec<f32>> = RefCell::new(Vec::with_capacity(1024));
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
    last_priority_ppm: AtomicU64,
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
            last_priority_ppm: AtomicU64::new(0),
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

    // Hook / capture / frame-boundary diagnostics.
    hook_calls: AtomicU64,
    valid_distances: AtomicU64,
    invalid_distances: AtomicU64,
    sample_cap_hits: AtomicU64,
    total_samples_captured: AtomicU64,
    present_calls: AtomicU64,
    empty_frames: AtomicU64,
    frames_with_samples: AtomicU64,
    max_samples_per_frame: AtomicUsize,
    last_capture_thread: AtomicU32,
    last_present_thread: AtomicU32,
    last_distance_bits: AtomicU32,
    max_distance_bits: AtomicU32,
    first_present_logged: AtomicBool,
}

static POOL: OnceLock<Pool> = OnceLock::new();
static STARTED_AT: OnceLock<Instant> = OnceLock::new();
static LAST_DIAGNOSTIC_LOG_SECOND: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub worker_count: usize,
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

    pub hook_calls: u64,
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

pub fn initialize() {
    if POOL.get().is_some() {
        return;
    }

    let started = Instant::now();
    let _ = STARTED_AT.set(started);

    let mut system_info = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut system_info) };
    let system_processors = system_info.dwNumberOfProcessors.max(1) as usize;
    let available_processors = thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(system_processors);
    let process_id = unsafe { GetCurrentProcessId() };
    let thread_id = unsafe { GetCurrentThreadId() };
    let processor_id = unsafe { GetCurrentProcessorNumber() };

    crate::config::log(&format!(
        "MC DEBUG environment: pid={process_id}, init_tid={thread_id}, init_cpu={processor_id}, system_logical_processors={system_processors}, rust_available_parallelism={available_processors}, active_processor_mask=0x{:X}, page_size={}, allocation_granularity={}, processor_type={}, pointer_bits={}, worker_affinity=not_pinned",
        system_info.dwActiveProcessorMask,
        system_info.dwPageSize,
        system_info.dwAllocationGranularity,
        system_info.dwProcessorType,
        usize::BITS
    ));

    // Leave at least one logical processor available for GTA/SA-MP and the
    // render thread. Keep a conservative cap for this first experiment.
    let requested_workers = available_processors.saturating_sub(1).clamp(1, 7);
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
            worker_stats.last_processor.store(initial_cpu, Ordering::Release);
            if initial_cpu < 64 {
                worker_stats.processors_seen.fetch_or(1_u64 << initial_cpu, Ordering::Relaxed);
            }
            crate::config::log(&format!(
                "MC DEBUG worker_started: index={index}, tid={worker_tid}, initial_cpu={initial_cpu}"
            ));

            while let Ok(job) = receiver.recv() {
                let started = Instant::now();
                let processor = unsafe { GetCurrentProcessorNumber() };
                let mut near = 0_u64;
                let mut mid = 0_u64;
                let mut far = 0_u64;
                let mut priority_sum = 0.0_f64;

                for distance in &job.distances[job.start..job.end] {
                    let d = f64::from(*distance).abs();
                    if d < 100.0 {
                        near += 1;
                    } else if d < 350.0 {
                        mid += 1;
                    } else {
                        far += 1;
                    }

                    // A cheap LOD-importance estimate based on captured distances.
                    let depth = d * d;
                    let projected_weight = 1.0 / (1.0 + depth * 0.0004);
                    let distance_fade = (1.0 - d / 2500.0).clamp(0.0, 1.0);
                    priority_sum += projected_weight * distance_fade;
                }

                let count = (job.end - job.start) as u64;
                let average_priority = if count == 0 {
                    0.0
                } else {
                    priority_sum / count as f64
                };
                let elapsed = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
                let current_tid = unsafe { GetCurrentThreadId() };

                worker_stats.thread_id.store(current_tid, Ordering::Release);
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
                worker_stats.last_priority_ppm.store(
                    (average_priority * 1_000_000.0).round() as u64,
                    Ordering::Release,
                );
            }
        }) {
            Ok(_thread) => workers.push(Worker { sender, stats }),
            Err(error) => {
                crate::config::log(&format!(
                    "MC DEBUG worker_start_failed: index={index}, error={error}"
                ));
                break;
            }
        }
    }

    if workers.is_empty() {
        crate::config::log("MC DEBUG ERROR: scene pipeline unavailable; no worker threads started");
        return;
    }

    let count = workers.len();
    let _ = POOL.set(Pool {
        workers,
        last_frame_entities: AtomicUsize::new(0),
        frames_submitted: AtomicU64::new(0),
        chunks_submitted: AtomicU64::new(0),
        chunks_dropped: AtomicU64::new(0),
        hook_calls: AtomicU64::new(0),
        valid_distances: AtomicU64::new(0),
        invalid_distances: AtomicU64::new(0),
        sample_cap_hits: AtomicU64::new(0),
        total_samples_captured: AtomicU64::new(0),
        present_calls: AtomicU64::new(0),
        empty_frames: AtomicU64::new(0),
        frames_with_samples: AtomicU64::new(0),
        max_samples_per_frame: AtomicUsize::new(0),
        last_capture_thread: AtomicU32::new(0),
        last_present_thread: AtomicU32::new(0),
        last_distance_bits: AtomicU32::new(0),
        max_distance_bits: AtomicU32::new(0),
        first_present_logged: AtomicBool::new(false),
    });
    crate::config::log(&format!(
        "Multicore scene pipeline ready: {count} worker threads; Direct3D remains on the game thread"
    ));
    crate::config::log("MC DEBUG counters initialized; periodic diagnostic snapshot interval=5s");
}

/// Called from the RenderWare render-list hook on the game/render thread.
/// Only a scalar is copied; worker threads never dereference the entity.
pub fn record_render_entity(distance: f32) {
    let Some(pool) = POOL.get() else {
        return;
    };
    pool.hook_calls.fetch_add(1, Ordering::Relaxed);
    pool.last_capture_thread.store(unsafe { GetCurrentThreadId() }, Ordering::Release);

    if !distance.is_finite() || distance < 0.0 {
        pool.invalid_distances.fetch_add(1, Ordering::Relaxed);
        return;
    }
    pool.valid_distances.fetch_add(1, Ordering::Relaxed);
    pool.last_distance_bits.store(distance.to_bits(), Ordering::Release);
    pool.max_distance_bits.fetch_max(distance.to_bits(), Ordering::Relaxed);

    FRAME_DISTANCES.with(|buffer| {
        let mut buffer = buffer.borrow_mut();
        if buffer.len() < MAX_SAMPLES_PER_FRAME {
            buffer.push(distance);
            pool.total_samples_captured.fetch_add(1, Ordering::Relaxed);
        } else {
            pool.sample_cap_hits.fetch_add(1, Ordering::Relaxed);
        }
    });
}

/// Dispatch the current frame's captured entity distances without waiting.
/// If a worker is still busy, its chunk is dropped rather than stalling GTA.
pub fn submit_frame() {
    let Some(pool) = POOL.get() else {
        return;
    };
    let present_number = pool.present_calls.fetch_add(1, Ordering::Relaxed) + 1;
    let present_thread = unsafe { GetCurrentThreadId() };
    pool.last_present_thread.store(present_thread, Ordering::Release);

    if !pool.first_present_logged.swap(true, Ordering::AcqRel) {
        crate::config::log(&format!(
            "MC DEBUG first_present: count={present_number}, tid={present_thread}, cpu={}",
            unsafe { GetCurrentProcessorNumber() }
        ));
    }

    let samples = FRAME_DISTANCES.with(|buffer| buffer.borrow_mut().split_off(0));
    pool.last_frame_entities.store(samples.len(), Ordering::Relaxed);
    pool.max_samples_per_frame.fetch_max(samples.len(), Ordering::Relaxed);
    if samples.is_empty() {
        pool.empty_frames.fetch_add(1, Ordering::Relaxed);
        maybe_log_diagnostics();
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
    maybe_log_diagnostics();
}

fn maybe_log_diagnostics() {
    let elapsed_seconds = STARTED_AT.get_or_init(Instant::now).elapsed().as_secs();
    let previous = LAST_DIAGNOSTIC_LOG_SECOND.load(Ordering::Relaxed);
    if elapsed_seconds < previous.saturating_add(DIAGNOSTIC_LOG_INTERVAL_SECONDS) {
        return;
    }
    if LAST_DIAGNOSTIC_LOG_SECOND
        .compare_exchange(previous, elapsed_seconds, Ordering::AcqRel, Ordering::Relaxed)
        .is_err()
    {
        return;
    }

    let snapshot = status();
    crate::config::log(&format!(
        "MC DEBUG snapshot: present_calls={}, hook_calls={}, valid_distances={}, invalid_distances={}, total_samples={}, last_frame_entities={}, max_frame_entities={}, frames_with_samples={}, empty_frames={}, sample_cap_hits={}, chunks_submitted={}, chunks_dropped={}, processed_entities={}, workers_used={}/{}, worker_jobs=[{}], observed_cpus={}, capture_tid={}, present_tid={}, last_distance={:.2}, max_distance={:.2}, worker_time_ms={}",
        snapshot.present_calls,
        snapshot.hook_calls,
        snapshot.valid_distances,
        snapshot.invalid_distances,
        snapshot.total_samples_captured,
        snapshot.last_frame_entities,
        snapshot.max_samples_per_frame,
        snapshot.frames_with_samples,
        snapshot.empty_frames,
        snapshot.sample_cap_hits,
        snapshot.chunks_submitted,
        snapshot.chunks_dropped,
        snapshot.processed_entities,
        snapshot.active_workers,
        snapshot.worker_count,
        snapshot.worker_jobs,
        snapshot.cpu_cores,
        snapshot.last_capture_thread,
        snapshot.last_present_thread,
        snapshot.last_distance,
        snapshot.max_distance,
        snapshot.worker_time_ms
    ));
    crate::config::log(&format!(
        "MC DEBUG game_state: {}",
        crate::game::diagnostic_snapshot()
    ));
}

pub fn status() -> Status {
    let Some(pool) = POOL.get() else {
        return Status::default();
    };

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
        worker_jobs.push(format!(
            "W{index}:J{jobs}@CPU{}#T{thread_id}",
            if processor == u32::MAX { "PENDING".to_owned() } else { processor.to_string() }
        ));
        processed_entities += worker.stats.entities.load(Ordering::Relaxed);
        worker_time_ns += worker.stats.elapsed_ns.load(Ordering::Relaxed);
        near_entities += worker.stats.near_entities.load(Ordering::Relaxed);
        mid_entities += worker.stats.mid_entities.load(Ordering::Relaxed);
        far_entities += worker.stats.far_entities.load(Ordering::Relaxed);
    }

    let processors = (0..64)
        .filter(|processor| processors_mask & (1_u64 << processor) != 0)
        .map(|processor| processor.to_string())
        .collect::<Vec<_>>();

    Status {
        worker_count: pool.workers.len(),
        active_workers,
        cpu_cores: if processors.is_empty() { "pending".to_owned() } else { processors.join(",") },
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
        hook_calls: pool.hook_calls.load(Ordering::Relaxed),
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
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn distance_bands_are_non_overlapping() {
        for distance in [0.0_f32, 99.9, 100.0, 349.9, 350.0, 2_500.0] {
            let near = distance < 100.0;
            let mid = (100.0..350.0).contains(&distance);
            let far = distance >= 350.0;
            assert_eq!(near as u8 + mid as u8 + far as u8, 1);
        }
    }

    #[test]
    fn positive_float_bits_preserve_numeric_order() {
        let values = [0.0_f32, 0.1, 1.0, 100.0, 1000.0];
        for pair in values.windows(2) {
            assert!(pair[0].to_bits() < pair[1].to_bits());
        }
    }
}

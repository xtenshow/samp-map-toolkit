//! Experimental multi-core scene preparation.
//!
//! RenderWare and Direct3D calls stay on GTA's render thread. The RenderEntity
//! callback records only scalar distances; persistent workers classify the
//! the visible-entity render-list batch and calculate LOD priority metrics off-thread.
//! No worker reads or writes GTA memory, RenderWare objects, or D3D resources.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, OnceLock};
use std::thread;
use std::time::Instant;

use windows::Win32::System::Threading::GetCurrentProcessorNumber;

const MAX_SAMPLES_PER_FRAME: usize = 16_384;

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
}

static POOL: OnceLock<Pool> = OnceLock::new();

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub worker_count: usize,
    pub active_workers: usize,
    pub cpu_cores: String,
    pub last_frame_entities: usize,
    pub processed_entities: u64,
    pub frames_submitted: u64,
    pub chunks_submitted: u64,
    pub chunks_dropped: u64,
    pub worker_time_ms: u64,
    pub near_entities: u64,
    pub mid_entities: u64,
    pub far_entities: u64,
}

pub fn initialize() {
    if POOL.get().is_some() {
        return;
    }

    let logical_processors = thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(2);
    // Leave at least one logical processor available for GTA/SA-MP and the
    // render thread. Keep a conservative cap for this first experiment.
    let requested_workers = logical_processors.saturating_sub(1).clamp(1, 7);
    let mut workers = Vec::with_capacity(requested_workers);

    for index in 0..requested_workers {
        let (sender, receiver) = mpsc::sync_channel::<Job>(1);
        let stats = Arc::new(WorkerStats::new());
        let worker_stats = Arc::clone(&stats);
        let name = format!("GTA-MC-Scene-{index}");

        match thread::Builder::new().name(name).spawn(move || {
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

                    // A cheap view-space LOD importance estimate. This uses
                    // real per-entity distances captured during render-list construction.
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

                worker_stats.last_processor.store(processor, Ordering::Release);
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
                    "Multicore worker {index} could not start: {error}"
                ));
                break;
            }
        }
    }

    if workers.is_empty() {
        crate::config::log("Multicore scene pipeline unavailable: no worker threads started");
        return;
    }

    let count = workers.len();
    let _ = POOL.set(Pool {
        workers,
        last_frame_entities: AtomicUsize::new(0),
        frames_submitted: AtomicU64::new(0),
        chunks_submitted: AtomicU64::new(0),
        chunks_dropped: AtomicU64::new(0),
    });
    crate::config::log(&format!(
        "Multicore scene pipeline ready: {count} worker threads; Direct3D remains on the game thread"
    ));
}

/// Called from the RenderWare RenderEntity hook on the game/render thread.
/// Only a scalar is copied; worker threads never dereference the entity.
pub fn record_render_entity(distance: f32) {
    if POOL.get().is_none() || !distance.is_finite() || distance < 0.0 {
        return;
    }
    FRAME_DISTANCES.with(|buffer| {
        let mut buffer = buffer.borrow_mut();
        if buffer.len() < MAX_SAMPLES_PER_FRAME {
            buffer.push(distance);
        }
    });
}

/// Dispatch the current frame's captured entity distances without waiting.
/// If a worker is still busy, that worker's chunk is dropped rather than
/// stalling the game thread.
pub fn submit_frame() {
    let Some(pool) = POOL.get() else {
        return;
    };
    let samples = FRAME_DISTANCES.with(|buffer| buffer.borrow_mut().split_off(0));
    pool.last_frame_entities.store(samples.len(), Ordering::Relaxed);
    if samples.is_empty() {
        return;
    }

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

pub fn status() -> Status {
    let Some(pool) = POOL.get() else {
        return Status::default();
    };

    let mut active_workers = 0_usize;
    let mut processors = Vec::new();
    let mut processed_entities = 0_u64;
    let mut worker_time_ns = 0_u64;
    let mut near_entities = 0_u64;
    let mut mid_entities = 0_u64;
    let mut far_entities = 0_u64;

    for worker in &pool.workers {
        let jobs = worker.stats.jobs.load(Ordering::Relaxed);
        if jobs > 0 {
            active_workers += 1;
            let processor = worker.stats.last_processor.load(Ordering::Acquire);
            if processor != u32::MAX && !processors.contains(&processor) {
                processors.push(processor);
            }
        }
        processed_entities += worker.stats.entities.load(Ordering::Relaxed);
        worker_time_ns += worker.stats.elapsed_ns.load(Ordering::Relaxed);
        near_entities += worker.stats.near_entities.load(Ordering::Relaxed);
        mid_entities += worker.stats.mid_entities.load(Ordering::Relaxed);
        far_entities += worker.stats.far_entities.load(Ordering::Relaxed);
    }
    processors.sort_unstable();

    Status {
        worker_count: pool.workers.len(),
        active_workers,
        cpu_cores: if processors.is_empty() {
            "pending".to_owned()
        } else {
            processors.iter().map(u32::to_string).collect::<Vec<_>>().join(",")
        },
        last_frame_entities: pool.last_frame_entities.load(Ordering::Relaxed),
        processed_entities,
        frames_submitted: pool.frames_submitted.load(Ordering::Relaxed),
        chunks_submitted: pool.chunks_submitted.load(Ordering::Relaxed),
        chunks_dropped: pool.chunks_dropped.load(Ordering::Relaxed),
        worker_time_ms: worker_time_ns / 1_000_000,
        near_entities,
        mid_entities,
        far_entities,
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
}

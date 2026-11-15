use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

use crate::{config, game, renderer};

thread_local! {
    static BENCHMARK: RefCell<Benchmark> = RefCell::new(Benchmark::new());
}

enum Phase {
    Idle,
    Warmup { started: Instant },
    Measuring { started: Instant, previous: Instant },
}

struct Benchmark {
    phase: Phase,
    key_was_down: bool,
    samples_ns: Vec<u64>,
}

impl Benchmark {
    fn new() -> Self {
        Self {
            phase: Phase::Idle,
            key_was_down: false,
            samples_ns: Vec::new(),
        }
    }

    fn on_present(&mut self) {
        let settings = config::get();
        if !settings.benchmark_enabled {
            return;
        }

        let is_down = unsafe { GetAsyncKeyState(settings.benchmark_key as i32) } < 0;
        let pressed = is_down && !self.key_was_down;
        self.key_was_down = is_down;

        if pressed && !matches!(self.phase, Phase::Idle) {
            self.abort("cancelled by benchmark key");
            return;
        }

        let world_ready = game::world_render_stable() && renderer::world_effect_ready();
        if matches!(self.phase, Phase::Idle) {
            if !pressed {
                return;
            }
            if !world_ready {
                config::log("Benchmark start ignored until FSR and the world are stable");
                return;
            }

            self.samples_ns.clear();
            self.samples_ns
                .reserve(settings.benchmark_duration_seconds.saturating_mul(300) as usize);
            self.phase = Phase::Warmup {
                started: Instant::now(),
            };
            config::log(&format!(
                "Benchmark warmup started: quality={}, scale={:.3}, warmup_seconds={}, duration_seconds={}",
                settings.quality.as_str(),
                settings.effective_render_scale(),
                settings.benchmark_warmup_seconds,
                settings.benchmark_duration_seconds
            ));
            return;
        }

        if !world_ready {
            self.abort("world, transition, or FSR state changed");
            return;
        }

        let now = Instant::now();
        match self.phase {
            Phase::Idle => {}
            Phase::Warmup { started } => {
                if now.duration_since(started)
                    >= Duration::from_secs(settings.benchmark_warmup_seconds as u64)
                {
                    self.phase = Phase::Measuring {
                        started: now,
                        previous: now,
                    };
                    config::log("Benchmark measurement started");
                }
            }
            Phase::Measuring { started, previous } => {
                let interval = now
                    .duration_since(previous)
                    .as_nanos()
                    .min(u64::MAX as u128) as u64;
                if interval > 0 {
                    self.samples_ns.push(interval);
                }
                if now.duration_since(started)
                    >= Duration::from_secs(settings.benchmark_duration_seconds as u64)
                {
                    self.finish();
                } else {
                    self.phase = Phase::Measuring {
                        started,
                        previous: now,
                    };
                }
            }
        }
    }

    fn abort(&mut self, reason: &str) {
        if !matches!(self.phase, Phase::Idle) {
            config::log(&format!("Benchmark aborted: {reason}"));
        }
        self.phase = Phase::Idle;
        self.samples_ns.clear();
    }

    fn finish(&mut self) {
        if self.samples_ns.is_empty() {
            self.abort("no valid Present samples");
            return;
        }

        self.samples_ns.sort_unstable();
        let count = self.samples_ns.len();
        let total_ns: u128 = self.samples_ns.iter().map(|&sample| sample as u128).sum();
        let mean_ns = total_ns as f64 / count as f64;
        let avg_fps = count as f64 * 1_000_000_000.0 / total_ns as f64;
        let p50 = percentile(&self.samples_ns, 0.50);
        let p95 = percentile(&self.samples_ns, 0.95);
        let p99 = percentile(&self.samples_ns, 0.99);
        let slow_count = count.div_ceil(100).max(1);
        let slow_total_ns: u128 = self.samples_ns[count - slow_count..]
            .iter()
            .map(|&sample| sample as u128)
            .sum();
        let one_percent_low_fps = slow_count as f64 * 1_000_000_000.0 / slow_total_ns as f64;
        let settings = config::get();
        config::log(&format!(
            "Benchmark result: quality={}, scale={:.3}, rcas_enabled={}, fxaa_enabled={}, samples={}, duration_seconds={:.3}, avg_fps={:.3}, mean_ms={:.3}, p50_ms={:.3}, p95_ms={:.3}, p99_ms={:.3}, one_percent_low_fps={:.3}",
            settings.quality.as_str(),
            settings.effective_render_scale(),
            settings.rcas_enabled,
            settings.fxaa_enabled,
            count,
            total_ns as f64 / 1_000_000_000.0,
            avg_fps,
            mean_ns / 1_000_000.0,
            p50 as f64 / 1_000_000.0,
            p95 as f64 / 1_000_000.0,
            p99 as f64 / 1_000_000.0,
            one_percent_low_fps
        ));

        self.phase = Phase::Idle;
        self.samples_ns.clear();
    }
}

fn percentile(sorted: &[u64], quantile: f64) -> u64 {
    let rank = (quantile * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

pub fn on_present_success() {
    BENCHMARK.with(|benchmark| benchmark.borrow_mut().on_present());
}

pub fn reset(reason: &str) {
    BENCHMARK.with(|benchmark| benchmark.borrow_mut().abort(reason));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_nearest_rank_percentiles() {
        let samples = [10, 20, 30, 40, 50];
        assert_eq!(percentile(&samples, 0.50), 30);
        assert_eq!(percentile(&samples, 0.95), 50);
        assert_eq!(percentile(&samples, 0.99), 50);
    }
}

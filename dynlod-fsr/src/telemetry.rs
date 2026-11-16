//! Lightweight, process-scoped performance telemetry for the in-game HUD.
//! CPU and RAM are read from Win32 process counters. GPU load uses the
//! Windows GPU Engine PDH counters and is reported as the busiest engine
//! belonging to this process; it is not a whole-adapter percentage.

use std::mem::size_of;
use std::slice;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::FILETIME;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData,
    PdhGetFormattedCounterArrayW, PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W,
    PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetProcessTimes};
use windows::core::w;

const SAMPLE_PERIOD: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Default)]
pub struct Metrics {
    pub cpu_pct: Option<f32>,
    pub ram_mb: Option<u32>,
    pub gpu_pct: Option<f32>,
}

struct PdhState {
    query: usize,
    counter: usize,
    baseline_ready: bool,
}

struct State {
    metrics: Metrics,
    last_sample: Instant,
    last_cpu: Option<(u64, Instant)>,
    pdh: Option<PdhState>,
    pdh_failed: bool,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State {
        metrics: Metrics::default(),
        last_sample: Instant::now() - SAMPLE_PERIOD,
        last_cpu: None,
        pdh: None,
        pdh_failed: false,
    }))
}

pub fn update() {
    let now = Instant::now();
    let mut state = state().lock().unwrap_or_else(|e| e.into_inner());
    if now.saturating_duration_since(state.last_sample) < SAMPLE_PERIOD {
        return;
    }
    state.last_sample = now;

    state.metrics.ram_mb = process_memory_mb();
    state.metrics.cpu_pct = process_cpu_pct(&mut state, now);
    if !state.pdh_failed {
        state.metrics.gpu_pct = process_gpu_pct(&mut state);
    } else {
        state.metrics.gpu_pct = None;
    }
}

pub fn snapshot() -> Metrics {
    state().lock().unwrap_or_else(|e| e.into_inner()).metrics
}

fn filetime_value(value: FILETIME) -> u64 {
    ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64
}

fn process_memory_mb() -> Option<u32> {
    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    counters.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    unsafe { GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) }.ok()?;
    Some((counters.WorkingSetSize / (1024 * 1024)) as u32)
}

fn process_cpu_pct(state: &mut State, now: Instant) -> Option<f32> {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        )
    }.ok()?;

    let total = filetime_value(kernel).saturating_add(filetime_value(user));
    let previous = state.last_cpu.replace((total, now));
    let (previous_total, previous_time) = previous?;
    let elapsed = now.saturating_duration_since(previous_time);
    let wall_100ns = (elapsed.as_nanos() / 100).max(1) as u64;
    let cpu_100ns = total.saturating_sub(previous_total);

    let mut system = SYSTEM_INFO::default();
    unsafe { GetSystemInfo(&mut system) };
    let cores = system.dwNumberOfProcessors.max(1) as f64;
    Some(((cpu_100ns as f64 / wall_100ns as f64 / cores) * 100.0).clamp(0.0, 100.0) as f32)
}

fn ensure_pdh(state: &mut State) -> bool {
    if state.pdh.is_some() {
        return true;
    }

    let mut query = PDH_HQUERY::default();
    let open_status = unsafe { PdhOpenQueryW(None, 0, &mut query) };
    if open_status != 0 {
        state.pdh_failed = true;
        return false;
    }

    let mut counter = PDH_HCOUNTER::default();
    let add_status = unsafe {
        PdhAddEnglishCounterW(
            query,
            w!("\\GPU Engine(*)\\Utilization Percentage"),
            0,
            &mut counter,
        )
    };
    if add_status != 0 {
        unsafe { PdhCloseQuery(query) };
        state.pdh_failed = true;
        return false;
    }

    // The first collection establishes the PDH rate-counter baseline.
    let collect_status = unsafe { PdhCollectQueryData(query) };
    if collect_status != 0 {
        unsafe { PdhCloseQuery(query) };
        state.pdh_failed = true;
        return false;
    }

    state.pdh = Some(PdhState {
        query: query.0 as usize,
        counter: counter.0 as usize,
        baseline_ready: true,
    });
    true
}

fn process_gpu_pct(state: &mut State) -> Option<f32> {
    if !ensure_pdh(state) {
        return None;
    }
    let pdh = state.pdh.as_ref()?;
    let query = PDH_HQUERY(pdh.query as *mut std::ffi::c_void);
    let counter = PDH_HCOUNTER(pdh.counter as *mut std::ffi::c_void);

    if unsafe { PdhCollectQueryData(query) } != 0 {
        return None;
    }

    let mut buffer_size = 0_u32;
    let mut item_count = 0_u32;
    let first_status = unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut buffer_size,
            &mut item_count,
            None,
        )
    };
    if buffer_size == 0 || (first_status == 0 && item_count == 0) {
        return None;
    }

    let words = (buffer_size as usize).div_ceil(size_of::<u64>());
    let mut storage = vec![0_u64; words];
    let second_status = unsafe {
        PdhGetFormattedCounterArrayW(
            counter,
            PDH_FMT_DOUBLE,
            &mut buffer_size,
            &mut item_count,
            Some(storage.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>()),
        )
    };
    if second_status != 0 || item_count == 0 {
        return None;
    }

    let items = unsafe {
        slice::from_raw_parts(
            storage.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(),
            item_count as usize,
        )
    };
    let pid_tag = format!("pid_{}_", unsafe { GetCurrentProcessId() });
    let mut highest: Option<f32> = None;

    for item in items {
        if item.szName.0.is_null() {
            continue;
        }
        let mut length = 0_usize;
        unsafe {
            while length < 512 && *item.szName.0.add(length) != 0 {
                length += 1;
            }
        }
        if length == 0 || length >= 512 {
            continue;
        }
        let name = String::from_utf16_lossy(unsafe {
            slice::from_raw_parts(item.szName.0, length)
        });
        if !name.contains(&pid_tag) {
            continue;
        }

        let value = unsafe { item.FmtValue.Anonymous.doubleValue };
        if item.FmtValue.CStatus <= 1 && value.is_finite() {
            let value = value.clamp(0.0, 100.0) as f32;
            highest = Some(highest.map_or(value, |current| current.max(value)));
        }
    }

    highest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_combines_high_and_low_words() {
        let value = FILETIME {
            dwLowDateTime: 0x89AB_CDEF,
            dwHighDateTime: 0x0123_4567,
        };
        assert_eq!(filetime_value(value), 0x0123_4567_89AB_CDEF);
    }
}

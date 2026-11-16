//! DynamicLOD adaptive controller integrated into the GTA SA FSR ASI.
use std::{
    fs,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use crate::config::{self, QualityPreset};
use crate::{game, renderer, telemetry};

const FAR_CLIP: usize = 0x00B7_C4F0;
const LOD_DISTANCE: usize = 0x0085_8FD8;
const CONTROL_INTERVAL: Duration = Duration::from_millis(250);
const REBUILD_COOLDOWN: Duration = Duration::from_millis(3500);
const SAMPLE_WINDOW: usize = 8;

#[derive(Clone, Copy)]
struct Settings {
    enabled: bool,
    show_overlay: bool,
    key: u16,
    target: f32,
    min_quality: f32,
    max_quality: f32,
    response: f32,
    recovery: f32,
    hysteresis: f32,
    sample_ms: u64,
    min_scale: f32,
    far_clip: f32,
    lod_distance: f32,
}

fn setting<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|line| {
        let line = line.split([';', '#']).next()?.trim();
        let (name, value) = line.split_once('=')?;
        name.trim().eq_ignore_ascii_case(key).then_some(value.trim())
    })
}
fn number(text: &str, key: &str, default: f32, min: f32, max: f32) -> f32 {
    setting(text, key).and_then(|v| v.parse::<f32>().ok())
        .filter(|v| v.is_finite()).map(|v| v.clamp(min, max)).unwrap_or(default)
}
fn boolean(text: &str, key: &str, default: bool) -> bool {
    match setting(text, key).unwrap_or("").to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => true,
        "false" | "no" | "off" | "0" => false,
        _ => default,
    }
}
fn key_code(text: &str) -> u16 {
    let value = setting(text, "dynamic_lod_toggle_key").unwrap_or("F11").to_ascii_uppercase();
    if let Some(n) = value.strip_prefix('F').and_then(|v| v.parse::<u16>().ok()) {
        if (1..=24).contains(&n) { return 0x70 + n - 1; }
    }
    if value.len() == 1 {
        let c = value.as_bytes()[0];
        if c.is_ascii_alphanumeric() { return c as u16; }
    }
    match value.as_str() {
        "INSERT" => 0x2D, "HOME" => 0x24, "END" => 0x23,
        "PAGEUP" => 0x21, "PAGEDOWN" => 0x22, "DELETE" => 0x2E,
        _ => 0x7A,
    }
}
impl Settings {
    fn load() -> Self {
        let path = config::base_directory().join("gta_sa_fsr.ini");
        let text = fs::read_to_string(path).unwrap_or_default();
        let min_q = number(&text, "dynamic_lod_min_quality", 30.0, 10.0, 95.0);
        let mut max_q = number(&text, "dynamic_lod_max_quality", 100.0, 50.0, 100.0);
        let mut min_q = min_q;
        if min_q >= max_q { if min_q >= 95.0 { min_q=95.0; max_q=100.0; } else { max_q=min_q+5.0; } }
        Self {
            enabled: boolean(&text, "dynamic_lod_enabled", false),
            show_overlay: boolean(&text, "dynamic_lod_show_overlay", true),
            key: key_code(&text),
            target: number(&text, "dynamic_lod_target_fps", 60.0, 1.0, 1000.0),
            min_quality: min_q, max_quality: max_q,
            response: number(&text, "dynamic_lod_response_speed", 1.0, 0.1, 5.0),
            recovery: number(&text, "dynamic_lod_recovery_speed", 0.5, 0.1, 3.0),
            hysteresis: number(&text, "dynamic_lod_hysteresis", 5.0, 1.0, 20.0),
            sample_ms: number(&text, "dynamic_lod_sample_interval", 500.0, 100.0, 2000.0) as u64,
            min_scale: number(&text, "dynamic_lod_min_render_scale", 0.40, 0.10, 1.0),
            far_clip: number(&text, "dynamic_lod_far_clip", 350.0, 0.0, 3600.0),
            lod_distance: number(&text, "dynamic_lod_lod_distance", 150.0, 0.0, 1000.0),
        }
    }
}
struct State {
    settings: Settings, enabled: bool, last_key: bool,
    frames: u64, sample_start: Instant, control_start: Instant,
    history: Vec<f32>, avg_history: Vec<f32>, smooth_fps: f32, avg_fps: f32, quality: f32, multiplier: f32,
    base_quality: QualityPreset, base_scale: f32, base_effective: f32,
    applied_scale: Option<f32>, last_rebuild: Instant,
    last_target_reload: Instant,
    original_far_clip: Option<f32>, original_lod_distance: Option<f32>,
    base_far_clip: Option<f32>, base_lod_distance: Option<f32>,
}
static STATE: OnceLock<Mutex<State>> = OnceLock::new();

pub fn initialize() {
    let settings = Settings::load();
    let cfg = config::get();
    let now = Instant::now();
    let _ = STATE.set(Mutex::new(State {
        settings, enabled: settings.enabled, last_key: false, frames: 0,
        sample_start: now, control_start: now, history: Vec::with_capacity(SAMPLE_WINDOW),
        avg_history: Vec::with_capacity(60), smooth_fps: 0.0, avg_fps: 0.0,
        quality: settings.max_quality, multiplier: 1.0,
        base_quality: cfg.quality, base_scale: cfg.render_scale,
        base_effective: cfg.effective_render_scale(), applied_scale: None,
        last_rebuild: now, last_target_reload: now,
        original_far_clip: None, original_lod_distance: None,
        base_far_clip: None, base_lod_distance: None,
    }));
    if game::is_supported_executable() {
        config::log("DynamicLOD controller initialized inside the FSR ASI");
    } else {
        config::log("DynamicLOD inactive: unsupported GTA SA executable");
    }
}

#[derive(Clone, Copy)]
pub struct OverlayStatus {
    pub enabled: bool,
    pub fps: Option<f32>,
    pub avg_fps: Option<f32>,
    pub target: f32,
    pub cpu_pct: Option<f32>,
    pub ram_mb: Option<u32>,
    pub gpu_pct: Option<f32>,
}

#[derive(Clone, Copy)]
pub struct MenuStatus {
    pub enabled: bool,
    pub show_overlay: bool,
    pub target: f32,
    pub min_scale: f32,
    pub hysteresis: f32,
    pub response: f32,
    pub recovery: f32,
    pub sample_ms: u64,
    pub far_clip: f32,
    pub lod_distance: f32,
    pub fps: Option<f32>,
    pub avg_fps: Option<f32>,
}

pub fn overlay_status() -> Option<OverlayStatus> {
    let lock = STATE.get()?;
    let s = lock.lock().unwrap_or_else(|e| e.into_inner());
    if !s.settings.show_overlay { return None; }
    let metrics = telemetry::snapshot();
    Some(OverlayStatus {
        enabled: s.enabled,
        fps: if s.history.is_empty() { None } else { Some(s.smooth_fps) },
        avg_fps: if s.avg_history.is_empty() { None } else { Some(s.avg_fps) },
        target: s.settings.target,
        cpu_pct: metrics.cpu_pct,
        ram_mb: metrics.ram_mb,
        gpu_pct: metrics.gpu_pct,
    })
}

pub fn on_frame() {
    if !game::is_supported_executable() { return; }
    let Some(lock) = STATE.get() else { return };
    let mut s = lock.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    telemetry::update();
    let down = unsafe { GetAsyncKeyState(s.settings.key as i32) } < 0;
    if down && !s.last_key { toggle(&mut s, now); }
    s.last_key = down;

    // Refresh the target FPS from the INI while the game is running.
    // This allows changing dynamic_lod_target_fps without restarting GTA.
    if now.saturating_duration_since(s.last_target_reload) >= Duration::from_secs(1) {
        let refreshed = Settings::load();
        if (refreshed.target - s.settings.target).abs() >= 0.1 {
            s.settings.target = refreshed.target;
            config::log(&format!("DynamicLOD target FPS changed to {:.0}", refreshed.target));
        }
        s.settings.show_overlay = refreshed.show_overlay;
        s.last_target_reload = now;
    }

    if !game::world_render_stable() {
        s.frames = 0; s.sample_start = now; s.history.clear(); s.avg_history.clear();
        s.smooth_fps = 0.0; s.avg_fps = 0.0;
        return;
    }
    s.frames = s.frames.saturating_add(1);
    let elapsed = now.saturating_duration_since(s.sample_start);
    if elapsed >= Duration::from_millis(s.settings.sample_ms) {
        s.smooth_fps = s.frames as f32 / elapsed.as_secs_f32().max(0.001);
        s.frames = 0; s.sample_start = now;
        let sampled_fps = s.smooth_fps;
        s.history.push(sampled_fps);
        if s.history.len() > SAMPLE_WINDOW { s.history.remove(0); }
        s.avg_history.push(sampled_fps);
        if s.avg_history.len() > 60 { s.avg_history.remove(0); }
        s.avg_fps = s.avg_history.iter().sum::<f32>() / s.avg_history.len().max(1) as f32;
        let mut weighted = 0.0; let mut weights = 0.0;
        for (i, fps) in s.history.iter().enumerate() {
            let w = ((i + 1) * (i + 1)) as f32; weighted += *fps * w; weights += w;
        }
        if weights > 0.0 { s.smooth_fps = weighted / weights; }
        update_controller(&mut s, now);
    }
    apply_distances(&s);
}

pub fn menu_status() -> Option<MenuStatus> {
    let lock = STATE.get()?;
    let s = lock.lock().unwrap_or_else(|e| e.into_inner());
    Some(MenuStatus {
        enabled: s.enabled,
        show_overlay: s.settings.show_overlay,
        target: s.settings.target,
        min_scale: s.settings.min_scale,
        hysteresis: s.settings.hysteresis,
        response: s.settings.response,
        recovery: s.settings.recovery,
        sample_ms: s.settings.sample_ms,
        far_clip: s.settings.far_clip,
        lod_distance: s.settings.lod_distance,
        fps: if s.history.is_empty() { None } else { Some(s.smooth_fps) },
        avg_fps: if s.avg_history.is_empty() { None } else { Some(s.avg_fps) },
    })
}

pub fn menu_set_enabled(enabled: bool) {
    if let Some(lock) = STATE.get() {
        let mut s = lock.lock().unwrap_or_else(|e| e.into_inner());
        if s.enabled != enabled {
            toggle(&mut s, Instant::now());
        }
    }
}

pub fn menu_set_overlay(visible: bool) {
    if let Some(lock) = STATE.get() {
        let mut s = lock.lock().unwrap_or_else(|e| e.into_inner());
        s.settings.show_overlay = visible;
        if let Err(error) = config::persist_setting(
            "dynamic_lod_show_overlay",
            if visible { "true" } else { "false" },
        ) {
            config::log(&format!("Could not save overlay visibility: {error}"));
        }
    }
}

pub fn menu_adjust_value(index: usize, direction: i32) {
    if direction == 0 { return; }
    let Some(lock) = STATE.get() else { return; };
    let mut s = lock.lock().unwrap_or_else(|e| e.into_inner());
    let (key, value) = match index {
        0 => {
            s.settings.target = (s.settings.target + direction as f32 * 5.0).clamp(1.0, 1000.0);
            ("dynamic_lod_target_fps", format!("{:.0}", s.settings.target))
        }
        1 => {
            s.settings.min_scale = (s.settings.min_scale + direction as f32 * 0.02).clamp(0.10, 0.95);
            ("dynamic_lod_min_render_scale", format!("{:.2}", s.settings.min_scale))
        }
        2 => {
            s.settings.hysteresis = (s.settings.hysteresis + direction as f32).clamp(1.0, 20.0);
            ("dynamic_lod_hysteresis", format!("{:.0}", s.settings.hysteresis))
        }
        3 => {
            s.settings.response = (s.settings.response + direction as f32 * 0.1).clamp(0.1, 5.0);
            ("dynamic_lod_response_speed", format!("{:.1}", s.settings.response))
        }
        4 => {
            s.settings.recovery = (s.settings.recovery + direction as f32 * 0.1).clamp(0.1, 3.0);
            ("dynamic_lod_recovery_speed", format!("{:.1}", s.settings.recovery))
        }
        5 => {
            s.settings.sample_ms = (s.settings.sample_ms as i32 + direction * 100).clamp(100, 2000) as u64;
            ("dynamic_lod_sample_interval", format!("{}", s.settings.sample_ms))
        }
        6 => {
            s.settings.far_clip = (s.settings.far_clip + direction as f32 * 25.0).clamp(0.0, 3600.0);
            if s.enabled { s.base_far_clip = Some(s.settings.far_clip); apply_distances(&s); }
            ("dynamic_lod_far_clip", format!("{:.0}", s.settings.far_clip))
        }
        7 => {
            s.settings.lod_distance = (s.settings.lod_distance + direction as f32 * 10.0).clamp(0.0, 1000.0);
            if s.enabled { s.base_lod_distance = Some(s.settings.lod_distance); apply_distances(&s); }
            ("dynamic_lod_lod_distance", format!("{:.0}", s.settings.lod_distance))
        }
        _ => return,
    };
    if let Err(error) = config::persist_setting(key, &value) {
        config::log(&format!("Could not save {key}: {error}"));
    }
}

fn toggle(s: &mut State, now: Instant) {
    if s.enabled {
        s.enabled = false; s.quality = s.settings.max_quality; s.multiplier = 1.0;
        s.history.clear(); s.avg_history.clear(); s.frames = 0;
        s.smooth_fps = 0.0; s.avg_fps = 0.0;
        if s.applied_scale.is_some() {
            let _ = config::set_quality(s.base_quality, false);
            let _ = config::set_render_scale(s.base_scale, false);
            renderer::request_quality_rebuild();
        }
        if let Some(value) = s.original_far_clip.take() { write_float(FAR_CLIP, value); }
        if let Some(value) = s.original_lod_distance.take() { write_float(LOD_DISTANCE, value); }
        s.base_far_clip = None; s.base_lod_distance = None;
        config::log("DynamicLOD disabled; FSR and draw-distance baselines restored");
    } else {
        s.original_far_clip = read_float(FAR_CLIP);
        s.original_lod_distance = read_float(LOD_DISTANCE);
        s.base_far_clip = Some(s.settings.far_clip);
        s.base_lod_distance = Some(s.settings.lod_distance);
        let current = config::get();
        s.base_quality = current.quality; s.base_scale = current.render_scale;
        s.base_effective = current.effective_render_scale();
        s.enabled = true; s.quality = s.settings.max_quality; s.multiplier = 1.0;
        s.history.clear(); s.frames = 0; s.smooth_fps = 0.0;
        s.applied_scale = None; s.sample_start = now; s.control_start = now; s.last_rebuild = now;
        config::log(&format!("DynamicLOD enabled; target {:.0} FPS", s.settings.target));
    }
    let value = if s.enabled { "true" } else { "false" };
    if let Err(e) = config::persist_setting("dynamic_lod_enabled", value) {
        config::log(&format!("DynamicLOD settings save failed: {e}"));
    }
}
fn update_controller(s: &mut State, now: Instant) {
    if !s.enabled || s.history.len() < 3 || now.saturating_duration_since(s.control_start) < CONTROL_INTERVAL { return; }
    s.control_start = now;
    let target = s.settings.target;
    let low = target - s.settings.hysteresis;
    let high = target + s.settings.hysteresis * 1.5;
    let mut delta = 0.0;
    if s.smooth_fps < low {
        let deficit = ((low - s.smooth_fps) / target).clamp(0.0, 1.0);
        delta = if deficit > 0.30 { -s.settings.response * (2.0 + deficit * 8.0) }
            else if deficit > 0.10 { -s.settings.response * (1.0 + deficit * 4.0) }
            else { -s.settings.response * (0.5 + deficit * 2.0) };
    } else if s.smooth_fps > high {
        let surplus = ((s.smooth_fps - high) / target).clamp(0.0, 1.0);
        delta = s.settings.recovery * (0.3 + surplus * 1.5);
    }
    s.quality = (s.quality + delta).clamp(s.settings.min_quality, s.settings.max_quality);
    let t = ((s.quality - s.settings.min_quality) / (s.settings.max_quality - s.settings.min_quality).max(1.0)).clamp(0.0, 1.0);
    s.multiplier = 0.25 + 0.75 * t;

    let runtime = config::get();
    if !runtime.enabled { return; }
    if let Some(last) = s.applied_scale {
        if runtime.quality != QualityPreset::Custom || (runtime.render_scale - last).abs() > 0.003 {
            s.base_quality = runtime.quality; s.base_scale = runtime.render_scale;
            s.base_effective = runtime.effective_render_scale(); s.applied_scale = None;
            config::log("Manual FSR quality change detected; DynLOD baseline refreshed");
        }
    }
    let max_scale = s.base_effective.clamp(0.10, 1.0);
    let min_scale = s.settings.min_scale.min(max_scale);
    let desired = min_scale + (max_scale - min_scale) * t;
    if (desired - runtime.effective_render_scale()).abs() >= 0.035
        && now.saturating_duration_since(s.last_rebuild) >= REBUILD_COOLDOWN {
        let new_scale = (desired * 100.0).round() / 100.0;
        if config::set_quality(QualityPreset::Custom, false).is_ok()
            && config::set_render_scale(new_scale, false).is_ok() {
            renderer::request_quality_rebuild(); s.applied_scale = Some(new_scale); s.last_rebuild = now;
            config::log(&format!("DynamicLOD set FSR render scale to {:.0}% (FPS {:.1}/{:.0})", new_scale*100.0, s.smooth_fps, target));
        }
    }
}
fn apply_distances(s: &State) {
    // Never touch GTA's draw-distance values while DynamicLOD is disabled.
    if !s.enabled { return; }
    let m = s.multiplier;
    if let Some(base) = s.base_far_clip {
        write_float(FAR_CLIP, (base * m).clamp(0.0, 3600.0));
    }
    if let Some(base) = s.base_lod_distance {
        write_float(LOD_DISTANCE, (base * m).clamp(0.0, 1000.0));
    }
}
fn read_float(address: usize) -> Option<f32> {
    let value = unsafe { std::ptr::read_volatile(address as *const f32) };
    value.is_finite().then_some(value)
}
fn write_float(address: usize, value: f32) {
    if !value.is_finite() { return; }
    let ptr = address as *mut f32;
    let old = unsafe { std::ptr::read_volatile(ptr) };
    if !old.is_finite() || (old - value).abs() > 0.25 {
        unsafe { std::ptr::write_volatile(ptr, value); }
    }
}

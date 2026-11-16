//! Native SA-MP /dyn command and click/keyboard menu.
use std::{
    ffi::{CStr, CString, c_char, c_void},
    mem::size_of,
    ptr,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::Win32::System::Memory::{MEMORY_BASIC_INFORMATION, MEM_COMMIT, VirtualQuery};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetCursorPos, GetForegroundWindow, ShowCursor,
};
use windows::core::s;

use crate::{config, dynlod, renderer};

type CommandProc = unsafe extern "cdecl" fn(*const c_char);
type AddCommandFn = unsafe extern "thiscall" fn(*mut c_void, *const c_char, CommandProc);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page { Main, Controller, Render, Distance, Hud }

struct MenuState {
    open: bool,
    page: Page,
    selected: usize,
    last_up: bool,
    last_down: bool,
    last_left: bool,
    last_right: bool,
    last_enter: bool,
    last_escape: bool,
    last_mouse: bool,
    cursor_adjustment: i32,
    command_registered: bool,
    last_register_attempt: Instant,
}
impl MenuState {
    fn new() -> Self {
        Self {
            open: false, page: Page::Main, selected: 0,
            last_up: false, last_down: false, last_left: false,
            last_right: false, last_enter: false, last_escape: false,
            last_mouse: false, cursor_adjustment: 0, command_registered: false,
            last_register_attempt: Instant::now() - Duration::from_secs(5),
        }
    }
}

static STATE: OnceLock<Mutex<MenuState>> = OnceLock::new();
fn state() -> &'static Mutex<MenuState> { STATE.get_or_init(|| Mutex::new(MenuState::new())) }

#[derive(Clone)]
pub struct MenuView {
    pub title: String,
    pub description: String,
    pub rows: Vec<String>,
    pub selected: usize,
}

struct SampLayout { input_global: usize, add_command: usize }
const SAMP_LAYOUTS: [SampLayout; 4] = [
    SampLayout { input_global: 0x21A0E8, add_command: 0x65AD0 }, // 0.3.7 R1
    SampLayout { input_global: 0x26E8CC, add_command: 0x69000 }, // 0.3.7 R3-1
    SampLayout { input_global: 0x26EB84, add_command: 0x69770 }, // 0.3.7 R5-1
    SampLayout { input_global: 0x2ACA14, add_command: 0x691B0 }, // 0.3.DL-1
];

pub fn on_frame(output_width: u32, output_height: u32) {
    let mut menu = state().lock().unwrap_or_else(|e| e.into_inner());
    if !menu.command_registered && menu.last_register_attempt.elapsed() >= Duration::from_secs(2) {
        menu.last_register_attempt = Instant::now();
        menu.command_registered = register_command();
        if menu.command_registered {
            config::log("SA-MP /dyn command registered");
        }
    }
    if !menu.open {
        restore_menu_cursor(&mut menu);
        return;
    }
    show_menu_cursor(&mut menu);

    let up = key_down(0x26);
    let down = key_down(0x28);
    let left = key_down(0x25);
    let right = key_down(0x27);
    let enter = key_down(0x0D);
    let escape = key_down(0x1B);
    let mouse = key_down(0x01);

    if edge(escape, &mut menu.last_escape) {
        menu.open = false;
        restore_menu_cursor(&mut menu);
        return;
    }
    if edge(up, &mut menu.last_up) {
        let count = rows_for_page(menu.page).len();
        menu.selected = if menu.selected == 0 { count.saturating_sub(1) } else { menu.selected - 1 };
    }
    if edge(down, &mut menu.last_down) {
        let count = rows_for_page(menu.page).len();
        menu.selected = (menu.selected + 1) % count.max(1);
    }
    if edge(left, &mut menu.last_left) { adjust_selected(&menu, -1); }
    if edge(right, &mut menu.last_right) { adjust_selected(&menu, 1); }

    let mouse_edge = edge(mouse, &mut menu.last_mouse);
    let enter_edge = edge(enter, &mut menu.last_enter);
    if let Some(index) = mouse_row(output_width, output_height, rows_for_page(menu.page).len()) {
        if mouse_edge {
            menu.selected = index;
            activate(&mut menu);
        }
    } else if enter_edge {
        activate(&mut menu);
    }
    if !menu.open { restore_menu_cursor(&mut menu); }
}

pub fn view() -> Option<MenuView> {
    let menu = state().lock().unwrap_or_else(|e| e.into_inner());
    if !menu.open { return None; }
    let status = dynlod::menu_status()?;
    let rows = match menu.page {
        Page::Main => vec![
            "CONTROLLER SETTINGS".to_owned(),
            "FSR AND RENDER SETTINGS".to_owned(),
            "WORLD DISTANCE SETTINGS".to_owned(),
            "HUD AND PERFORMANCE STATS".to_owned(),
            "CLOSE MENU".to_owned(),
        ],
        Page::Controller => vec![
            format!("DYNLOD : {}", on_off(status.enabled)),
            format!("TARGET FPS : {:.0}", status.target),
            format!("MIN RENDER SCALE : {:.0}%", status.min_scale * 100.0),
            format!("FPS HYSTERESIS : {:.0}", status.hysteresis),
            format!("RESPONSE SPEED : {:.1}", status.response),
            format!("RECOVERY SPEED : {:.1}", status.recovery),
            format!("SAMPLE INTERVAL : {} MS", status.sample_ms),
            "BACK".to_owned(),
        ],
        Page::Render => {
            let cfg = config::get();
            vec![
                format!("FSR : {}", on_off(cfg.enabled)),
                format!("QUALITY : {}", cfg.quality.as_str().replace('_', " ").to_ascii_uppercase()),
                format!("RCAS SHARPENING : {}", on_off(cfg.rcas_enabled)),
                format!("FXAA : {}", on_off(cfg.fxaa_enabled)),
                format!("RENDER SCALE : {:.0}%", cfg.effective_render_scale() * 100.0),
                "BACK".to_owned(),
            ]
        }
        Page::Distance => vec![
            format!("FAR CLIP DISTANCE : {:.0}", status.far_clip),
            format!("WORLD LOD DISTANCE : {:.0}", status.lod_distance),
            "BACK".to_owned(),
        ],
        Page::Hud => vec![
            format!("HUD DISPLAY : {}", on_off(status.show_overlay)),
            "BACK".to_owned(),
        ],
    };
    let (title, description) = match menu.page {
        Page::Main => ("DYNLOD CONTROL PANEL", "Yek bakhsh ro entekhab kon ta setting hash baz she."),
        Page::Controller => ("CONTROLLER SETTINGS", "In bakhsh FPS target va sorat-e taghir quality ro modiriyat mikone."),
        Page::Render => ("FSR AND RENDER SETTINGS", "In bakhsh FSR quality, sharpen va render scale ro control mikone."),
        Page::Distance => ("WORLD DISTANCE SETTINGS", "In bakhsh fasele namayesh world va LOD ro taghir mide."),
        Page::Hud => ("HUD AND PERFORMANCE", "In bakhsh HUD, average FPS va etelaate system ro neshon mide."),
    };
    Some(MenuView { title: title.to_owned(), description: description.to_owned(), rows, selected: menu.selected })
}

fn metric(value: Option<f32>) -> String { value.map(|v| format!("{v:.1}")).unwrap_or_else(|| "N A".to_owned()) }
fn on_off(value: bool) -> &'static str { if value { "ON" } else { "OFF" } }

fn rows_for_page(page: Page) -> Vec<&'static str> {
    match page {
        Page::Main => vec!["controller", "render", "distance", "hud", "close"],
        Page::Controller => vec!["toggle", "target", "minscale", "hysteresis", "response", "recovery", "sample", "back"],
        Page::Render => vec!["fsr", "quality", "rcas", "fxaa", "scale", "back"],
        Page::Distance => vec!["far", "lod", "back"],
        Page::Hud => vec!["overlay", "back"],
    }
}

fn activate(menu: &mut MenuState) {
    let status = match dynlod::menu_status() { Some(s) => s, None => return };
    match menu.page {
        Page::Main => {
            menu.page = match menu.selected {
                0 => Page::Controller, 1 => Page::Render, 2 => Page::Distance,
                3 => Page::Hud, _ => { menu.open = false; return; }
            };
            menu.selected = 0;
        }
        Page::Controller => match menu.selected {
            0 => dynlod::menu_set_enabled(!status.enabled),
            1..=6 => dynlod::menu_adjust_value(menu.selected - 1, 1),
            _ => { menu.page = Page::Main; menu.selected = 0; }
        },
        Page::Render => {
            let cfg = config::get();
            match menu.selected {
                0 => {
                    let enabled = !cfg.enabled;
                    let _ = config::set_enabled(enabled, true);
                    renderer::set_effect_enabled_from_menu(enabled);
                    if enabled { renderer::request_quality_rebuild(); }
                }
                1 => {
                    let quality = cfg.quality.cycle(1);
                    let _ = config::set_quality(quality, true);
                    renderer::request_quality_rebuild();
                }
                2 => { let _ = config::set_rcas_enabled(!cfg.rcas_enabled, true); }
                3 => { let _ = config::set_fxaa_enabled(!cfg.fxaa_enabled, true); }
                4 => { let step = if cfg.render_scale < 0.95 { 0.05 } else { -0.05 }; let _ = config::set_quality(config::QualityPreset::Custom, false); let _ = config::set_render_scale((cfg.render_scale + step).clamp(0.1, 1.0), true); renderer::request_quality_rebuild(); }
                _ => { menu.page = Page::Main; menu.selected = 0; }
            }
        }
        Page::Distance => match menu.selected {
            0 => dynlod::menu_adjust_value(6, 1),
            1 => dynlod::menu_adjust_value(7, 1),
            _ => { menu.page = Page::Main; menu.selected = 0; }
        },
        Page::Hud => match menu.selected {
            0 => dynlod::menu_set_overlay(!status.show_overlay),
            _ => { menu.page = Page::Main; menu.selected = 0; }
        }
    }
}

fn adjust_selected(menu: &MenuState, direction: i32) {
    match menu.page {
        Page::Controller => {
            if (1..=6).contains(&menu.selected) {
                dynlod::menu_adjust_value(menu.selected - 1, direction);
            }
        }
        Page::Render => {
            let cfg = config::get();
            match menu.selected {
                1 => {
                    let quality = cfg.quality.cycle(if direction < 0 { -1 } else { 1 });
                    let _ = config::set_quality(quality, true);
                    renderer::request_quality_rebuild();
                }
                4 => {
                    let current = cfg.effective_render_scale();
                    let next = (current + direction as f32 * 0.02).clamp(0.1, 1.0);
                    let _ = config::set_quality(config::QualityPreset::Custom, false);
                    let _ = config::set_render_scale(next, true);
                    renderer::request_quality_rebuild();
                }
                _ => {}
            }
        }
        Page::Distance => {
            if menu.selected == 0 { dynlod::menu_adjust_value(6, direction); }
            if menu.selected == 1 { dynlod::menu_adjust_value(7, direction); }
        }
        _ => {}
    }
}

fn show_menu_cursor(menu: &mut MenuState) {
    if menu.cursor_adjustment != 0 { return; }
    for _ in 0..16 {
        let count = unsafe { ShowCursor(true) };
        menu.cursor_adjustment += 1;
        if count >= 0 { break; }
    }
}

fn restore_menu_cursor(menu: &mut MenuState) {
    while menu.cursor_adjustment > 0 {
        unsafe { ShowCursor(false) };
        menu.cursor_adjustment -= 1;
    }
}

fn edge(value: bool, previous: &mut bool) -> bool {
    let changed = value && !*previous;
    *previous = value;
    changed
}

fn key_down(vk: i32) -> bool { (unsafe { GetAsyncKeyState(vk) }) < 0 }

fn mouse_row(output_width: u32, output_height: u32, row_count: usize) -> Option<usize> {
    let window = unsafe { GetForegroundWindow() };
    if window.0.is_null() { return None; }
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    if unsafe { ScreenToClient(window, &mut point) }.0 == 0 {
        return None;
    }
    let mut client = RECT::default();
    unsafe { GetClientRect(window, &mut client) }.ok()?;
    let cw = (client.right - client.left).max(1) as f32;
    let ch = (client.bottom - client.top).max(1) as f32;
    let x = point.x as f32 * output_width as f32 / cw;
    let y = point.y as f32 * output_height as f32 / ch;
    let scale = (output_width as f32 / 1280.0).clamp(0.75, 1.8);
    let panel_w = 600.0 * scale;
    let panel_h = (112.0 + row_count as f32 * 28.0) * scale;
    let left = (output_width as f32 - panel_w) * 0.5;
    let top = (output_height as f32 - panel_h) * 0.5;
    let row_top = top + 96.0 * scale;
    let row_height = 28.0 * scale;
    if x < left + 20.0 * scale || x > left + panel_w - 20.0 * scale || y < row_top {
        return None;
    }
    let index = ((y - row_top) / row_height).floor() as usize;
    (index < row_count).then_some(index)
}

fn register_command() -> bool {
    let module = match unsafe { GetModuleHandleA(s!("samp.dll")) } {
        Ok(module) => module,
        Err(_) => return false,
    };
    let base = module.0 as usize;
    for layout in SAMP_LAYOUTS {
        let global = (base + layout.input_global) as *const *mut c_void;
        let input = unsafe { ptr::read_unaligned(global) };
        if input.is_null() || !readable_committed(input) { continue; }
        let vtable = unsafe { ptr::read_unaligned(input.cast::<*const usize>()) as usize };
        if vtable < base || vtable > base + 0x800000 { continue; }
        let add_command: AddCommandFn = unsafe { std::mem::transmute(base + layout.add_command) };
        let name = CString::new("dyn").unwrap();
        unsafe { add_command(input, name.as_ptr(), dyn_command) };
        return true;
    }
    false
}

fn readable_committed(address: *mut c_void) -> bool {
    let mut info = MEMORY_BASIC_INFORMATION::default();
    let read = unsafe {
        VirtualQuery(
            Some(address.cast_const()),
            &mut info,
            size_of::<MEMORY_BASIC_INFORMATION>(),
        )
    };
    read != 0 && info.State == MEM_COMMIT
}

unsafe extern "cdecl" fn dyn_command(params: *const c_char) {
    let arguments = if params.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(params) }.to_string_lossy().trim().to_ascii_lowercase()
    };
    let mut menu = state().lock().unwrap_or_else(|e| e.into_inner());
    match arguments.as_str() {
        "" => {
            menu.open = true;
            menu.page = Page::Main;
            menu.selected = 0;
        }
        "on" => dynlod::menu_set_enabled(true),
        "off" => dynlod::menu_set_enabled(false),
        "toggle" => {
            if let Some(status) = dynlod::menu_status() { dynlod::menu_set_enabled(!status.enabled); }
        }
        "hud" | "hud toggle" => {
            if let Some(status) = dynlod::menu_status() { dynlod::menu_set_overlay(!status.show_overlay); }
        }
        "hud on" => dynlod::menu_set_overlay(true),
        "hud off" => dynlod::menu_set_overlay(false),
        _ => {
            menu.open = true;
            menu.page = Page::Main;
            menu.selected = 0;
        }
    }
}

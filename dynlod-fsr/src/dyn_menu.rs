//! Rebuilt native SA-MP /dyn menu with shared mouse/render geometry.
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
enum Page {
    Main,
    Controller,
    Render,
    Distance,
    Hud,
}

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
            open: false,
            page: Page::Main,
            selected: 0,
            last_up: false,
            last_down: false,
            last_left: false,
            last_right: false,
            last_enter: false,
            last_escape: false,
            last_mouse: false,
            cursor_adjustment: 0,
            command_registered: false,
            last_register_attempt: Instant::now() - Duration::from_secs(5),
        }
    }
}

static STATE: OnceLock<Mutex<MenuState>> = OnceLock::new();

fn state() -> &'static Mutex<MenuState> {
    STATE.get_or_init(|| Mutex::new(MenuState::new()))
}

#[derive(Clone)]
pub struct MenuView {
    pub title: String,
    pub description: String,
    pub rows: Vec<String>,
    pub selected: usize,
}

/// The same layout is used by both the D3D renderer and mouse hit-testing.
/// Keeping one source of geometry prevents visible rows and clickable rows
/// from drifting apart at different resolutions or window sizes.
pub struct MenuLayout {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
    pub scale: f32,
    pub row_top: f32,
    pub row_height: f32,
}

impl MenuLayout {
    pub fn new(output_width: u32, output_height: u32, row_count: usize) -> Self {
        let scale = (output_width as f32 / 1280.0).clamp(0.70, 1.60);
        let width = 620.0 * scale;
        let height = (140.0 + row_count as f32 * 34.0) * scale;
        let left = (output_width as f32 - width) * 0.5;
        let top = (output_height as f32 - height) * 0.5;

        Self {
            left,
            top,
            width,
            height,
            scale,
            row_top: top + 100.0 * scale,
            row_height: 34.0 * scale,
        }
    }

    pub fn row_at(&self, x: f32, y: f32, row_count: usize) -> Option<usize> {
        let x_min = self.left + 10.0 * self.scale;
        let x_max = self.left + self.width - 10.0 * self.scale;
        if x < x_min || x > x_max || y < self.row_top {
            return None;
        }

        let index = ((y - self.row_top) / self.row_height).floor() as usize;
        if index < row_count {
            Some(index)
        } else {
            None
        }
    }
}

struct SampLayout {
    input_global: usize,
    add_command: usize,
}

const SAMP_LAYOUTS: [SampLayout; 4] = [
    SampLayout { input_global: 0x21A0E8, add_command: 0x65AD0 }, // 0.3.7 R1
    SampLayout { input_global: 0x26E8CC, add_command: 0x69000 }, // 0.3.7 R3-1
    SampLayout { input_global: 0x26EB84, add_command: 0x69770 }, // 0.3.7 R5-1
    SampLayout { input_global: 0x2ACA14, add_command: 0x691B0 }, // 0.3.DL-1
];

const MAIN_ROWS: &[&str] = &["controller", "render", "distance", "hud", "close"];
const CONTROLLER_ROWS: &[&str] = &[
    "toggle", "target", "minscale", "hysteresis", "response", "recovery", "sample", "back",
];
const RENDER_ROWS: &[&str] = &["fsr", "quality", "rcas", "fxaa", "scale", "back"];
const DISTANCE_ROWS: &[&str] = &["far", "lod", "back"];
const HUD_ROWS: &[&str] = &["overlay", "back"];

pub fn on_frame(output_width: u32, output_height: u32) {
    let mut menu = state().lock().unwrap_or_else(|e| e.into_inner());

    if !menu.command_registered
        && menu.last_register_attempt.elapsed() >= Duration::from_secs(2)
    {
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

    // GTA/SA-MP may hide the cursor again during its own frame processing.
    // Re-check every frame, but undo temporary increments when it was already visible.
    keep_menu_cursor_visible(&mut menu);

    let row_count = rows_for_page(menu.page).len();
    let layout = MenuLayout::new(output_width, output_height, row_count);
    let mouse_row = mouse_position(output_width, output_height)
        .and_then(|(x, y)| layout.row_at(x, y, row_count));

    let up = key_down(0x26);
    let down = key_down(0x28);
    let left = key_down(0x25);
    let right = key_down(0x27);
    let enter = key_down(0x0D);
    let escape = key_down(0x1B);
    let mouse = key_down(0x01);

    if edge(escape, &mut menu.last_escape) {
        close_menu(&mut menu);
        return;
    }

    if edge(up, &mut menu.last_up) {
        let count = row_count.max(1);
        menu.selected = if menu.selected == 0 {
            count - 1
        } else {
            menu.selected - 1
        };
    }

    if edge(down, &mut menu.last_down) {
        menu.selected = (menu.selected + 1) % row_count.max(1);
    }

    if edge(left, &mut menu.last_left) {
        adjust_selected(&menu, -1);
    }
    if edge(right, &mut menu.last_right) {
        adjust_selected(&menu, 1);
    }

    let mouse_pressed = edge(mouse, &mut menu.last_mouse);
    let enter_pressed = edge(enter, &mut menu.last_enter);

    // A click both selects and activates the row under the cursor. Keyboard
    // selection remains stable while the cursor is stationary.
    if mouse_pressed {
        if let Some(index) = mouse_row {
            menu.selected = index;
            activate(&mut menu);
        }
    } else if enter_pressed {
        activate(&mut menu);
    }

    if !menu.open {
        close_menu(&mut menu);
    }
}

pub fn view() -> Option<MenuView> {
    let menu = state().lock().unwrap_or_else(|e| e.into_inner());
    if !menu.open {
        return None;
    }

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
        Page::Main => (
            "DYNLOD CONTROL PANEL",
            "Select a section. Use the mouse or arrow keys and Enter.",
        ),
        Page::Controller => (
            "CONTROLLER SETTINGS",
            "Set the FPS goal and how quickly quality changes.",
        ),
        Page::Render => (
            "FSR AND RENDER SETTINGS",
            "Change FSR quality, sharpening and internal render scale.",
        ),
        Page::Distance => (
            "WORLD DISTANCE SETTINGS",
            "Adjust the far clip and world LOD distances.",
        ),
        Page::Hud => (
            "HUD AND PERFORMANCE",
            "Toggle the live HUD and performance information.",
        ),
    };

    Some(MenuView {
        title: title.to_owned(),
        description: description.to_owned(),
        rows,
        selected: menu.selected,
    })
}

fn on_off(value: bool) -> &'static str {
    if value { "ON" } else { "OFF" }
}

fn rows_for_page(page: Page) -> &'static [&'static str] {
    match page {
        Page::Main => MAIN_ROWS,
        Page::Controller => CONTROLLER_ROWS,
        Page::Render => RENDER_ROWS,
        Page::Distance => DISTANCE_ROWS,
        Page::Hud => HUD_ROWS,
    }
}

fn activate(menu: &mut MenuState) {
    let Some(status) = dynlod::menu_status() else {
        return;
    };

    match menu.page {
        Page::Main => {
            menu.page = match menu.selected {
                0 => Page::Controller,
                1 => Page::Render,
                2 => Page::Distance,
                3 => Page::Hud,
                _ => {
                    menu.open = false;
                    return;
                }
            };
            menu.selected = 0;
        }
        Page::Controller => match menu.selected {
            0 => dynlod::menu_set_enabled(!status.enabled),
            1..=6 => dynlod::menu_adjust_value(menu.selected - 1, 1),
            _ => {
                menu.page = Page::Main;
                menu.selected = 0;
            }
        },
        Page::Render => {
            let cfg = config::get();
            match menu.selected {
                0 => {
                    let enabled = !cfg.enabled;
                    let _ = config::set_enabled(enabled, true);
                    renderer::set_effect_enabled_from_menu(enabled);
                    if enabled {
                        renderer::request_quality_rebuild();
                    }
                }
                1 => {
                    let quality = cfg.quality.cycle(1);
                    let _ = config::set_quality(quality, true);
                    renderer::request_quality_rebuild();
                }
                2 => {
                    let _ = config::set_rcas_enabled(!cfg.rcas_enabled, true);
                }
                3 => {
                    let _ = config::set_fxaa_enabled(!cfg.fxaa_enabled, true);
                }
                4 => {
                    let step = if cfg.effective_render_scale() < 0.95 { 0.05 } else { -0.05 };
                    let _ = config::set_quality(crate::config::QualityPreset::Custom, false);
                    let _ = config::set_render_scale(
                        (cfg.effective_render_scale() + step).clamp(0.10, 1.0),
                        true,
                    );
                    renderer::request_quality_rebuild();
                }
                _ => {
                    menu.page = Page::Main;
                    menu.selected = 0;
                }
            }
        }
        Page::Distance => match menu.selected {
            0 => dynlod::menu_adjust_value(6, 1),
            1 => dynlod::menu_adjust_value(7, 1),
            _ => {
                menu.page = Page::Main;
                menu.selected = 0;
            }
        },
        Page::Hud => match menu.selected {
            0 => dynlod::menu_set_overlay(!status.show_overlay),
            _ => {
                menu.page = Page::Main;
                menu.selected = 0;
            }
        },
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
                    let next = (cfg.effective_render_scale() + direction as f32 * 0.02)
                        .clamp(0.10, 1.0);
                    let _ = config::set_quality(crate::config::QualityPreset::Custom, false);
                    let _ = config::set_render_scale(next, true);
                    renderer::request_quality_rebuild();
                }
                _ => {}
            }
        }
        Page::Distance => {
            if menu.selected == 0 {
                dynlod::menu_adjust_value(6, direction);
            }
            if menu.selected == 1 {
                dynlod::menu_adjust_value(7, direction);
            }
        }
        _ => {}
    }
}

fn keep_menu_cursor_visible(menu: &mut MenuState) {
    for _ in 0..32 {
        let count = unsafe { ShowCursor(true) };
        if count > 0 {
            // We only needed to inspect the count. Undo this temporary increment.
            unsafe { ShowCursor(false) };
            break;
        }

        menu.cursor_adjustment += 1;
        if count >= 0 {
            break;
        }
    }
}

fn restore_menu_cursor(menu: &mut MenuState) {
    while menu.cursor_adjustment > 0 {
        unsafe { ShowCursor(false) };
        menu.cursor_adjustment -= 1;
    }
}

fn close_menu(menu: &mut MenuState) {
    menu.open = false;
    menu.page = Page::Main;
    menu.selected = 0;
    restore_menu_cursor(menu);
}

fn edge(value: bool, previous: &mut bool) -> bool {
    let pressed = value && !*previous;
    *previous = value;
    pressed
}

fn key_down(vk: i32) -> bool {
    (unsafe { GetAsyncKeyState(vk) }) < 0
}

fn mouse_position(output_width: u32, output_height: u32) -> Option<(f32, f32)> {
    let window = unsafe { GetForegroundWindow() };
    if window.0.is_null() {
        return None;
    }

    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    if unsafe { ScreenToClient(window, &mut point) }.0 == 0 {
        return None;
    }

    let mut client = RECT::default();
    unsafe { GetClientRect(window, &mut client) }.ok()?;

    let client_width = (client.right - client.left).max(1) as f32;
    let client_height = (client.bottom - client.top).max(1) as f32;
    let x = point.x as f32 * output_width.max(1) as f32 / client_width;
    let y = point.y as f32 * output_height.max(1) as f32 / client_height;
    Some((x, y))
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
        if input.is_null() || !readable_committed(input) {
            continue;
        }

        let vtable = unsafe { ptr::read_unaligned(input.cast::<*const usize>()) as usize };
        if vtable < base || vtable > base + 0x800000 {
            continue;
        }

        let add_command: AddCommandFn =
            unsafe { std::mem::transmute(base + layout.add_command) };
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
        unsafe { CStr::from_ptr(params) }
            .to_string_lossy()
            .trim()
            .to_ascii_lowercase()
    };

    let mut menu = state().lock().unwrap_or_else(|e| e.into_inner());
    match arguments.as_str() {
        "" | "menu" | "settings" => {
            menu.open = true;
            menu.page = Page::Main;
            menu.selected = 0;
            menu.last_mouse = key_down(0x01);
            menu.last_enter = key_down(0x0D);
            menu.last_escape = key_down(0x1B);
            menu.last_up = key_down(0x26);
            menu.last_down = key_down(0x28);
            menu.last_left = key_down(0x25);
            menu.last_right = key_down(0x27);
        }
        "close" => close_menu(&mut menu),
        "on" => dynlod::menu_set_enabled(true),
        "off" => dynlod::menu_set_enabled(false),
        "toggle" => {
            if let Some(status) = dynlod::menu_status() {
                dynlod::menu_set_enabled(!status.enabled);
            }
        }
        "hud" | "hud toggle" => {
            if let Some(status) = dynlod::menu_status() {
                dynlod::menu_set_overlay(!status.show_overlay);
            }
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

#[cfg(test)]
mod tests {
    use super::MenuLayout;

    #[test]
    fn every_visible_row_is_clickable_at_720p() {
        let layout = MenuLayout::new(1280, 720, 8);
        let x = layout.left + layout.width * 0.5;

        for row in 0..8 {
            let y = layout.row_top + (row as f32 + 0.5) * layout.row_height;
            assert_eq!(layout.row_at(x, y, 8), Some(row));
        }
    }

    #[test]
    fn every_visible_row_is_clickable_at_1080p() {
        let layout = MenuLayout::new(1920, 1080, 8);
        let x = layout.left + layout.width * 0.5;

        for row in 0..8 {
            let y = layout.row_top + (row as f32 + 0.5) * layout.row_height;
            assert_eq!(layout.row_at(x, y, 8), Some(row));
        }
    }

    #[test]
    fn clicks_outside_the_menu_rows_are_ignored() {
        let layout = MenuLayout::new(1280, 720, 5);

        assert_eq!(layout.row_at(layout.left - 1.0, layout.row_top + 5.0, 5), None);
        assert_eq!(layout.row_at(layout.left + layout.width * 0.5, layout.row_top - 1.0, 5), None);
        assert_eq!(layout.row_at(layout.left + layout.width * 0.5, layout.row_top + 5.0 * layout.row_height, 5), None);
    }
}

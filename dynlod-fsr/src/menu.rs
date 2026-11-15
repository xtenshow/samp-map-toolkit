use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use minhook::MinHook;
use windows::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows::Win32::System::Memory::{
    PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS, VirtualProtect,
};
use windows::Win32::System::Threading::GetCurrentProcess;

use crate::{benchmark, config, renderer};

type ProcessPcMenuOptionsFn = unsafe extern "thiscall" fn(*mut c_void, i8, i8) -> i8;
type ProcessUserInputFn = unsafe extern "thiscall" fn(*mut c_void, i8, i8, i8, i8, i8);
type DrawQuitGameScreenFn = unsafe extern "thiscall" fn(*mut c_void);
type SwitchToNewScreenFn = unsafe extern "thiscall" fn(*mut c_void, i8) -> i8;
type DrawStandardMenuFn = unsafe extern "thiscall" fn(*mut c_void, u32);
type TextGetFn = unsafe extern "thiscall" fn(*mut c_void, *const c_char) -> *const c_char;
type StretchFn = unsafe extern "thiscall" fn(*mut c_void, f32) -> f32;
type DrawRectFn = unsafe extern "cdecl" fn(*const GameRect, *const GameColor);

#[repr(C)]
struct GameRect {
    left: f32,
    bottom: f32,
    right: f32,
    top: f32,
}

#[repr(C)]
struct GameColor {
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
}

static ORIGINAL_PROCESS_PC_MENU_OPTIONS: OnceLock<ProcessPcMenuOptionsFn> = OnceLock::new();
static ORIGINAL_PROCESS_USER_INPUT: OnceLock<ProcessUserInputFn> = OnceLock::new();
static ORIGINAL_DRAW_QUIT_GAME_SCREEN: OnceLock<DrawQuitGameScreenFn> = OnceLock::new();
static ORIGINAL_EMPTY_PAGE: OnceLock<[u8; MENU_SCREEN_SIZE]> = OnceLock::new();
static RESOLUTION_LABELS: OnceLock<Vec<CString>> = OnceLock::new();
static FSR_PAGE_ACTIVE: AtomicBool = AtomicBool::new(false);
static FSR_PAGE_DRAW_LOGGED: AtomicBool = AtomicBool::new(false);
static FSR_PAGE_DRAW_COMPLETED_LOGGED: AtomicBool = AtomicBool::new(false);
static FSR_PAGE_TEXT_LOGGED: AtomicBool = AtomicBool::new(false);
static FSR_RESOLUTION_DRAGGING: AtomicBool = AtomicBool::new(false);
static FSR_RESOLUTION_DRAG_DIRTY: AtomicBool = AtomicBool::new(false);

const MENU_SCREENS_ADDRESS: usize = 0x8C_E008;
const ACTIVE_MENU_BASE_OPERAND_ADDRESS: usize = 0x57_9568;
const MENU_SCREEN_SIZE: usize = 0xE2;
const MENU_ENTRIES_OFFSET: usize = 0x0A;
const MENU_ENTRY_SIZE: usize = 0x12;
const ADVANCED_DISPLAY_PAGE: usize = 27;
const EMPTY_PAGE: usize = 43;
const ADVANCED_FSR_LINK_INDEX: usize = 7;
const ADVANCED_BACK_INDEX: usize = 8;
const FSR_TOGGLE_ENTRY_INDEX: usize = 0;
const FSR_QUALITY_ENTRY_INDEX: usize = 1;
const FSR_RCAS_ENTRY_INDEX: usize = 2;
const FSR_FXAA_ENTRY_INDEX: usize = 3;
const FSR_RESOLUTION_ENTRY_INDEX: usize = 4;
const FSR_BACK_ENTRY_INDEX: usize = 5;
const MENU_MANAGER_CURRENT_ENTRY_OFFSET: usize = 0x54;
const MENU_MANAGER_CURRENT_PAGE_OFFSET: usize = 0x15D;
const MENU_MANAGER_MOUSE_X_OFFSET: usize = 0xBC;
const MENU_MANAGER_MOUSE_Y_OFFSET: usize = 0xC0;
const PAD_NEW_ESCAPE_ADDRESS: usize = 0xB7_33A8;
const PAD_OLD_ESCAPE_ADDRESS: usize = 0xB7_3138;
const PAD_LEFT_SHIFT_ADDRESS: usize = 0xB7_33EC;
const PAD_RIGHT_SHIFT_ADDRESS: usize = 0xB7_33EE;
const PAD_SHIFT_ADDRESS: usize = 0xB7_33F0;
const NEW_MOUSE_LEFT_BUTTON_ADDRESS: usize = 0xB7_3418;
const OLD_MOUSE_LEFT_BUTTON_ADDRESS: usize = 0xB7_342C;
const PROCESS_PC_MENU_OPTIONS_ADDRESS: usize = 0x57_CD50;
const PROCESS_USER_INPUT_ADDRESS: usize = 0x57_B480;
const DRAW_QUIT_GAME_SCREEN_ADDRESS: usize = 0x57_D860;
const SWITCH_TO_NEW_SCREEN_ADDRESS: usize = 0x57_3680;
const DRAW_STANDARD_MENU_ADDRESS: usize = 0x57_94A0;
const STRETCH_X_ADDRESS: usize = 0x57_33E0;
const STRETCH_Y_ADDRESS: usize = 0x57_3410;
const DRAW_RECT_ADDRESS: usize = 0x72_7B60;
const TEXT_GET_ADDRESS: usize = 0x6A_0050;
const STANDARD_MENU_TITLE_CALL_ADDRESS: usize = 0x57_9688;
const STANDARD_MENU_TITLE_CALL_BYTES: [u8; 5] = [0xE8, 0xC3, 0x69, 0x12, 0x00];
const STANDARD_MENU_TEXT_CALL_ADDRESS: usize = 0x57_9D73;
const STANDARD_MENU_TEXT_CALL_BYTES: [u8; 5] = [0xE8, 0xD8, 0x62, 0x12, 0x00];
const MENU_ACTION_CHANGE_MENU: u8 = 5;
const MENU_ACTION_BACK: u8 = 2;
const MENU_ACTION_SKIP: u8 = 20;
const FSR_TOGGLE_ACTION: u8 = 70;
const FSR_QUALITY_ACTION: u8 = 71;
const FSR_RCAS_ACTION: u8 = 72;
const FSR_FXAA_ACTION: u8 = 73;
const FSR_RESOLUTION_ACTION: u8 = 74;
const FSR_SETTINGS_KEY: &[u8] = b"FSRSET\0";
const FSR_TOGGLE_KEY: &[u8] = b"FSRONX\0";
const FSR_QUALITY_KEY: &[u8] = b"FSRQAL\0";
const FSR_RCAS_KEY: &[u8] = b"FSRRCAS\0";
const FSR_FXAA_KEY: &[u8] = b"FSRFXAA\0";
const FSR_RESOLUTION_KEY: &[u8] = b"FSRRES\0";
const FSR_SETTINGS: &[u8] = b"FSR Settings\0";
const FSR_ON: &[u8] = b"FSR: ON\0";
const FSR_OFF: &[u8] = b"FSR: OFF\0";
const QUALITY_PERFORMANCE: &[u8] = b"FSR Quality: Performance\0";
const QUALITY_BALANCED: &[u8] = b"FSR Quality: Balanced\0";
const QUALITY_QUALITY: &[u8] = b"FSR Quality: Quality\0";
const QUALITY_ULTRA: &[u8] = b"FSR Quality: Ultra Quality\0";
const QUALITY_CUSTOM: &[u8] = b"FSR Quality: Custom\0";
const RCAS_ON: &[u8] = b"RCAS: ON\0";
const RCAS_OFF: &[u8] = b"RCAS: OFF\0";
const FXAA_ON: &[u8] = b"FXAA: ON\0";
const FXAA_OFF: &[u8] = b"FXAA: OFF\0";
const MIN_RENDER_SCALE_PERCENT: u8 = 10;
const MAX_RENDER_SCALE_PERCENT: u8 = 100;
const RESOLUTION_BAR_LEFT: f32 = 360.0;
const RESOLUTION_BAR_RIGHT: f32 = 700.0;
const RESOLUTION_BAR_TOP: f32 = 253.0;
const RESOLUTION_BAR_BOTTOM: f32 = 262.0;

pub unsafe fn install() -> Result<(), String> {
    let process_options = unsafe {
        MinHook::create_hook(
            PROCESS_PC_MENU_OPTIONS_ADDRESS as *mut c_void,
            process_pc_menu_options_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("ProcessPCMenuOptions detour: {status}"))?;
    ORIGINAL_PROCESS_PC_MENU_OPTIONS
        .set(unsafe { std::mem::transmute::<*mut c_void, ProcessPcMenuOptionsFn>(process_options) })
        .map_err(|_| "ProcessPCMenuOptions trampoline was already initialized".to_owned())?;

    let process_input = unsafe {
        MinHook::create_hook(
            PROCESS_USER_INPUT_ADDRESS as *mut c_void,
            process_user_input_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("ProcessUserInput detour: {status}"))?;
    ORIGINAL_PROCESS_USER_INPUT
        .set(unsafe { std::mem::transmute::<*mut c_void, ProcessUserInputFn>(process_input) })
        .map_err(|_| "ProcessUserInput trampoline was already initialized".to_owned())?;

    let draw_quit = unsafe {
        MinHook::create_hook(
            DRAW_QUIT_GAME_SCREEN_ADDRESS as *mut c_void,
            draw_quit_game_screen_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("DrawQuitGameScreen detour: {status}"))?;
    ORIGINAL_DRAW_QUIT_GAME_SCREEN
        .set(unsafe { std::mem::transmute::<*mut c_void, DrawQuitGameScreenFn>(draw_quit) })
        .map_err(|_| "DrawQuitGameScreen trampoline was already initialized".to_owned())?;

    unsafe {
        patch_text_callsite(
            STANDARD_MENU_TITLE_CALL_ADDRESS,
            STANDARD_MENU_TITLE_CALL_BYTES,
        )
    }?;
    unsafe {
        patch_text_callsite(
            STANDARD_MENU_TEXT_CALL_ADDRESS,
            STANDARD_MENU_TEXT_CALL_BYTES,
        )
    }?;
    unsafe { patch_advanced_display_page() }
}

unsafe fn patch_text_callsite(address: usize, expected: [u8; 5]) -> Result<(), String> {
    let call = address as *mut u8;
    let current = unsafe { std::slice::from_raw_parts(call, expected.len()) };
    if current != expected {
        return Err(format!(
            "menu text callsite 0x{address:08X} is already modified"
        ));
    }

    let next_instruction = address + expected.len();
    let relative = text_get_callsite_shim as *const () as usize as i64 - next_instruction as i64;
    let relative =
        i32::try_from(relative).map_err(|_| "menu text shim is outside rel32 range".to_owned())?;
    let mut patched = [0_u8; 5];
    patched[0] = 0xE8;
    patched[1..].copy_from_slice(&relative.to_le_bytes());

    let mut old_protection = PAGE_PROTECTION_FLAGS::default();
    unsafe {
        VirtualProtect(
            call.cast(),
            patched.len(),
            PAGE_EXECUTE_READWRITE,
            &mut old_protection,
        )
    }
    .map_err(|error| format!("make menu text call writable: {error}"))?;
    unsafe { std::ptr::copy_nonoverlapping(patched.as_ptr(), call, patched.len()) };

    let mut replaced_protection = PAGE_PROTECTION_FLAGS::default();
    unsafe {
        VirtualProtect(
            call.cast(),
            patched.len(),
            old_protection,
            &mut replaced_protection,
        )
    }
    .map_err(|error| format!("restore menu text call protection: {error}"))?;
    unsafe { FlushInstructionCache(GetCurrentProcess(), Some(call.cast()), patched.len()) }
        .map_err(|error| format!("flush menu text call: {error}"))?;
    Ok(())
}

unsafe fn patch_advanced_display_page() -> Result<(), String> {
    let entries = unsafe { page_entries(ADVANCED_DISPLAY_PAGE) };
    let slot = |index: usize| unsafe {
        std::slice::from_raw_parts(entries.add(index * MENU_ENTRY_SIZE), MENU_ENTRY_SIZE)
    };
    let desired = entry_matches(
        slot(ADVANCED_FSR_LINK_INDEX),
        MENU_ACTION_CHANGE_MENU,
        FSR_SETTINGS_KEY,
    ) && is_back_entry(slot(ADVANCED_BACK_INDEX))
        && (9..=11).all(|index| entry_is_empty(slot(index)));
    if desired {
        return Ok(());
    }

    let stock = is_back_entry(slot(7)) && (8..=11).all(|index| entry_is_empty(slot(index)));
    let v0154 = entry_matches(slot(7), 66, b"FSRTGL\0")
        && is_back_entry(slot(8))
        && (9..=11).all(|index| entry_is_empty(slot(index)));
    let v0155 = entry_matches(slot(7), 66, b"FSRTGL\0")
        && entry_matches(slot(8), 67, b"FSRQAL\0")
        && entry_matches(slot(9), 68, b"FSRRCAS\0")
        && entry_matches(slot(10), 69, b"FSRFXAA\0")
        && is_back_entry(slot(11));
    if !stock && !v0154 && !v0155 {
        return Err("Advanced Display custom slots are already occupied".to_owned());
    }

    let back_source = if stock {
        7
    } else if v0154 {
        8
    } else {
        11
    };
    let mut back = [0_u8; MENU_ENTRY_SIZE];
    back.copy_from_slice(slot(back_source));
    let link = menu_entry(
        MENU_ACTION_CHANGE_MENU,
        FSR_SETTINGS_KEY,
        11,
        EMPTY_PAGE as u8,
        0,
        0,
        1,
    );
    unsafe {
        write_entry(entries, ADVANCED_FSR_LINK_INDEX, &link);
        write_entry(entries, ADVANCED_BACK_INDEX, &back);
        for index in 9..=11 {
            write_entry(entries, index, &[0_u8; MENU_ENTRY_SIZE]);
        }
    }
    Ok(())
}

unsafe fn open_fsr_page(menu_manager: *mut c_void) -> Result<(), String> {
    let page = unsafe { page_address(EMPTY_PAGE) };
    let original = ORIGINAL_EMPTY_PAGE.get_or_init(|| {
        let mut bytes = [0_u8; MENU_SCREEN_SIZE];
        unsafe { std::ptr::copy_nonoverlapping(page, bytes.as_mut_ptr(), MENU_SCREEN_SIZE) };
        bytes
    });
    let current = unsafe { std::slice::from_raw_parts(page, MENU_SCREEN_SIZE) };
    if current != original {
        return Err("stock empty menu page changed before FSR page entry".to_owned());
    }

    let custom_page = build_fsr_page(config::get().quality == config::QualityPreset::Custom);
    unsafe { std::ptr::copy_nonoverlapping(custom_page.as_ptr(), page, MENU_SCREEN_SIZE) };
    FSR_PAGE_DRAW_LOGGED.store(false, Ordering::Release);
    FSR_PAGE_DRAW_COMPLETED_LOGGED.store(false, Ordering::Release);
    FSR_PAGE_TEXT_LOGGED.store(false, Ordering::Release);
    FSR_RESOLUTION_DRAGGING.store(false, Ordering::Release);
    FSR_RESOLUTION_DRAG_DIRTY.store(false, Ordering::Release);
    FSR_PAGE_ACTIVE.store(true, Ordering::Release);
    let switch_screen =
        unsafe { std::mem::transmute::<usize, SwitchToNewScreenFn>(SWITCH_TO_NEW_SCREEN_ADDRESS) };
    unsafe { switch_screen(menu_manager, EMPTY_PAGE as i8) };
    unsafe { set_current_entry(menu_manager, FSR_TOGGLE_ENTRY_INDEX as i32) };
    config::log("FSR Settings page opened");
    Ok(())
}

unsafe fn close_fsr_page(menu_manager: *mut c_void) {
    finish_resolution_drag();
    let switch_screen =
        unsafe { std::mem::transmute::<usize, SwitchToNewScreenFn>(SWITCH_TO_NEW_SCREEN_ADDRESS) };
    unsafe { switch_screen(menu_manager, ADVANCED_DISPLAY_PAGE as i8) };
    if let Some(original) = ORIGINAL_EMPTY_PAGE.get() {
        unsafe {
            std::ptr::copy_nonoverlapping(
                original.as_ptr(),
                page_address(EMPTY_PAGE),
                MENU_SCREEN_SIZE,
            )
        };
    }
    FSR_PAGE_ACTIVE.store(false, Ordering::Release);
    FSR_RESOLUTION_DRAGGING.store(false, Ordering::Release);
    FSR_RESOLUTION_DRAG_DIRTY.store(false, Ordering::Release);
    unsafe { set_current_entry(menu_manager, ADVANCED_FSR_LINK_INDEX as i32) };
    config::log("FSR Settings page closed");
}

fn build_fsr_page(custom_resolution: bool) -> [u8; MENU_SCREEN_SIZE] {
    let mut page = [0_u8; MENU_SCREEN_SIZE];
    page[..FSR_SETTINGS_KEY.len()].copy_from_slice(FSR_SETTINGS_KEY);
    page[8] = ADVANCED_DISPLAY_PAGE as u8;
    page[9] = ADVANCED_FSR_LINK_INDEX as u8;
    let entries = unsafe { page.as_mut_ptr().add(MENU_ENTRIES_OFFSET) };
    let rows = [
        (FSR_TOGGLE_ACTION, FSR_TOGGLE_KEY, 127_u16),
        (FSR_QUALITY_ACTION, FSR_QUALITY_KEY, 157),
        (FSR_RCAS_ACTION, FSR_RCAS_KEY, 187),
        (FSR_FXAA_ACTION, FSR_FXAA_KEY, 217),
        (
            if custom_resolution {
                FSR_RESOLUTION_ACTION
            } else {
                MENU_ACTION_SKIP
            },
            FSR_RESOLUTION_KEY,
            247,
        ),
    ];
    for (index, (action, key, y)) in rows.into_iter().enumerate() {
        let entry = menu_entry(action, key, 12, EMPTY_PAGE as u8, 57, y, 1);
        unsafe { write_entry(entries, index, &entry) };
    }
    let back = menu_entry(
        MENU_ACTION_BACK,
        b"FEDS_TB\0",
        11,
        ADVANCED_DISPLAY_PAGE as u8,
        320,
        360,
        3,
    );
    unsafe { write_entry(entries, FSR_BACK_ENTRY_INDEX, &back) };
    page
}

unsafe extern "thiscall" fn process_pc_menu_options_hook(
    menu_manager: *mut c_void,
    input: i8,
    enter: i8,
) -> i8 {
    if !menu_manager.is_null() {
        let page = unsafe { current_page(menu_manager) };
        let entry = unsafe { current_entry(menu_manager) };
        if page == ADVANCED_DISPLAY_PAGE && entry == ADVANCED_FSR_LINK_INDEX as i32 {
            if enter != 0
                && let Err(error) = unsafe { open_fsr_page(menu_manager) }
            {
                config::log(&format!("Could not open FSR Settings page: {error}"));
            }
            return 1;
        }
        if FSR_PAGE_ACTIVE.load(Ordering::Acquire) && page == EMPTY_PAGE {
            return 1;
        }
    }

    let Some(original) = ORIGINAL_PROCESS_PC_MENU_OPTIONS.get().copied() else {
        config::log("ProcessPCMenuOptions hook called without a trampoline");
        return 0;
    };
    unsafe { original(menu_manager, input, enter) }
}

unsafe extern "thiscall" fn process_user_input_hook(
    menu_manager: *mut c_void,
    down: i8,
    up: i8,
    enter: i8,
    exit: i8,
    input: i8,
) {
    if !menu_manager.is_null()
        && FSR_PAGE_ACTIVE.load(Ordering::Acquire)
        && unsafe { current_page(menu_manager) } == EMPTY_PAGE
    {
        if exit != 0 || unsafe { escape_pressed() } {
            unsafe { close_fsr_page(menu_manager) };
            return;
        }
        if unsafe { handle_resolution_mouse(menu_manager) } {
            return;
        }
        let entry = unsafe { current_entry(menu_manager) }.clamp(0, FSR_BACK_ENTRY_INDEX as i32);
        if down != 0 || up != 0 {
            let direction = if down != 0 { 1 } else { -1 };
            let custom = config::get().quality == config::QualityPreset::Custom;
            let next = next_selectable_entry(entry as usize, direction, custom);
            unsafe { set_current_entry(menu_manager, next as i32) };
            return;
        }
        if entry as usize == FSR_BACK_ENTRY_INDEX && enter != 0 {
            unsafe { close_fsr_page(menu_manager) };
            return;
        }
        handle_setting_input(entry as usize, input, enter);
        return;
    }

    let Some(original) = ORIGINAL_PROCESS_USER_INPUT.get().copied() else {
        config::log("ProcessUserInput hook called without a trampoline");
        return;
    };
    unsafe { original(menu_manager, down, up, enter, exit, input) };
}

unsafe fn handle_resolution_mouse(menu_manager: *mut c_void) -> bool {
    let left_button =
        unsafe { std::ptr::read_unaligned(NEW_MOUSE_LEFT_BUTTON_ADDRESS as *const u8) } != 0;
    let old_left_button =
        unsafe { std::ptr::read_unaligned(OLD_MOUSE_LEFT_BUTTON_ADDRESS as *const u8) } != 0;

    if FSR_RESOLUTION_DRAGGING.load(Ordering::Acquire) {
        if left_button {
            unsafe { update_resolution_from_mouse(menu_manager) };
        } else {
            finish_resolution_drag();
        }
        return true;
    }

    if config::get().quality != config::QualityPreset::Custom
        || !left_button
        || old_left_button
        || !unsafe { resolution_bar_hovered(menu_manager) }
    {
        return false;
    }

    FSR_RESOLUTION_DRAGGING.store(true, Ordering::Release);
    FSR_RESOLUTION_DRAG_DIRTY.store(false, Ordering::Release);
    unsafe { set_current_entry(menu_manager, FSR_RESOLUTION_ENTRY_INDEX as i32) };
    unsafe { update_resolution_from_mouse(menu_manager) };
    true
}

unsafe fn resolution_bar_hovered(menu_manager: *mut c_void) -> bool {
    let (left, top, right, bottom) = unsafe { stretched_resolution_bar(menu_manager) };
    let (mouse_x, mouse_y) = unsafe { menu_mouse_position(menu_manager) };
    mouse_x > left && mouse_x < right && mouse_y > top && mouse_y < bottom
}

unsafe fn update_resolution_from_mouse(menu_manager: *mut c_void) {
    let (left, _, right, _) = unsafe { stretched_resolution_bar(menu_manager) };
    let (mouse_x, _) = unsafe { menu_mouse_position(menu_manager) };
    let requested = resolution_percent_from_position(mouse_x, left, right);
    if requested == render_scale_percent() {
        return;
    }

    let _ = config::set_render_scale(f32::from(requested) / 100.0, false);
    FSR_RESOLUTION_DRAG_DIRTY.store(true, Ordering::Release);
}

fn finish_resolution_drag() {
    if !FSR_RESOLUTION_DRAGGING.swap(false, Ordering::AcqRel)
        || !FSR_RESOLUTION_DRAG_DIRTY.swap(false, Ordering::AcqRel)
    {
        return;
    }

    let percent = render_scale_percent();
    let persistence = config::set_render_scale(f32::from(percent) / 100.0, true);
    renderer::request_quality_rebuild();
    setting_changed(&format!("Custom render resolution changed: {percent}%"));
    log_persistence_error(persistence);
}

fn resolution_percent_from_position(mouse_x: f32, left: f32, right: f32) -> u8 {
    let position = ((mouse_x - left) / (right - left)).clamp(0.0, 1.0);
    (f32::from(MIN_RENDER_SCALE_PERCENT)
        + position * f32::from(MAX_RENDER_SCALE_PERCENT - MIN_RENDER_SCALE_PERCENT))
    .round() as u8
}

unsafe extern "thiscall" fn draw_quit_game_screen_hook(menu_manager: *mut c_void) {
    if FSR_PAGE_ACTIVE.load(Ordering::Acquire)
        && !menu_manager.is_null()
        && unsafe { current_page(menu_manager) } == EMPTY_PAGE
    {
        if !FSR_PAGE_DRAW_LOGGED.swap(true, Ordering::AcqRel) {
            config::log("FSR Settings standard draw started");
        }
        let draw_standard =
            unsafe { std::mem::transmute::<usize, DrawStandardMenuFn>(DRAW_STANDARD_MENU_ADDRESS) };
        unsafe { draw_standard(menu_manager, 1) };
        unsafe { draw_resolution_bar(menu_manager) };
        if !FSR_PAGE_DRAW_COMPLETED_LOGGED.swap(true, Ordering::AcqRel) {
            config::log("FSR Settings standard draw completed");
        }
        return;
    }

    let Some(original) = ORIGINAL_DRAW_QUIT_GAME_SCREEN.get().copied() else {
        config::log("DrawQuitGameScreen hook called without a trampoline");
        return;
    };
    unsafe { original(menu_manager) };
}

unsafe fn draw_resolution_bar(menu_manager: *mut c_void) {
    let settings = config::get();
    let percent = effective_render_scale_percent(settings);
    let custom = settings.quality == config::QualityPreset::Custom;
    let fill_right = resolution_bar_fill_right(percent);
    let thumb_center = fill_right.clamp(RESOLUTION_BAR_LEFT + 4.0, RESOLUTION_BAR_RIGHT - 4.0);

    unsafe {
        draw_menu_rect(
            menu_manager,
            RESOLUTION_BAR_LEFT,
            RESOLUTION_BAR_TOP,
            RESOLUTION_BAR_RIGHT,
            RESOLUTION_BAR_BOTTOM,
            GameColor {
                red: 55,
                green: 55,
                blue: 55,
                alpha: 220,
            },
        );
        draw_menu_rect(
            menu_manager,
            RESOLUTION_BAR_LEFT + 2.0,
            RESOLUTION_BAR_TOP + 2.0,
            fill_right,
            RESOLUTION_BAR_BOTTOM - 2.0,
            if custom {
                GameColor {
                    red: 255,
                    green: 180,
                    blue: 0,
                    alpha: 255,
                }
            } else {
                GameColor {
                    red: 105,
                    green: 105,
                    blue: 105,
                    alpha: 150,
                }
            },
        );
        draw_menu_rect(
            menu_manager,
            thumb_center - 2.0,
            RESOLUTION_BAR_TOP - 2.0,
            thumb_center + 2.0,
            RESOLUTION_BAR_BOTTOM + 2.0,
            if custom {
                GameColor {
                    red: 255,
                    green: 255,
                    blue: 255,
                    alpha: 255,
                }
            } else {
                GameColor {
                    red: 150,
                    green: 150,
                    blue: 150,
                    alpha: 150,
                }
            },
        );
    }
}

fn resolution_bar_fill_right(percent: u8) -> f32 {
    let inner_left = RESOLUTION_BAR_LEFT + 2.0;
    let inner_width = RESOLUTION_BAR_RIGHT - RESOLUTION_BAR_LEFT - 4.0;
    let position = f32::from(percent.saturating_sub(MIN_RENDER_SCALE_PERCENT))
        / f32::from(MAX_RENDER_SCALE_PERCENT - MIN_RENDER_SCALE_PERCENT);
    inner_left + inner_width * position.clamp(0.0, 1.0)
}

unsafe fn stretched_resolution_bar(menu_manager: *mut c_void) -> (f32, f32, f32, f32) {
    let stretch_x = unsafe { std::mem::transmute::<usize, StretchFn>(STRETCH_X_ADDRESS) };
    let stretch_y = unsafe { std::mem::transmute::<usize, StretchFn>(STRETCH_Y_ADDRESS) };
    (
        unsafe { stretch_x(menu_manager, RESOLUTION_BAR_LEFT) },
        unsafe { stretch_y(menu_manager, RESOLUTION_BAR_TOP) },
        unsafe { stretch_x(menu_manager, RESOLUTION_BAR_RIGHT) },
        unsafe { stretch_y(menu_manager, RESOLUTION_BAR_BOTTOM) },
    )
}

unsafe fn menu_mouse_position(menu_manager: *mut c_void) -> (f32, f32) {
    let mouse_x = unsafe {
        std::ptr::read_unaligned(
            menu_manager
                .cast::<u8>()
                .add(MENU_MANAGER_MOUSE_X_OFFSET)
                .cast::<i32>(),
        )
    } as f32;
    let mouse_y = unsafe {
        std::ptr::read_unaligned(
            menu_manager
                .cast::<u8>()
                .add(MENU_MANAGER_MOUSE_Y_OFFSET)
                .cast::<i32>(),
        )
    } as f32;
    (mouse_x, mouse_y)
}

unsafe fn draw_menu_rect(
    menu_manager: *mut c_void,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    color: GameColor,
) {
    let stretch_x = unsafe { std::mem::transmute::<usize, StretchFn>(STRETCH_X_ADDRESS) };
    let stretch_y = unsafe { std::mem::transmute::<usize, StretchFn>(STRETCH_Y_ADDRESS) };
    let draw_rect = unsafe { std::mem::transmute::<usize, DrawRectFn>(DRAW_RECT_ADDRESS) };
    let rect = GameRect {
        left: unsafe { stretch_x(menu_manager, left) },
        bottom: unsafe { stretch_y(menu_manager, bottom) },
        right: unsafe { stretch_x(menu_manager, right) },
        top: unsafe { stretch_y(menu_manager, top) },
    };
    unsafe { draw_rect(&rect, &color) };
}

fn next_selectable_entry(current: usize, direction: i32, custom: bool) -> usize {
    let mut next = current;
    for _ in 0..=FSR_BACK_ENTRY_INDEX {
        next = if direction > 0 {
            (next + 1) % (FSR_BACK_ENTRY_INDEX + 1)
        } else if next == 0 {
            FSR_BACK_ENTRY_INDEX
        } else {
            next - 1
        };
        if custom || next != FSR_RESOLUTION_ENTRY_INDEX {
            return next;
        }
    }
    current
}

fn handle_setting_input(entry: usize, input: i8, enter: i8) {
    match entry {
        FSR_TOGGLE_ENTRY_INDEX => {
            let current = renderer::effect_enabled_for_menu();
            if let Some(enabled) = requested_bool(current, input, enter)
                && enabled != current
            {
                let persistence = config::set_enabled(enabled, true);
                renderer::set_effect_enabled_from_menu(enabled);
                setting_changed(if enabled {
                    "FSR enabled"
                } else {
                    "FSR disabled"
                });
                log_persistence_error(persistence);
            }
        }
        FSR_QUALITY_ENTRY_INDEX if input != 0 || enter != 0 => {
            let direction = if input < 0 { -1 } else { 1 };
            let quality = config::get().quality.cycle(direction);
            let persistence = config::set_quality(quality, true);
            update_resolution_action(quality == config::QualityPreset::Custom);
            renderer::request_quality_rebuild();
            setting_changed(&format!("FSR quality changed: {}", quality.as_str()));
            log_persistence_error(persistence);
        }
        FSR_RCAS_ENTRY_INDEX => {
            let current = config::get().rcas_enabled;
            if let Some(enabled) = requested_bool(current, input, enter)
                && enabled != current
            {
                let persistence = config::set_rcas_enabled(enabled, true);
                setting_changed(if enabled {
                    "RCAS enabled"
                } else {
                    "RCAS disabled"
                });
                log_persistence_error(persistence);
            }
        }
        FSR_FXAA_ENTRY_INDEX => {
            let current = config::get().fxaa_enabled;
            if let Some(enabled) = requested_bool(current, input, enter)
                && enabled != current
            {
                let persistence = config::set_fxaa_enabled(enabled, true);
                setting_changed(if enabled {
                    "FXAA enabled"
                } else {
                    "FXAA disabled"
                });
                log_persistence_error(persistence);
            }
        }
        FSR_RESOLUTION_ENTRY_INDEX
            if config::get().quality == config::QualityPreset::Custom
                && (input != 0 || enter != 0) =>
        {
            let current = render_scale_percent();
            let step = resolution_step(shift_held());
            let requested = if input < 0 {
                current.saturating_sub(step)
            } else {
                current.saturating_add(step)
            }
            .clamp(MIN_RENDER_SCALE_PERCENT, MAX_RENDER_SCALE_PERCENT);
            if requested != current {
                let persistence = config::set_render_scale(requested as f32 / 100.0, true);
                renderer::request_quality_rebuild();
                setting_changed(&format!("Custom render resolution changed: {requested}%"));
                log_persistence_error(persistence);
            }
        }
        _ => {}
    }
}

fn requested_bool(current: bool, input: i8, enter: i8) -> Option<bool> {
    if input != 0 || enter != 0 {
        Some(!current)
    } else {
        None
    }
}

fn resolution_step(shift_held: bool) -> u8 {
    if shift_held { 5 } else { 1 }
}

fn shift_held() -> bool {
    unsafe {
        std::ptr::read_unaligned(PAD_LEFT_SHIFT_ADDRESS as *const u16) != 0
            || std::ptr::read_unaligned(PAD_RIGHT_SHIFT_ADDRESS as *const u16) != 0
            || std::ptr::read_unaligned(PAD_SHIFT_ADDRESS as *const u16) != 0
    }
}

fn setting_changed(message: &str) {
    benchmark::reset("FSR Settings page value changed");
    config::log(message);
}

fn log_persistence_error(result: Result<(), String>) {
    if let Err(error) = result {
        config::log(&format!("Could not save FSR Settings value: {error}"));
    }
}

fn update_resolution_action(custom: bool) {
    if !FSR_PAGE_ACTIVE.load(Ordering::Acquire) {
        return;
    }
    let entry =
        unsafe { page_entries(EMPTY_PAGE).add(FSR_RESOLUTION_ENTRY_INDEX * MENU_ENTRY_SIZE) };
    unsafe {
        *entry = if custom {
            FSR_RESOLUTION_ACTION
        } else {
            MENU_ACTION_SKIP
        }
    };
}

unsafe extern "thiscall" fn text_get_callsite_shim(
    text: *mut c_void,
    key: *const c_char,
) -> *const c_char {
    let custom_key = unsafe {
        key_matches(key, FSR_SETTINGS_KEY)
            || key_matches(key, FSR_TOGGLE_KEY)
            || key_matches(key, FSR_QUALITY_KEY)
            || key_matches(key, FSR_RCAS_KEY)
            || key_matches(key, FSR_FXAA_KEY)
            || key_matches(key, FSR_RESOLUTION_KEY)
    };
    if custom_key
        && FSR_PAGE_ACTIVE.load(Ordering::Acquire)
        && !FSR_PAGE_TEXT_LOGGED.swap(true, Ordering::AcqRel)
    {
        config::log("FSR Settings text shim entered");
    }
    let settings = config::get();
    let label = if unsafe { key_matches(key, FSR_SETTINGS_KEY) } {
        Some(FSR_SETTINGS)
    } else if unsafe { key_matches(key, FSR_TOGGLE_KEY) } {
        Some(if renderer::effect_enabled_for_menu() {
            FSR_ON
        } else {
            FSR_OFF
        })
    } else if unsafe { key_matches(key, FSR_QUALITY_KEY) } {
        Some(match settings.quality {
            config::QualityPreset::Performance => QUALITY_PERFORMANCE,
            config::QualityPreset::Balanced => QUALITY_BALANCED,
            config::QualityPreset::Quality => QUALITY_QUALITY,
            config::QualityPreset::UltraQuality => QUALITY_ULTRA,
            config::QualityPreset::Custom => QUALITY_CUSTOM,
        })
    } else if unsafe { key_matches(key, FSR_RCAS_KEY) } {
        Some(if settings.rcas_enabled {
            RCAS_ON
        } else {
            RCAS_OFF
        })
    } else if unsafe { key_matches(key, FSR_FXAA_KEY) } {
        Some(if settings.fxaa_enabled {
            FXAA_ON
        } else {
            FXAA_OFF
        })
    } else {
        None
    };
    if let Some(label) = label {
        return label.as_ptr().cast();
    }
    if unsafe { key_matches(key, FSR_RESOLUTION_KEY) } {
        let percent = effective_render_scale_percent(settings);
        return resolution_labels()[percent.saturating_sub(MIN_RENDER_SCALE_PERCENT) as usize]
            .as_ptr();
    }

    let original = unsafe { std::mem::transmute::<usize, TextGetFn>(TEXT_GET_ADDRESS) };
    unsafe { original(text, key) }
}

fn resolution_labels() -> &'static Vec<CString> {
    RESOLUTION_LABELS.get_or_init(|| {
        (MIN_RENDER_SCALE_PERCENT..=MAX_RENDER_SCALE_PERCENT)
            .map(|percent| CString::new(format!("Resolution: {percent}%")).unwrap())
            .collect()
    })
}

fn render_scale_percent() -> u8 {
    (config::get().render_scale * 100.0).round().clamp(
        MIN_RENDER_SCALE_PERCENT as f32,
        MAX_RENDER_SCALE_PERCENT as f32,
    ) as u8
}

fn effective_render_scale_percent(settings: config::Config) -> u8 {
    (settings.effective_render_scale() * 100.0).round().clamp(
        MIN_RENDER_SCALE_PERCENT as f32,
        MAX_RENDER_SCALE_PERCENT as f32,
    ) as u8
}

fn menu_entry(
    action: u8,
    key: &[u8],
    save_slot: u8,
    target: u8,
    x: u16,
    y: u16,
    alignment: u8,
) -> [u8; MENU_ENTRY_SIZE] {
    let mut entry = [0_u8; MENU_ENTRY_SIZE];
    entry[0] = action;
    entry[1..1 + key.len()].copy_from_slice(key);
    entry[9] = save_slot;
    entry[10] = target;
    entry[12..14].copy_from_slice(&x.to_le_bytes());
    entry[14..16].copy_from_slice(&y.to_le_bytes());
    entry[16] = alignment;
    entry
}

unsafe fn write_entry(entries: *mut u8, index: usize, entry: &[u8; MENU_ENTRY_SIZE]) {
    unsafe {
        std::ptr::copy_nonoverlapping(
            entry.as_ptr(),
            entries.add(index * MENU_ENTRY_SIZE),
            MENU_ENTRY_SIZE,
        )
    };
}

fn entry_matches(entry: &[u8], action: u8, key: &[u8]) -> bool {
    entry[0] == action && entry[1..1 + key.len()] == *key
}

fn is_back_entry(entry: &[u8]) -> bool {
    entry[0] == MENU_ACTION_BACK && entry[1..8] == b"FEDS_TB"[..]
}

fn entry_is_empty(entry: &[u8]) -> bool {
    entry.iter().all(|byte| *byte == 0)
}

unsafe fn active_menu_base() -> *mut u8 {
    let base = unsafe { std::ptr::read_unaligned(ACTIVE_MENU_BASE_OPERAND_ADDRESS as *const u32) }
        as usize;
    if base == 0 {
        MENU_SCREENS_ADDRESS as *mut u8
    } else {
        base as *mut u8
    }
}

unsafe fn page_address(page: usize) -> *mut u8 {
    unsafe { active_menu_base().add(page * MENU_SCREEN_SIZE) }
}

unsafe fn page_entries(page: usize) -> *mut u8 {
    unsafe { page_address(page).add(MENU_ENTRIES_OFFSET) }
}

unsafe fn current_page(menu_manager: *mut c_void) -> usize {
    unsafe {
        *menu_manager
            .cast::<u8>()
            .add(MENU_MANAGER_CURRENT_PAGE_OFFSET) as usize
    }
}

unsafe fn current_entry(menu_manager: *mut c_void) -> i32 {
    unsafe {
        std::ptr::read_unaligned(
            menu_manager
                .cast::<u8>()
                .add(MENU_MANAGER_CURRENT_ENTRY_OFFSET)
                .cast::<i32>(),
        )
    }
}

unsafe fn set_current_entry(menu_manager: *mut c_void, entry: i32) {
    unsafe {
        std::ptr::write_unaligned(
            menu_manager
                .cast::<u8>()
                .add(MENU_MANAGER_CURRENT_ENTRY_OFFSET)
                .cast::<i32>(),
            entry,
        )
    };
}

unsafe fn escape_pressed() -> bool {
    let new_state = unsafe { std::ptr::read_unaligned(PAD_NEW_ESCAPE_ADDRESS as *const u16) };
    let old_state = unsafe { std::ptr::read_unaligned(PAD_OLD_ESCAPE_ADDRESS as *const u16) };
    new_state != 0 && old_state == 0
}

unsafe fn key_matches(key: *const c_char, expected: &[u8]) -> bool {
    if key.is_null() {
        return false;
    }
    expected
        .iter()
        .enumerate()
        .all(|(index, byte)| unsafe { *key.cast::<u8>().add(index) == *byte })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_entries_fit_stock_fields() {
        for key in [
            FSR_SETTINGS_KEY,
            FSR_TOGGLE_KEY,
            FSR_QUALITY_KEY,
            FSR_RCAS_KEY,
            FSR_FXAA_KEY,
            FSR_RESOLUTION_KEY,
        ] {
            assert!(key.len() <= 8);
        }
        let entry = menu_entry(
            FSR_RESOLUTION_ACTION,
            FSR_RESOLUTION_KEY,
            12,
            43,
            57,
            247,
            1,
        );
        assert_eq!(entry[0], FSR_RESOLUTION_ACTION);
        assert_eq!(u16::from_le_bytes([entry[12], entry[13]]), 57);
        assert_eq!(u16::from_le_bytes([entry[14], entry[15]]), 247);
    }

    #[test]
    fn navigation_skips_disabled_resolution() {
        assert_eq!(
            next_selectable_entry(FSR_FXAA_ENTRY_INDEX, 1, false),
            FSR_BACK_ENTRY_INDEX
        );
        assert_eq!(
            next_selectable_entry(FSR_BACK_ENTRY_INDEX, -1, false),
            FSR_FXAA_ENTRY_INDEX
        );
        assert_eq!(
            next_selectable_entry(FSR_FXAA_ENTRY_INDEX, 1, true),
            FSR_RESOLUTION_ENTRY_INDEX
        );
    }

    #[test]
    fn boolean_inputs_set_or_toggle_values() {
        assert_eq!(requested_bool(true, -1, 0), Some(false));
        assert_eq!(requested_bool(false, -1, 0), Some(true));
        assert_eq!(requested_bool(true, 1, 0), Some(false));
        assert_eq!(requested_bool(false, 1, 0), Some(true));
        assert_eq!(requested_bool(true, 0, 1), Some(false));
        assert_eq!(requested_bool(false, 0, 0), None);
    }

    #[test]
    fn shift_changes_resolution_step() {
        assert_eq!(resolution_step(false), 1);
        assert_eq!(resolution_step(true), 5);
    }

    #[test]
    fn resolution_bar_tracks_percentage() {
        assert_eq!(
            resolution_bar_fill_right(MIN_RENDER_SCALE_PERCENT),
            RESOLUTION_BAR_LEFT + 2.0
        );
        assert_eq!(
            resolution_bar_fill_right(MAX_RENDER_SCALE_PERCENT),
            RESOLUTION_BAR_RIGHT - 2.0
        );
        assert!(resolution_bar_fill_right(77) > resolution_bar_fill_right(50));
    }

    #[test]
    fn mouse_position_maps_to_resolution_range() {
        assert_eq!(resolution_percent_from_position(10.0, 10.0, 110.0), 10);
        assert_eq!(resolution_percent_from_position(60.0, 10.0, 110.0), 55);
        assert_eq!(resolution_percent_from_position(110.0, 10.0, 110.0), 100);
        assert_eq!(resolution_percent_from_position(-10.0, 10.0, 110.0), 10);
        assert_eq!(resolution_percent_from_position(130.0, 10.0, 110.0), 100);
    }

    #[test]
    fn resolution_percent_uses_effective_preset_scale() {
        let mut settings = config::Config {
            quality: config::QualityPreset::Balanced,
            render_scale: 0.91,
            ..config::Config::default()
        };
        assert_eq!(effective_render_scale_percent(settings), 59);

        settings.quality = config::QualityPreset::Custom;
        assert_eq!(effective_render_scale_percent(settings), 91);
    }
}

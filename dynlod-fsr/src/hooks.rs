use core::ffi::c_void;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use minhook::MinHook;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D9::{
    D3DCLEAR_TARGET, D3DDEVTYPE, D3DPRESENT_PARAMETERS, D3DRECT, D3DSBT_ALL, IDirect3DDevice9,
};
use windows::Win32::Graphics::Gdi::RGNDATA;
use windows::Win32::System::LibraryLoader::LoadLibraryA;
use windows::core::{Interface, s};

use crate::{benchmark, borderless, config, dynlod, game, menu, overlay, renderer};

type Direct3DCreate9Fn = unsafe extern "system" fn(u32) -> *mut c_void;
type CreateDeviceFn = unsafe extern "system" fn(
    *mut c_void,
    u32,
    D3DDEVTYPE,
    HWND,
    u32,
    *mut D3DPRESENT_PARAMETERS,
    *mut *mut c_void,
) -> i32;
type SceneFn = unsafe extern "system" fn(*mut c_void) -> i32;
type PresentFn =
    unsafe extern "system" fn(*mut c_void, *const RECT, *const RECT, HWND, *const RGNDATA) -> i32;
type ResetFn = unsafe extern "system" fn(*mut c_void, *mut D3DPRESENT_PARAMETERS) -> i32;
type ClearFn =
    unsafe extern "system" fn(*mut c_void, u32, *const D3DRECT, u32, u32, f32, u32) -> i32;
type StartGameFn = unsafe extern "thiscall" fn(*mut c_void) -> i32;
type GameVoidFn = unsafe extern "C" fn();
type RwCameraBeginUpdateFn = unsafe extern "C" fn(*mut c_void) -> *mut c_void;

static ORIGINAL_DIRECT3D_CREATE9: OnceLock<Direct3DCreate9Fn> = OnceLock::new();
static ORIGINAL_START_GAME: OnceLock<StartGameFn> = OnceLock::new();
static ORIGINAL_REINIT_GAME: OnceLock<GameVoidFn> = OnceLock::new();
static ORIGINAL_SHUTDOWN_FOR_RESTART: OnceLock<GameVoidFn> = OnceLock::new();
static ORIGINAL_RENDER_SCENE: OnceLock<GameVoidFn> = OnceLock::new();
static ORIGINAL_POST_EFFECTS_RENDER: OnceLock<GameVoidFn> = OnceLock::new();
static ORIGINAL_RENDER_2D_STUFF: OnceLock<GameVoidFn> = OnceLock::new();
static ORIGINAL_CORONAS_RENDER: OnceLock<GameVoidFn> = OnceLock::new();
static ORIGINAL_BEFORE_MAIN_MIRROR_RENDER: OnceLock<GameVoidFn> = OnceLock::new();
static ORIGINAL_RENDER_MIRROR_BUFFER: OnceLock<GameVoidFn> = OnceLock::new();
static ORIGINAL_RW_CAMERA_BEGIN_UPDATE: OnceLock<RwCameraBeginUpdateFn> = OnceLock::new();
static ORIGINAL_HUD_DRAW: OnceLock<GameVoidFn> = OnceLock::new();
static D3D9_HOOKS: OnceLock<Mutex<HashMap<usize, D3D9Hooks>>> = OnceLock::new();
static CREATE_DEVICE_TARGETS: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();
static DEVICE_HOOKS: OnceLock<Mutex<HashMap<usize, DeviceHooks>>> = OnceLock::new();
static RESET_TARGETS: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();
static PRESENT_TARGETS: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();
static END_SCENE_TARGETS: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();
static CLEAR_TARGETS: OnceLock<Mutex<HashMap<usize, usize>>> = OnceLock::new();
static END_SCENE_OBSERVED: AtomicBool = AtomicBool::new(false);
static SKY_REPLAY_LOGGED: AtomicBool = AtomicBool::new(false);
static POST_EFFECTS_DEFER_LOGGED: AtomicBool = AtomicBool::new(false);
static CORONAS_LOW_DIMENSIONS_LOGGED: AtomicBool = AtomicBool::new(false);
static MIRROR_FRAME_BLOCK_LOGGED: AtomicBool = AtomicBool::new(false);
static ACTIVE_DEVICE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

thread_local! {
    static POST_PROCESS_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static POST_EFFECTS_PENDING: Cell<bool> = const { Cell::new(false) };
    static POST_EFFECTS_REPLAY_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static CORONAS_DIMENSIONS_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static MIRROR_RENDER_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static OUTER_FRAME_BLOCKED: Cell<bool> = const { Cell::new(true) };
    static MIRROR_FRAME_BLOCKED: Cell<bool> = const { Cell::new(false) };
}

const D3D9_CREATE_DEVICE_INDEX: usize = 16;
const DEVICE_RESET_INDEX: usize = 16;
const DEVICE_PRESENT_INDEX: usize = 17;
const DEVICE_END_SCENE_INDEX: usize = 42;
const DEVICE_CLEAR_INDEX: usize = 43;
const START_GAME_ADDRESS: usize = 0x57_3330;
const REINIT_GAME_ADDRESS: usize = 0x53_BCF0;
const SHUTDOWN_FOR_RESTART_ADDRESS: usize = 0x53_C550;
const RENDER_SCENE_ADDRESS: usize = 0x53_DF40;
const POST_EFFECTS_RENDER_ADDRESS: usize = 0x70_46E0;
const RENDER_2D_STUFF_ADDRESS: usize = 0x53_E230;
const CORONAS_RENDER_ADDRESS: usize = 0x6F_AEC0;
const BEFORE_MAIN_MIRROR_RENDER_ADDRESS: usize = 0x72_7140;
const RENDER_MIRROR_BUFFER_ADDRESS: usize = 0x72_6090;
const RW_CAMERA_BEGIN_UPDATE_ADDRESS: usize = 0x7E_E190;
const HUD_DRAW_ADDRESS: usize = 0x58_FAE0;
const RENDER_SKY_POLYS_ADDRESS: usize = 0x71_4650;
const RESET_RENDERWARE_STATE_ADDRESS: usize = 0x73_4650;
const RS_GLOBAL_MAXIMUM_WIDTH_ADDRESS: usize = 0xC1_7044;
const RS_GLOBAL_MAXIMUM_HEIGHT_ADDRESS: usize = 0xC1_7048;
const RW_CAMERA_FRAME_BUFFER_OFFSET: usize = 0x60;
const RW_CAMERA_Z_BUFFER_OFFSET: usize = 0x64;
const RW_RASTER_WIDTH_OFFSET: usize = 0x0C;
const RW_RASTER_HEIGHT_OFFSET: usize = 0x10;
const MIRROR_TYPE_ADDRESS: usize = 0xC7_C724;
const D3DERR_INVALIDCALL: i32 = 0x8876_086C_u32 as i32;

struct D3D9Hooks {
    original_create_device: usize,
}

struct DeviceHooks {
    original_reset: usize,
    original_present: usize,
    original_end_scene: usize,
    original_clear: usize,
}

pub unsafe fn install() -> Result<(), String> {
    if !game::is_supported_executable() {
        return Err(
            "unsupported GTA SA executable (requires 1.0 US Compact or Hoodlum)".to_owned(),
        );
    }

    // Keep the module loaded so the bootstrap is installed before RenderWare
    // resolves and calls Direct3DCreate9. This creates no probe device.
    let _d3d9 = unsafe { LoadLibraryA(s!("d3d9.dll")) }
        .map_err(|error| format!("Load d3d9.dll for bootstrap: {error}"))?;

    let original = unsafe {
        MinHook::create_hook_api(
            "d3d9.dll",
            "Direct3DCreate9",
            direct3d_create9_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("Direct3DCreate9 bootstrap hook: {status}"))?;
    ORIGINAL_DIRECT3D_CREATE9
        .set(unsafe { std::mem::transmute::<*mut c_void, Direct3DCreate9Fn>(original) })
        .map_err(|_| "Direct3DCreate9 trampoline was already initialized".to_owned())?;

    unsafe { install_game_transition_hooks() }?;

    match unsafe { menu::install() } {
        Ok(()) => config::log("Advanced Display FSR menu hook installed"),
        Err(error) => config::log(&format!(
            "Advanced Display FSR menu unavailable; rendering remains active: {error}"
        )),
    }

    unsafe { MinHook::enable_all_hooks() }
        .map_err(|status| format!("Enable plugin hooks: {status}"))?;
    config::log("Direct3DCreate9 bootstrap installed");
    config::log("Game transition hooks installed");
    Ok(())
}

unsafe fn install_game_transition_hooks() -> Result<(), String> {
    let start_game = unsafe {
        MinHook::create_hook(
            START_GAME_ADDRESS as *mut c_void,
            start_game_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("DoSettingsBeforeStartingAGame detour: {status}"))?;
    ORIGINAL_START_GAME
        .set(unsafe { std::mem::transmute::<*mut c_void, StartGameFn>(start_game) })
        .map_err(|_| "Start-game trampoline was already initialized".to_owned())?;

    let reinit_game = unsafe {
        MinHook::create_hook(
            REINIT_GAME_ADDRESS as *mut c_void,
            reinit_game_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("ReInitGameObjectVariables detour: {status}"))?;
    ORIGINAL_REINIT_GAME
        .set(unsafe { std::mem::transmute::<*mut c_void, GameVoidFn>(reinit_game) })
        .map_err(|_| "Reinit-game trampoline was already initialized".to_owned())?;

    let shutdown_for_restart = unsafe {
        MinHook::create_hook(
            SHUTDOWN_FOR_RESTART_ADDRESS as *mut c_void,
            shutdown_for_restart_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("ShutDownForRestart detour: {status}"))?;
    ORIGINAL_SHUTDOWN_FOR_RESTART
        .set(unsafe { std::mem::transmute::<*mut c_void, GameVoidFn>(shutdown_for_restart) })
        .map_err(|_| "Restart-shutdown trampoline was already initialized".to_owned())?;

    let render_scene = unsafe {
        MinHook::create_hook(
            RENDER_SCENE_ADDRESS as *mut c_void,
            render_scene_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("RenderScene detour: {status}"))?;
    ORIGINAL_RENDER_SCENE
        .set(unsafe { std::mem::transmute::<*mut c_void, GameVoidFn>(render_scene) })
        .map_err(|_| "RenderScene trampoline was already initialized".to_owned())?;

    let post_effects_render = unsafe {
        MinHook::create_hook(
            POST_EFFECTS_RENDER_ADDRESS as *mut c_void,
            post_effects_render_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("CPostEffects::Render detour: {status}"))?;
    ORIGINAL_POST_EFFECTS_RENDER
        .set(unsafe { std::mem::transmute::<*mut c_void, GameVoidFn>(post_effects_render) })
        .map_err(|_| "CPostEffects::Render trampoline was already initialized".to_owned())?;

    let render_2d_stuff = unsafe {
        MinHook::create_hook(
            RENDER_2D_STUFF_ADDRESS as *mut c_void,
            render_2d_stuff_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("Render2dStuff detour: {status}"))?;
    ORIGINAL_RENDER_2D_STUFF
        .set(unsafe { std::mem::transmute::<*mut c_void, GameVoidFn>(render_2d_stuff) })
        .map_err(|_| "Render2dStuff trampoline was already initialized".to_owned())?;

    let coronas_render = unsafe {
        MinHook::create_hook(
            CORONAS_RENDER_ADDRESS as *mut c_void,
            coronas_render_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("CCoronas::Render detour: {status}"))?;
    ORIGINAL_CORONAS_RENDER
        .set(unsafe { std::mem::transmute::<*mut c_void, GameVoidFn>(coronas_render) })
        .map_err(|_| "CCoronas::Render trampoline was already initialized".to_owned())?;

    let before_main_mirror_render = unsafe {
        MinHook::create_hook(
            BEFORE_MAIN_MIRROR_RENDER_ADDRESS as *mut c_void,
            before_main_mirror_render_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("CMirrors::BeforeMainRender detour: {status}"))?;
    ORIGINAL_BEFORE_MAIN_MIRROR_RENDER
        .set(unsafe { std::mem::transmute::<*mut c_void, GameVoidFn>(before_main_mirror_render) })
        .map_err(|_| "CMirrors::BeforeMainRender trampoline was already initialized".to_owned())?;

    let render_mirror_buffer = unsafe {
        MinHook::create_hook(
            RENDER_MIRROR_BUFFER_ADDRESS as *mut c_void,
            render_mirror_buffer_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("CMirrors::RenderMirrorBuffer detour: {status}"))?;
    ORIGINAL_RENDER_MIRROR_BUFFER
        .set(unsafe { std::mem::transmute::<*mut c_void, GameVoidFn>(render_mirror_buffer) })
        .map_err(|_| {
            "CMirrors::RenderMirrorBuffer trampoline was already initialized".to_owned()
        })?;

    let camera_begin_update = unsafe {
        MinHook::create_hook(
            RW_CAMERA_BEGIN_UPDATE_ADDRESS as *mut c_void,
            rw_camera_begin_update_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("RwCameraBeginUpdate detour: {status}"))?;
    ORIGINAL_RW_CAMERA_BEGIN_UPDATE
        .set(unsafe {
            std::mem::transmute::<*mut c_void, RwCameraBeginUpdateFn>(camera_begin_update)
        })
        .map_err(|_| "RwCameraBeginUpdate trampoline was already initialized".to_owned())?;

    let hud_draw = unsafe {
        MinHook::create_hook(
            HUD_DRAW_ADDRESS as *mut c_void,
            hud_draw_hook as *mut c_void,
        )
    }
    .map_err(|status| format!("CHud::Draw detour: {status}"))?;
    ORIGINAL_HUD_DRAW
        .set(unsafe { std::mem::transmute::<*mut c_void, GameVoidFn>(hud_draw) })
        .map_err(|_| "CHud::Draw trampoline was already initialized".to_owned())?;

    Ok(())
}

unsafe extern "thiscall" fn start_game_hook(menu: *mut c_void) -> i32 {
    game::mark_start_game_requested();
    config::log("New-game transition requested");

    let Some(original) = ORIGINAL_START_GAME.get().copied() else {
        config::log("Start-game hook called without a trampoline");
        return 0;
    };
    unsafe { original(menu) }
}

unsafe extern "C" fn reinit_game_hook() {
    let Some(original) = ORIGINAL_REINIT_GAME.get().copied() else {
        config::log("Reinit-game hook called without a trampoline");
        return;
    };

    unsafe { original() };
    game::mark_reinit_seen();
    config::log("Game object reinitialization completed");
}

unsafe extern "C" fn shutdown_for_restart_hook() {
    benchmark::reset("game restart");
    renderer::invalidate();
    config::log("Game restart shutdown; FSR resources invalidated");

    let Some(original) = ORIGINAL_SHUTDOWN_FOR_RESTART.get().copied() else {
        config::log("Restart-shutdown hook called without a trampoline");
        return;
    };
    unsafe { original() };
}

unsafe extern "C" fn render_scene_hook() {
    let Some(original) = ORIGINAL_RENDER_SCENE.get().copied() else {
        config::log("RenderScene hook called without a trampoline");
        return;
    };

    let mut world_redirected = false;
    let world_result = match config::get().pipeline_stage {
        config::PipelineStage::WorldProbe => {
            let device = ACTIVE_DEVICE.load(Ordering::Acquire) as *mut c_void;
            if !device.is_null()
                && let Some(device_reference) =
                    unsafe { IDirect3DDevice9::from_raw_borrowed(&device) }
            {
                renderer::probe_world_render(device_reference)
            } else {
                Ok(())
            }
        }
        config::PipelineStage::WorldResources => {
            let device = ACTIVE_DEVICE.load(Ordering::Acquire) as *mut c_void;
            if !device.is_null()
                && let Some(device_reference) =
                    unsafe { IDirect3DDevice9::from_raw_borrowed(&device) }
            {
                renderer::prepare_world_resources(device_reference)
            } else {
                Ok(())
            }
        }
        config::PipelineStage::WorldBind
        | config::PipelineStage::WorldEasu
        | config::PipelineStage::WorldHudEasu => {
            let device = ACTIVE_DEVICE.load(Ordering::Acquire) as *mut c_void;
            if !device.is_null()
                && let Some(device_reference) =
                    unsafe { IDirect3DDevice9::from_raw_borrowed(&device) }
            {
                renderer::begin_world_render_for_frame(device_reference, outer_frame_blocked()).map(
                    |redirected| {
                        world_redirected = redirected;
                    },
                )
            } else {
                Ok(())
            }
        }
        _ => Ok(()),
    };
    if let Err(error) = world_result {
        config::log(&format!("RenderScene stage failed: {error}"));
    }

    if renderer::world_frame_active()
        && !outer_frame_blocked()
        && !game::auxiliary_render_scene_active()
        && !mirror_render_active()
    {
        let device = ACTIVE_DEVICE.load(Ordering::Acquire) as *mut c_void;
        if !device.is_null()
            && let Some(device_reference) = unsafe { IDirect3DDevice9::from_raw_borrowed(&device) }
            && let Err(error) = renderer::rebind_world_after_camera_update(device_reference)
        {
            config::log(&format!(
                "RenderScene-entry low-target rebind failed: {error}"
            ));
        }
        world_redirected = renderer::world_frame_active();
    }

    if world_redirected {
        let device = ACTIVE_DEVICE.load(Ordering::Acquire) as *mut c_void;
        if !device.is_null()
            && let Some(device_reference) = unsafe { IDirect3DDevice9::from_raw_borrowed(&device) }
            && let Err(error) = unsafe { replay_sky_gradient(device_reference) }
        {
            config::log(&format!("Sky-gradient replay failed: {error}"));
        }
    }

    unsafe { original() };

    if world_redirected && config::get().pipeline_stage != config::PipelineStage::WorldHudEasu {
        let device = ACTIVE_DEVICE.load(Ordering::Acquire) as *mut c_void;
        if !device.is_null()
            && let Some(device_reference) = unsafe { IDirect3DDevice9::from_raw_borrowed(&device) }
            && let Err(error) = renderer::end_world_render(device_reference)
        {
            config::log(&format!("Restore native world target failed: {error}"));
        }
    }
}

unsafe fn replay_sky_gradient(device: &IDirect3DDevice9) -> Result<(), String> {
    let state = unsafe { device.CreateStateBlock(D3DSBT_ALL) }
        .map_err(|error| format!("Capture pre-sky D3D9 state: {error}"))?;
    let render_sky = unsafe { std::mem::transmute::<usize, GameVoidFn>(RENDER_SKY_POLYS_ADDRESS) };
    unsafe { render_sky() };
    unsafe { state.Apply() }.map_err(|error| format!("Restore post-sky D3D9 state: {error}"))?;

    let reset_renderware_state =
        unsafe { std::mem::transmute::<usize, GameVoidFn>(RESET_RENDERWARE_STATE_ADDRESS) };
    unsafe { reset_renderware_state() };
    renderer::rebind_world_after_camera_update(device)?;

    if !SKY_REPLAY_LOGGED.swap(true, Ordering::AcqRel) {
        config::log("Native sky-gradient replayed inside low-resolution world frame");
    }
    Ok(())
}

unsafe extern "C" fn hud_draw_hook() {
    finish_deferred_post_effects("CHud::Draw fallback");
    let Some(original) = ORIGINAL_HUD_DRAW.get().copied() else {
        config::log("CHud::Draw hook called without a trampoline");
        return;
    };
    unsafe { original() };
}

unsafe extern "C" fn post_effects_render_hook() {
    let Some(original) = ORIGINAL_POST_EFFECTS_RENDER.get().copied() else {
        config::log("CPostEffects::Render hook called without a trampoline");
        return;
    };

    let should_defer = renderer::world_frame_active()
        && !game::auxiliary_render_scene_active()
        && !mirror_render_active()
        && !POST_EFFECTS_REPLAY_ACTIVE.with(Cell::get);
    if should_defer {
        POST_EFFECTS_PENDING.with(|pending| pending.set(true));
        if !POST_EFFECTS_DEFER_LOGGED.swap(true, Ordering::AcqRel) {
            config::log("Native post-effects deferred until low-resolution late effects finish");
        }
        return;
    }

    unsafe { original() };
}

unsafe extern "C" fn coronas_render_hook() {
    let Some(original) = ORIGINAL_CORONAS_RENDER.get().copied() else {
        config::log("CCoronas::Render hook called without a trampoline");
        return;
    };
    let Some((width, height)) = renderer::world_render_dimensions() else {
        unsafe { original() };
        return;
    };
    if game::auxiliary_render_scene_active()
        || mirror_render_active()
        || CORONAS_DIMENSIONS_ACTIVE.with(|active| active.replace(true))
    {
        unsafe { original() };
        return;
    }

    let camera = game::main_camera() as *mut u8;
    if camera.is_null() {
        CORONAS_DIMENSIONS_ACTIVE.with(|active| active.set(false));
        unsafe { original() };
        return;
    }
    let frame_buffer = unsafe { *camera.add(RW_CAMERA_FRAME_BUFFER_OFFSET).cast::<*mut u8>() };
    let z_buffer = unsafe { *camera.add(RW_CAMERA_Z_BUFFER_OFFSET).cast::<*mut u8>() };
    if frame_buffer.is_null() || z_buffer.is_null() {
        CORONAS_DIMENSIONS_ACTIVE.with(|active| active.set(false));
        unsafe { original() };
        return;
    }

    let frame_width = unsafe { frame_buffer.add(RW_RASTER_WIDTH_OFFSET).cast::<i32>() };
    let frame_height = unsafe { frame_buffer.add(RW_RASTER_HEIGHT_OFFSET).cast::<i32>() };
    let z_width = unsafe { z_buffer.add(RW_RASTER_WIDTH_OFFSET).cast::<i32>() };
    let z_height = unsafe { z_buffer.add(RW_RASTER_HEIGHT_OFFSET).cast::<i32>() };
    let global_width = RS_GLOBAL_MAXIMUM_WIDTH_ADDRESS as *mut i32;
    let global_height = RS_GLOBAL_MAXIMUM_HEIGHT_ADDRESS as *mut i32;
    let saved = unsafe {
        (
            *frame_width,
            *frame_height,
            *z_width,
            *z_height,
            *global_width,
            *global_height,
        )
    };

    unsafe {
        *frame_width = width as i32;
        *frame_height = height as i32;
        *z_width = width as i32;
        *z_height = height as i32;
        *global_width = width as i32;
        *global_height = height as i32;
        original();
        *frame_width = saved.0;
        *frame_height = saved.1;
        *z_width = saved.2;
        *z_height = saved.3;
        *global_width = saved.4;
        *global_height = saved.5;
    }
    CORONAS_DIMENSIONS_ACTIVE.with(|active| active.set(false));

    if !CORONAS_LOW_DIMENSIONS_LOGGED.swap(true, Ordering::AcqRel) {
        config::log(&format!(
            "CCoronas::Render dimensions scaled to {width}x{height} for low-resolution frame"
        ));
    }
}

unsafe extern "C" fn before_main_mirror_render_hook() {
    let Some(original) = ORIGINAL_BEFORE_MAIN_MIRROR_RENDER.get().copied() else {
        config::log("CMirrors::BeforeMainRender hook called without a trampoline");
        return;
    };

    let world_stable = game::world_render_stable();
    renderer::sample_outer_world_state(world_stable);
    OUTER_FRAME_BLOCKED.with(|blocked| blocked.set(!world_stable));
    MIRROR_FRAME_BLOCKED.with(|blocked| blocked.set(false));

    let mirror_type_before = unsafe { read_mirror_type() };
    unsafe { original() };
    let mirror_type_after = unsafe { read_mirror_type() };
    if mirror_type_before != 0 || mirror_type_after != 0 {
        MIRROR_FRAME_BLOCKED.with(|blocked| blocked.set(true));
        OUTER_FRAME_BLOCKED.with(|blocked| blocked.set(true));
        if !MIRROR_FRAME_BLOCK_LOGGED.swap(true, Ordering::AcqRel) {
            config::log(&format!(
                "World FSR blocked for mirror frame: type_before={mirror_type_before}, type_after={mirror_type_after}"
            ));
        }
    }
}

unsafe extern "C" fn render_mirror_buffer_hook() {
    let Some(original) = ORIGINAL_RENDER_MIRROR_BUFFER.get().copied() else {
        config::log("CMirrors::RenderMirrorBuffer hook called without a trampoline");
        return;
    };
    let previous = MIRROR_RENDER_ACTIVE.with(|active| active.replace(true));
    unsafe { original() };
    MIRROR_RENDER_ACTIVE.with(|active| active.set(previous));
}

unsafe fn read_mirror_type() -> u32 {
    unsafe { std::ptr::read_volatile(MIRROR_TYPE_ADDRESS as *const u32) }
}

fn mirror_render_active() -> bool {
    MIRROR_RENDER_ACTIVE.with(Cell::get)
}

fn outer_frame_blocked() -> bool {
    OUTER_FRAME_BLOCKED.with(Cell::get)
}

fn clear_outer_frame_gate() {
    OUTER_FRAME_BLOCKED.with(|blocked| blocked.set(true));
    MIRROR_FRAME_BLOCKED.with(|blocked| blocked.set(false));
}

unsafe extern "C" fn render_2d_stuff_hook() {
    finish_deferred_post_effects("Render2dStuff");
    let Some(original) = ORIGINAL_RENDER_2D_STUFF.get().copied() else {
        config::log("Render2dStuff hook called without a trampoline");
        return;
    };
    unsafe { original() };
    clear_outer_frame_gate();
}

fn finish_deferred_post_effects(context: &str) {
    let post_effects_pending = POST_EFFECTS_PENDING.with(|pending| pending.replace(false));
    finish_pending_world_frame(context);
    if !post_effects_pending {
        return;
    }

    let Some(original) = ORIGINAL_POST_EFFECTS_RENDER.get().copied() else {
        config::log("Deferred CPostEffects::Render has no trampoline");
        return;
    };
    if POST_EFFECTS_REPLAY_ACTIVE.with(|active| active.replace(true)) {
        config::log("Recursive deferred CPostEffects::Render skipped");
        return;
    }
    unsafe { original() };
    POST_EFFECTS_REPLAY_ACTIVE.with(|active| active.set(false));
}

fn finish_pending_world_frame(context: &str) {
    let device = ACTIVE_DEVICE.load(Ordering::Acquire) as *mut c_void;
    if !device.is_null()
        && let Some(device_reference) = unsafe { IDirect3DDevice9::from_raw_borrowed(&device) }
        && let Err(error) = renderer::end_world_render(device_reference)
    {
        config::log(&format!(
            "Finish world frame before {context} failed: {error}"
        ));
    }
}

unsafe extern "C" fn rw_camera_begin_update_hook(camera: *mut c_void) -> *mut c_void {
    let Some(original) = ORIGINAL_RW_CAMERA_BEGIN_UPDATE.get().copied() else {
        config::log("RwCameraBeginUpdate hook called without a trampoline");
        return std::ptr::null_mut();
    };
    let result = unsafe { original(camera) };
    if result.is_null()
        || camera as usize != game::main_camera()
        || game::auxiliary_render_scene_active()
        || mirror_render_active()
    {
        return result;
    }

    let device = ACTIVE_DEVICE.load(Ordering::Acquire) as *mut c_void;
    if !device.is_null()
        && let Some(device_reference) = unsafe { IDirect3DDevice9::from_raw_borrowed(&device) }
        && let Err(error) = renderer::rebind_world_after_camera_update(device_reference)
    {
        config::log(&format!(
            "Post-RwCameraBeginUpdate world rebind failed: {error}"
        ));
    }
    result
}

unsafe extern "system" fn direct3d_create9_hook(sdk_version: u32) -> *mut c_void {
    let Some(original) = ORIGINAL_DIRECT3D_CREATE9.get().copied() else {
        config::log("Direct3DCreate9 called without a trampoline");
        return std::ptr::null_mut();
    };
    let object = unsafe { original(sdk_version) };
    if object.is_null() {
        return object;
    }

    if let Err(error) = unsafe { hook_create_device(object) } {
        config::log(&format!("CreateDevice hook failed: {error}"));
    }
    object
}

unsafe fn hook_create_device(object: *mut c_void) -> Result<(), String> {
    let current_vtable = unsafe { *(object.cast::<*mut usize>()) };
    if current_vtable.is_null() {
        return Err("IDirect3D9 vtable is null".to_owned());
    }
    let target = unsafe { *current_vtable.add(D3D9_CREATE_DEVICE_INDEX) };
    if target == 0 {
        return Err("IDirect3D9 CreateDevice target is null".to_owned());
    }

    let mut targets = create_device_targets()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let (original_create_device, created) = match targets.get(&target).copied() {
        Some(trampoline) => (trampoline, false),
        None => {
            let trampoline = unsafe {
                MinHook::create_hook(target as *mut c_void, create_device_hook as *mut c_void)
            }
            .map_err(|status| format!("CreateDevice detour: {status}"))?
                as usize;
            targets.insert(target, trampoline);
            (trampoline, true)
        }
    };
    drop(targets);

    d3d9_hooks()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(
            object as usize,
            D3D9Hooks {
                original_create_device,
            },
        );
    if created {
        unsafe { MinHook::enable_hook(target as *mut c_void) }
            .map_err(|status| format!("Enable CreateDevice detour: {status}"))?;
    }
    config::log(&format!(
        "CreateDevice detoured: object={object:p}, target=0x{target:08X}, trampoline=0x{original_create_device:08X}"
    ));
    Ok(())
}

unsafe extern "system" fn create_device_hook(
    d3d9: *mut c_void,
    adapter: u32,
    device_type: D3DDEVTYPE,
    focus_window: HWND,
    behavior_flags: u32,
    parameters: *mut D3DPRESENT_PARAMETERS,
    returned_device: *mut *mut c_void,
) -> i32 {
    let Some(original_address) = original_create_device(d3d9) else {
        config::log("CreateDevice called without a detour record");
        return D3DERR_INVALIDCALL;
    };
    config::log(&format!(
        "CreateDevice entered: object={d3d9:p}, original=0x{original_address:08X}"
    ));
    let prepared_borderless = if config::get().borderless_fullscreen {
        match unsafe { borderless::prepare_create(focus_window, behavior_flags, parameters) } {
            Ok(prepared) => prepared,
            Err(error) => {
                config::log(&format!(
                    "Borderless CreateDevice conversion skipped: {error}"
                ));
                None
            }
        }
    } else {
        None
    };
    let original = unsafe { std::mem::transmute::<usize, CreateDeviceFn>(original_address) };
    let result = unsafe {
        original(
            d3d9,
            adapter,
            device_type,
            focus_window,
            behavior_flags,
            parameters,
            returned_device,
        )
    };
    config::log(&format!("CreateDevice returned: HRESULT=0x{result:08X}"));

    if let Some(prepared) = prepared_borderless {
        if result >= 0 {
            let (width, height) = prepared.dimensions();
            prepared.commit();
            config::log(&format!("Fullscreen borderless active: {width}x{height}"));
        } else {
            unsafe { prepared.rollback(parameters) };
            config::log("Borderless window state rolled back after CreateDevice failure");
        }
    }

    if result >= 0 && !returned_device.is_null() {
        let device = unsafe { *returned_device };
        if !device.is_null() {
            ACTIVE_DEVICE.store(device as usize, Ordering::Release);
            if let Err(error) = unsafe { hook_device_methods(device) } {
                config::log(&format!("IDirect3DDevice9 method hook failed: {error}"));
            }
        }
    }
    result
}

unsafe fn hook_device_methods(device: *mut c_void) -> Result<(), String> {
    let current_vtable = unsafe { *(device.cast::<*mut usize>()) };
    if current_vtable.is_null() {
        return Err("IDirect3DDevice9 vtable is null".to_owned());
    }
    if device_hooks()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .contains_key(&(device as usize))
    {
        return Ok(());
    }

    let reset_target = unsafe { *current_vtable.add(DEVICE_RESET_INDEX) };
    let present_target = unsafe { *current_vtable.add(DEVICE_PRESENT_INDEX) };
    let end_scene_target = unsafe { *current_vtable.add(DEVICE_END_SCENE_INDEX) };
    let clear_target = unsafe { *current_vtable.add(DEVICE_CLEAR_INDEX) };
    let (original_reset, reset_created) = unsafe {
        create_or_get_detour(
            reset_targets(),
            reset_target,
            reset_hook as *mut c_void,
            "Reset",
        )
    }?;
    let (original_end_scene, end_scene_created) = unsafe {
        create_or_get_detour(
            end_scene_targets(),
            end_scene_target,
            end_scene_hook as *mut c_void,
            "EndScene",
        )
    }?;
    let (original_clear, clear_created) = unsafe {
        create_or_get_detour(
            clear_targets(),
            clear_target,
            clear_hook as *mut c_void,
            "Clear",
        )
    }?;
    let (original_present, present_created) = if config::get().benchmark_enabled {
        match unsafe {
            create_or_get_detour(
                present_targets(),
                present_target,
                present_hook as *mut c_void,
                "Present",
            )
        } {
            Ok(hook) => hook,
            Err(error) => {
                config::log(&format!("Benchmark Present hook unavailable: {error}"));
                (0, false)
            }
        }
    } else {
        (0, false)
    };

    device_hooks()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(
            device as usize,
            DeviceHooks {
                original_reset,
                original_present,
                original_end_scene,
                original_clear,
            },
        );

    if reset_created {
        unsafe { MinHook::enable_hook(reset_target as *mut c_void) }
            .map_err(|status| format!("Enable Reset detour: {status}"))?;
    }
    if end_scene_created {
        unsafe { MinHook::enable_hook(end_scene_target as *mut c_void) }
            .map_err(|status| format!("Enable EndScene detour: {status}"))?;
    }
    if clear_created {
        unsafe { MinHook::enable_hook(clear_target as *mut c_void) }
            .map_err(|status| format!("Enable Clear detour: {status}"))?;
    }
    if present_created
        && let Err(status) = unsafe { MinHook::enable_hook(present_target as *mut c_void) }
    {
        config::log(&format!(
            "Benchmark Present hook could not be enabled: {status}"
        ));
    }
    config::log(&format!(
        "Device methods detoured: device={device:p}, Reset=0x{reset_target:08X}, Present=0x{present_target:08X}, EndScene=0x{end_scene_target:08X}, Clear=0x{clear_target:08X}, benchmark_present={}",
        original_present != 0
    ));
    Ok(())
}

unsafe fn create_or_get_detour(
    targets: &'static Mutex<HashMap<usize, usize>>,
    target: usize,
    detour: *mut c_void,
    name: &str,
) -> Result<(usize, bool), String> {
    if target == 0 {
        return Err(format!("{name} target is null"));
    }
    let mut targets = targets.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(trampoline) = targets.get(&target).copied() {
        return Ok((trampoline, false));
    }

    let trampoline = unsafe { MinHook::create_hook(target as *mut c_void, detour) }
        .map_err(|status| format!("{name} detour: {status}"))? as usize;
    targets.insert(target, trampoline);
    Ok((trampoline, true))
}

unsafe extern "system" fn clear_hook(
    device: *mut c_void,
    count: u32,
    rectangles: *const D3DRECT,
    flags: u32,
    color: u32,
    depth: f32,
    stencil: u32,
) -> i32 {
    let Some(original_address) = original_clear(device) else {
        config::log("Clear called without a detour record");
        return D3DERR_INVALIDCALL;
    };
    let capture_color = flags & (D3DCLEAR_TARGET as u32) != 0
        && count == 0
        && unsafe { IDirect3DDevice9::from_raw_borrowed(&device) }.is_some_and(|device_reference| {
            let target = unsafe { device_reference.GetRenderTarget(0) }.ok();
            let back_buffer = unsafe {
                device_reference.GetBackBuffer(
                    0,
                    0,
                    windows::Win32::Graphics::Direct3D9::D3DBACKBUFFER_TYPE_MONO,
                )
            }
            .ok();
            matches!((target, back_buffer), (Some(target), Some(back_buffer)) if target.as_raw() == back_buffer.as_raw())
        });
    let original = unsafe { std::mem::transmute::<usize, ClearFn>(original_address) };
    let result = unsafe { original(device, count, rectangles, flags, color, depth, stencil) };
    if result >= 0 && capture_color {
        renderer::capture_native_clear_color(color);
    }
    result
}

unsafe extern "system" fn end_scene_hook(device: *mut c_void) -> i32 {
    let Some(original_address) = original_end_scene(device) else {
        config::log("EndScene called without a detour record");
        return D3DERR_INVALIDCALL;
    };

    finish_deferred_post_effects("EndScene fallback");
    clear_outer_frame_gate();

    let original = unsafe { std::mem::transmute::<usize, SceneFn>(original_address) };
    if POST_PROCESS_ACTIVE.with(Cell::get) {
        return unsafe { original(device) };
    }

    // Finish the game's scene first. StretchRect is invalid inside an active
    // BeginScene/EndScene pair, so post-processing runs afterward in its own scene.
    let result = unsafe { original(device) };
    if result < 0 {
        return result;
    }

    let was_observed = END_SCENE_OBSERVED.swap(true, Ordering::AcqRel);
    if !was_observed {
        config::log("EndScene observed; post-processing armed");
        return result;
    }

    POST_PROCESS_ACTIVE.with(|active| active.set(true));
    if let Some(device_reference) = unsafe { IDirect3DDevice9::from_raw_borrowed(&device) } {
        if let Err(error) = renderer::process_frame(device_reference) {
            config::log(&format!("Post-process failed: {error}"));
        }
        dynlod::on_frame();
        if let Err(error) = overlay::draw(device_reference) {
            config::log(&format!("Diagnostic overlay failed: {error}"));
        }
    } else {
        dynlod::on_frame();
    }
    POST_PROCESS_ACTIVE.with(|active| active.set(false));
    result
}

unsafe extern "system" fn present_hook(
    device: *mut c_void,
    source_rect: *const RECT,
    destination_rect: *const RECT,
    destination_window: HWND,
    dirty_region: *const RGNDATA,
) -> i32 {
    let Some(original_address) = original_present(device) else {
        config::log("Present called without a detour record");
        return D3DERR_INVALIDCALL;
    };
    let original = unsafe { std::mem::transmute::<usize, PresentFn>(original_address) };
    let result = unsafe {
        original(
            device,
            source_rect,
            destination_rect,
            destination_window,
            dirty_region,
        )
    };
    if result >= 0 {
        benchmark::on_present_success();
    }
    result
}

unsafe extern "system" fn reset_hook(
    device: *mut c_void,
    parameters: *mut D3DPRESENT_PARAMETERS,
) -> i32 {
    let Some(original_address) = original_reset(device) else {
        config::log("Reset called without a detour record");
        return D3DERR_INVALIDCALL;
    };

    benchmark::reset("D3D9 device reset");
    renderer::invalidate();
    let prepared_borderless = if config::get().borderless_fullscreen {
        match unsafe { borderless::prepare_reset(parameters) } {
            Ok(prepared) => prepared,
            Err(error) => {
                config::log(&format!("Borderless Reset conversion skipped: {error}"));
                None
            }
        }
    } else {
        None
    };
    let original = unsafe { std::mem::transmute::<usize, ResetFn>(original_address) };
    let result = unsafe { original(device, parameters) };
    if let Some(prepared) = prepared_borderless {
        if result >= 0 {
            prepared.commit();
            config::log("Fullscreen borderless reapplied after Reset");
        } else {
            unsafe { prepared.rollback(parameters) };
            config::log("Borderless window state rolled back after Reset failure");
        }
    }
    result
}

fn original_create_device(object: *mut c_void) -> Option<usize> {
    d3d9_hooks()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&(object as usize))
        .map(|hooks| hooks.original_create_device)
}

fn original_end_scene(device: *mut c_void) -> Option<usize> {
    device_hooks()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&(device as usize))
        .map(|hooks| hooks.original_end_scene)
}

fn original_present(device: *mut c_void) -> Option<usize> {
    device_hooks()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&(device as usize))
        .map(|hooks| hooks.original_present)
        .filter(|&address| address != 0)
}

fn original_reset(device: *mut c_void) -> Option<usize> {
    device_hooks()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&(device as usize))
        .map(|hooks| hooks.original_reset)
}

fn original_clear(device: *mut c_void) -> Option<usize> {
    device_hooks()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&(device as usize))
        .map(|hooks| hooks.original_clear)
}

fn d3d9_hooks() -> &'static Mutex<HashMap<usize, D3D9Hooks>> {
    D3D9_HOOKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn create_device_targets() -> &'static Mutex<HashMap<usize, usize>> {
    CREATE_DEVICE_TARGETS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn reset_targets() -> &'static Mutex<HashMap<usize, usize>> {
    RESET_TARGETS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn present_targets() -> &'static Mutex<HashMap<usize, usize>> {
    PRESENT_TARGETS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn end_scene_targets() -> &'static Mutex<HashMap<usize, usize>> {
    END_SCENE_TARGETS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn clear_targets() -> &'static Mutex<HashMap<usize, usize>> {
    CLEAR_TARGETS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn device_hooks() -> &'static Mutex<HashMap<usize, DeviceHooks>> {
    DEVICE_HOOKS.get_or_init(|| Mutex::new(HashMap::new()))
}

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::System::LibraryLoader::GetModuleHandleA;
use windows::core::s;

const EXECUTABLE_SIGNATURE_ADDRESS: usize = 0x40_1000;
const GTA_SA_10_US_COMPACT_SIGNATURE: u32 = 0x53EC_8B55;
const GTA_SA_10_US_HOODLUM_SIGNATURE: u32 = 0x1619_7BE9;
const GAME_STATE_ADDRESS: usize = 0xC8_D4C0;
const MENU_ACTIVE_ADDRESS: usize = 0xBA_67A4;
const GAME_NOT_LOADED_ADDRESS: usize = 0xBA_6831;
const LOADING_SCREEN_ACTIVE_ADDRESS: usize = 0xBA_B318;
const CODE_PAUSE_ADDRESS: usize = 0xB7_CB48;
const USER_PAUSE_ADDRESS: usize = 0xB7_CB49;
const STREAMING_INITIALIZED_ADDRESS: usize = 0x96_54B8;
const LOADING_SCENE_ADDRESS: usize = 0x96_54BD;
const MODELS_REQUESTED_ADDRESS: usize = 0x8E_4CB8;
const PLAYER_INFO_ADDRESS: usize = 0xB7_CD98;
const PLAYER_STATE_OFFSET: usize = 0xDC;
const AUXILIARY_RENDER_SCENE_ADDRESS: usize = 0xC7_C728;
const MAIN_CAMERA_ADDRESS: usize = 0xC1_703C;
const THE_CAMERA_ADDRESS: usize = 0xB6_F028;
const ENTRY_EXIT_STATE_ADDRESS: usize = 0x96_A7CC;
const SCREEN_FADE_STATUS_ADDRESS: usize = 0x50_AE20;
const GAME_STATE_PLAYING: i32 = 9;
const PLAYER_STATE_PLAYING: u8 = 0;

static SUPPORTED_EXECUTABLE: OnceLock<bool> = OnceLock::new();
static START_GAME_REQUESTED: AtomicBool = AtomicBool::new(false);
static REINIT_SEEN: AtomicBool = AtomicBool::new(false);

struct WorldSnapshot {
    game_state: i32,
    menu_active: bool,
    game_not_loaded: bool,
    loading_screen_active: bool,
    code_paused: bool,
    user_paused: bool,
    streaming_initialized: bool,
    loading_scene: bool,
    models_requested: u32,
    player_ped: usize,
    player_state: u8,
}

pub fn is_supported_executable() -> bool {
    *SUPPORTED_EXECUTABLE.get_or_init(|| {
        let signature = unsafe { read_address::<u32>(EXECUTABLE_SIGNATURE_ADDRESS) };
        matches!(
            signature,
            GTA_SA_10_US_COMPACT_SIGNATURE | GTA_SA_10_US_HOODLUM_SIGNATURE
        )
    })
}

pub fn mark_start_game_requested() {
    REINIT_SEEN.store(false, Ordering::Release);
    START_GAME_REQUESTED.store(true, Ordering::Release);
}

pub fn mark_reinit_seen() {
    if START_GAME_REQUESTED.load(Ordering::Acquire) {
        REINIT_SEEN.store(true, Ordering::Release);
    }
}

pub fn ready_player_ped() -> Option<usize> {
    if !is_supported_executable()
        || !START_GAME_REQUESTED.load(Ordering::Acquire)
        || !REINIT_SEEN.load(Ordering::Acquire)
    {
        return None;
    }

    readiness_candidate(current_world_snapshot())
}

pub fn world_activation_player_ped() -> Option<usize> {
    if !is_supported_executable() {
        return None;
    }
    readiness_candidate(current_world_snapshot())
}

fn current_world_snapshot() -> WorldSnapshot {
    WorldSnapshot {
        game_state: unsafe { read_address::<i32>(GAME_STATE_ADDRESS) },
        menu_active: unsafe { read_address::<u8>(MENU_ACTIVE_ADDRESS) } != 0,
        game_not_loaded: unsafe { read_address::<u8>(GAME_NOT_LOADED_ADDRESS) } != 0,
        loading_screen_active: unsafe { read_address::<u8>(LOADING_SCREEN_ACTIVE_ADDRESS) } != 0,
        code_paused: unsafe { read_address::<u8>(CODE_PAUSE_ADDRESS) } != 0,
        user_paused: unsafe { read_address::<u8>(USER_PAUSE_ADDRESS) } != 0,
        streaming_initialized: unsafe { read_address::<i32>(STREAMING_INITIALIZED_ADDRESS) } != 0,
        loading_scene: unsafe { read_address::<u8>(LOADING_SCENE_ADDRESS) } != 0,
        models_requested: unsafe { read_address::<u32>(MODELS_REQUESTED_ADDRESS) },
        player_ped: unsafe { read_address::<usize>(PLAYER_INFO_ADDRESS) },
        player_state: unsafe { read_address::<u8>(PLAYER_INFO_ADDRESS + PLAYER_STATE_OFFSET) },
    }
}

pub fn auxiliary_render_scene_active() -> bool {
    is_supported_executable() && unsafe { read_address::<u8>(AUXILIARY_RENDER_SCENE_ADDRESS) } != 0
}

pub fn world_render_probe_ready() -> bool {
    if !is_supported_executable() {
        return false;
    }

    let game_state = unsafe { read_address::<i32>(GAME_STATE_ADDRESS) };
    let menu_active = unsafe { read_address::<u8>(MENU_ACTIVE_ADDRESS) } != 0;
    let loading_screen = unsafe { read_address::<u8>(LOADING_SCREEN_ACTIVE_ADDRESS) } != 0;
    let player_ped = unsafe { read_address::<usize>(PLAYER_INFO_ADDRESS) };
    let player_state = unsafe { read_address::<u8>(PLAYER_INFO_ADDRESS + PLAYER_STATE_OFFSET) };
    game_state == GAME_STATE_PLAYING
        && !menu_active
        && !loading_screen
        && player_ped != 0
        && player_state == PLAYER_STATE_PLAYING
}

pub fn world_render_stable() -> bool {
    if !is_supported_executable() {
        return false;
    }

    let snapshot = current_world_snapshot();
    let entry_exit_state = unsafe { read_address::<i32>(ENTRY_EXIT_STATE_ADDRESS) };
    if !runtime_world_base_candidate(&snapshot, entry_exit_state) {
        return false;
    }
    let fade_status = unsafe {
        let get_status = std::mem::transmute::<
            usize,
            unsafe extern "thiscall" fn(*mut std::ffi::c_void) -> i32,
        >(SCREEN_FADE_STATUS_ADDRESS);
        get_status(THE_CAMERA_ADDRESS as *mut std::ffi::c_void)
    };
    runtime_world_candidate(snapshot, entry_exit_state, fade_status)
}

pub fn diagnostic_snapshot() -> String {
    if !is_supported_executable() {
        return "supported_executable=false".to_owned();
    }
    let snapshot = current_world_snapshot();
    let entry_exit_state = unsafe { read_address::<i32>(ENTRY_EXIT_STATE_ADDRESS) };
    let auxiliary_scene = unsafe { read_address::<u8>(AUXILIARY_RENDER_SCENE_ADDRESS) } != 0;
    let main_camera = unsafe { read_address::<usize>(MAIN_CAMERA_ADDRESS) };
    let samp_dll_loaded = unsafe { GetModuleHandleA(s!("samp.dll")) }.is_ok();
    let executable_module_base = unsafe { GetModuleHandleA(s!("gta_sa.exe")) }
        .map(|module| module.0 as usize)
        .unwrap_or(0);
    format!(
        "supported_executable=true, executable_module_base=0x{:08X}, game_state={}, menu_active={}, game_not_loaded={}, loading_screen={}, code_paused={}, user_paused={}, streaming_initialized={}, loading_scene={}, models_requested={}, player_ped=0x{:08X}, player_state={}, entry_exit_state={}, auxiliary_render_scene={}, main_camera=0x{:08X}, samp_dll_loaded={}",
        executable_module_base,
        snapshot.game_state,
        snapshot.menu_active,
        snapshot.game_not_loaded,
        snapshot.loading_screen_active,
        snapshot.code_paused,
        snapshot.user_paused,
        snapshot.streaming_initialized,
        snapshot.loading_scene,
        snapshot.models_requested,
        snapshot.player_ped,
        snapshot.player_state,
        entry_exit_state,
        auxiliary_scene,
        main_camera,
        samp_dll_loaded
    )
}

pub fn main_camera() -> usize {
    if !is_supported_executable() {
        return 0;
    }
    unsafe { read_address::<usize>(MAIN_CAMERA_ADDRESS) }
}

fn readiness_candidate(snapshot: WorldSnapshot) -> Option<usize> {
    (snapshot.game_state == GAME_STATE_PLAYING
        && !snapshot.menu_active
        && !snapshot.game_not_loaded
        && !snapshot.loading_screen_active
        && !snapshot.code_paused
        && !snapshot.user_paused
        && snapshot.streaming_initialized
        && !snapshot.loading_scene
        && snapshot.models_requested == 0
        && snapshot.player_ped != 0
        && snapshot.player_state == PLAYER_STATE_PLAYING)
        .then_some(snapshot.player_ped)
}

fn runtime_world_candidate(
    snapshot: WorldSnapshot,
    entry_exit_state: i32,
    fade_status: i32,
) -> bool {
    runtime_world_base_candidate(&snapshot, entry_exit_state) && fade_status == 0
}

fn runtime_world_base_candidate(snapshot: &WorldSnapshot, entry_exit_state: i32) -> bool {
    snapshot.game_state == GAME_STATE_PLAYING
        && !snapshot.menu_active
        && !snapshot.game_not_loaded
        && !snapshot.loading_screen_active
        && !snapshot.code_paused
        && !snapshot.user_paused
        && snapshot.player_ped != 0
        && snapshot.player_state == PLAYER_STATE_PLAYING
        && entry_exit_state == 0
}

unsafe fn read_address<T: Copy>(address: usize) -> T {
    unsafe { std::ptr::read_volatile(address as *const T) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_snapshot() -> WorldSnapshot {
        WorldSnapshot {
            game_state: 9,
            menu_active: false,
            game_not_loaded: false,
            loading_screen_active: false,
            code_paused: false,
            user_paused: false,
            streaming_initialized: true,
            loading_scene: false,
            models_requested: 0,
            player_ped: 0x1234,
            player_state: 0,
        }
    }

    #[test]
    fn accepts_ready_world() {
        assert_eq!(readiness_candidate(ready_snapshot()), Some(0x1234));
    }

    #[test]
    fn rejects_active_loading_or_streaming() {
        let mut snapshot = ready_snapshot();
        snapshot.loading_scene = true;
        assert_eq!(readiness_candidate(snapshot), None);

        let mut snapshot = ready_snapshot();
        snapshot.models_requested = 1;
        assert_eq!(readiness_candidate(snapshot), None);
    }

    #[test]
    fn rejects_invalid_player_state() {
        let mut snapshot = ready_snapshot();
        snapshot.player_state = 1;
        assert_eq!(readiness_candidate(snapshot), None);
    }

    #[test]
    fn runtime_gate_allows_normal_model_streaming() {
        let mut snapshot = ready_snapshot();
        snapshot.streaming_initialized = false;
        snapshot.loading_scene = true;
        snapshot.models_requested = 12;
        assert!(runtime_world_candidate(snapshot, 0, 0));
    }

    #[test]
    fn runtime_gate_rejects_transition_or_fade() {
        assert!(!runtime_world_candidate(ready_snapshot(), 1, 0));
        assert!(!runtime_world_candidate(ready_snapshot(), 0, 1));
    }
}

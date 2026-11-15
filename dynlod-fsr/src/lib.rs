#![allow(non_snake_case)]

mod benchmark;
mod borderless;
mod config;
mod dynlod;
mod game;
mod hooks;
mod menu;
mod overlay;
mod renderer;
mod shaders;

use core::ffi::c_void;

use windows::Win32::Foundation::{CloseHandle, HINSTANCE, TRUE};
use windows::Win32::System::LibraryLoader::DisableThreadLibraryCalls;
use windows::Win32::System::SystemServices::DLL_PROCESS_ATTACH;
use windows::Win32::System::Threading::{CreateThread, THREAD_CREATION_FLAGS};
use windows::core::BOOL;

#[unsafe(no_mangle)]
unsafe extern "system" fn DllMain(
    instance: HINSTANCE,
    reason: u32,
    _reserved: *mut c_void,
) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        let _ = unsafe { DisableThreadLibraryCalls(instance.into()) };
        if let Ok(thread) = unsafe {
            CreateThread(
                None,
                0,
                Some(initialize),
                None,
                THREAD_CREATION_FLAGS(0),
                None,
            )
        } {
            let _ = unsafe { CloseHandle(thread) };
        }
    }

    TRUE
}

unsafe extern "system" fn initialize(_parameter: *mut c_void) -> u32 {
    config::initialize();
    dynlod::initialize();

    match unsafe { hooks::install() } {
        Ok(()) => config::log("D3D9 hook bootstrap ready"),
        Err(error) => config::log(&format!("Hook installation failed: {error}")),
    }

    0
}

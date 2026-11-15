use core::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{HWND, RECT, TRUE};
use windows::Win32::Graphics::Direct3D9::{
    D3DCREATE_ADAPTERGROUP_DEVICE, D3DFMT_UNKNOWN, D3DPRESENT_INTERVAL_DEFAULT,
    D3DPRESENT_INTERVAL_FOUR, D3DPRESENT_INTERVAL_THREE, D3DPRESENT_INTERVAL_TWO,
    D3DPRESENT_PARAMETERS,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetWindowLongPtrW, GetWindowRect, IsWindow, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, WS_BORDER, WS_DLGFRAME,
    WS_EX_APPWINDOW, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_STATICEDGE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_WINDOWEDGE, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_POPUPWINDOW,
};

static ACTIVE_WINDOW: AtomicIsize = AtomicIsize::new(0);

pub struct PreparedBorderless {
    window: WindowSnapshot,
    presentation: D3DPRESENT_PARAMETERS,
    monitor_rect: RECT,
}

#[derive(Clone, Copy)]
struct WindowSnapshot {
    hwnd: HWND,
    style: i32,
    ex_style: i32,
    rect: RECT,
}

impl PreparedBorderless {
    pub fn dimensions(&self) -> (i32, i32) {
        (
            self.monitor_rect.right - self.monitor_rect.left,
            self.monitor_rect.bottom - self.monitor_rect.top,
        )
    }

    pub fn commit(self) {
        ACTIVE_WINDOW.store(self.window.hwnd.0 as isize, Ordering::Release);
    }

    pub unsafe fn rollback(self, parameters: *mut D3DPRESENT_PARAMETERS) {
        if !parameters.is_null() {
            unsafe { *parameters = self.presentation };
        }
        unsafe { restore_window(self.window) };
    }
}

pub unsafe fn prepare_create(
    focus_window: HWND,
    behavior_flags: u32,
    parameters: *mut D3DPRESENT_PARAMETERS,
) -> Result<Option<PreparedBorderless>, String> {
    if parameters.is_null()
        || behavior_flags & (D3DCREATE_ADAPTERGROUP_DEVICE as u32) != 0
        || unsafe { (*parameters).Windowed.as_bool() }
    {
        return Ok(None);
    }

    let device_window = unsafe { (*parameters).hDeviceWindow };
    let hwnd = if !device_window.0.is_null() {
        device_window
    } else {
        focus_window
    };
    unsafe { prepare(hwnd, parameters) }.map(Some)
}

pub unsafe fn prepare_reset(
    parameters: *mut D3DPRESENT_PARAMETERS,
) -> Result<Option<PreparedBorderless>, String> {
    let address = ACTIVE_WINDOW.load(Ordering::Acquire);
    if address == 0 || parameters.is_null() {
        return Ok(None);
    }

    let hwnd = HWND(address as *mut c_void);
    unsafe { prepare(hwnd, parameters) }.map(Some)
}

unsafe fn prepare(
    hwnd: HWND,
    parameters: *mut D3DPRESENT_PARAMETERS,
) -> Result<PreparedBorderless, String> {
    if hwnd.0.is_null() || !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        return Err("device window is null or invalid".to_owned());
    }

    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.0.is_null() {
        return Err("MonitorFromWindow returned null".to_owned());
    }
    let mut monitor_info = MONITORINFO {
        cbSize: size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut monitor_info).as_bool() } {
        return Err("GetMonitorInfoW failed".to_owned());
    }

    let mut window_rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut window_rect) }
        .map_err(|error| format!("GetWindowRect: {error}"))?;
    let snapshot = WindowSnapshot {
        hwnd,
        style: unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) },
        ex_style: unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) },
        rect: window_rect,
    };
    let presentation = unsafe { *parameters };

    unsafe {
        SetWindowLongPtrW(hwnd, GWL_STYLE, borderless_style(snapshot.style));
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, borderless_ex_style(snapshot.ex_style));
    }
    let monitor_rect = monitor_info.rcMonitor;
    if let Err(error) = unsafe {
        SetWindowPos(
            hwnd,
            None,
            monitor_rect.left,
            monitor_rect.top,
            monitor_rect.right - monitor_rect.left,
            monitor_rect.bottom - monitor_rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
    } {
        unsafe { restore_window(snapshot) };
        return Err(format!("SetWindowPos: {error}"));
    }

    unsafe { normalize_presentation(&mut *parameters, hwnd) };
    Ok(PreparedBorderless {
        window: snapshot,
        presentation,
        monitor_rect,
    })
}

unsafe fn restore_window(snapshot: WindowSnapshot) {
    unsafe {
        SetWindowLongPtrW(snapshot.hwnd, GWL_STYLE, snapshot.style);
        SetWindowLongPtrW(snapshot.hwnd, GWL_EXSTYLE, snapshot.ex_style);
    }
    let _ = unsafe {
        SetWindowPos(
            snapshot.hwnd,
            None,
            snapshot.rect.left,
            snapshot.rect.top,
            snapshot.rect.right - snapshot.rect.left,
            snapshot.rect.bottom - snapshot.rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
    };
}

unsafe fn normalize_presentation(parameters: &mut D3DPRESENT_PARAMETERS, hwnd: HWND) {
    parameters.hDeviceWindow = hwnd;
    parameters.Windowed = TRUE;
    parameters.BackBufferWidth = 0;
    parameters.BackBufferHeight = 0;
    parameters.BackBufferFormat = D3DFMT_UNKNOWN;
    parameters.FullScreen_RefreshRateInHz = 0;
    parameters.PresentationInterval = windowed_interval(parameters.PresentationInterval);
}

fn borderless_style(style: i32) -> i32 {
    let remove = WS_OVERLAPPEDWINDOW.0 | WS_POPUPWINDOW.0 | WS_BORDER.0 | WS_DLGFRAME.0;
    (((style as u32) & !remove) | WS_POPUP.0) as i32
}

fn borderless_ex_style(ex_style: i32) -> i32 {
    let remove = WS_EX_TOOLWINDOW.0
        | WS_EX_WINDOWEDGE.0
        | WS_EX_CLIENTEDGE.0
        | WS_EX_STATICEDGE.0
        | WS_EX_DLGMODALFRAME.0
        | WS_EX_TOPMOST.0;
    (((ex_style as u32) & !remove) | WS_EX_APPWINDOW.0) as i32
}

fn windowed_interval(interval: u32) -> u32 {
    match interval as i32 {
        D3DPRESENT_INTERVAL_TWO | D3DPRESENT_INTERVAL_THREE | D3DPRESENT_INTERVAL_FOUR => {
            D3DPRESENT_INTERVAL_DEFAULT as u32
        }
        _ => interval,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::WindowsAndMessaging::{WS_CAPTION, WS_EX_TOPMOST, WS_VISIBLE};

    #[test]
    fn converts_window_styles_without_hiding_the_window() {
        let style = borderless_style((WS_OVERLAPPEDWINDOW | WS_VISIBLE).0 as i32) as u32;
        assert_ne!(style & WS_POPUP.0, 0);
        assert_ne!(style & WS_VISIBLE.0, 0);
        assert_eq!(style & WS_CAPTION.0, 0);

        let ex_style = borderless_ex_style((WS_EX_TOPMOST | WS_EX_TOOLWINDOW).0 as i32) as u32;
        assert_ne!(ex_style & WS_EX_APPWINDOW.0, 0);
        assert_eq!(ex_style & (WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0), 0);
    }

    #[test]
    fn normalizes_fullscreen_only_present_intervals() {
        assert_eq!(
            windowed_interval(D3DPRESENT_INTERVAL_TWO as u32),
            D3DPRESENT_INTERVAL_DEFAULT as u32
        );
        assert_eq!(windowed_interval(1), 1);
        assert_eq!(windowed_interval(u32::MAX), u32::MAX);
    }
}

//! Small safe wrappers around the Win32 calls the app needs.

use crate::render::{Canvas, Rgb};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    CreateBitmap, CreateDIBSection, DeleteObject, GetDC, GetPixel, ReleaseDC, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS,
};
use windows_sys::Win32::System::Console::{AllocConsole, AttachConsole, ATTACH_PARENT_PROCESS};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Copy `s` into a fixed-size UTF-16 buffer, truncating and NUL-terminating.
pub fn copy_wide(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

pub fn attach_console() {
    unsafe {
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            AllocConsole();
        }
    }
}

/// True when another copy is still running for this user after waiting up to `wait` for
/// it to exit (used right after an update). The mutex is kept for the lifetime of the
/// process on purpose.
pub fn already_running(wait: Duration) -> bool {
    let name = wide("Local\\MouseBattery.SingleInstance");
    let end = Instant::now() + wait;
    loop {
        unsafe {
            let handle = CreateMutexW(null(), 0, name.as_ptr());
            if handle.is_null() || GetLastError() != ERROR_ALREADY_EXISTS {
                return false;
            }
            CloseHandle(handle);
        }
        if Instant::now() >= end {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

pub fn message_box(title: &str, text: &str) {
    let (t, m) = (wide(title), wide(text));
    unsafe {
        MessageBoxW(null_mut(), m.as_ptr(), t.as_ptr(), MB_OK | MB_ICONINFORMATION);
    }
}

pub fn set_dpi_aware() {
    unsafe {
        SetProcessDPIAware();
    }
}

pub fn dpi_scale() -> f32 {
    unsafe { GetDpiForSystem() as f32 / 96.0 }
}

pub fn tray_icon_size() -> usize {
    unsafe { GetSystemMetrics(SM_CXSMICON).max(16) as usize }
}

pub fn window_rect(hwnd: HWND) -> Option<RECT> {
    if hwnd.is_null() {
        return None;
    }
    let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
    (unsafe { GetWindowRect(hwnd, &mut r) } != 0).then_some(r)
}

pub fn screen_size() -> (i32, i32) {
    unsafe { (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN)) }
}

pub fn taskbar_rect() -> RECT {
    let class = wide("Shell_TrayWnd");
    let hwnd = unsafe { FindWindowW(class.as_ptr(), null()) };
    match window_rect(hwnd) {
        Some(r) if r.right > r.left && r.bottom > r.top => r,
        _ => {
            let (w, h) = screen_size();
            RECT { left: 0, top: h - 48, right: w, bottom: h }
        }
    }
}

/// Left edge of the notification area (clock + tray icons), if Windows tells us.
pub fn tray_left() -> Option<i32> {
    let (tray, notify) = (wide("Shell_TrayWnd"), wide("TrayNotifyWnd"));
    unsafe {
        let tb = FindWindowW(tray.as_ptr(), null());
        if tb.is_null() {
            return None;
        }
        let r = window_rect(FindWindowExW(tb, null_mut(), notify.as_ptr(), null()))?;
        (r.right > r.left).then_some(r.left)
    }
}

pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

pub fn screen_pixel(x: i32, y: i32) -> Option<Rgb> {
    unsafe {
        let dc = GetDC(null_mut());
        if dc.is_null() {
            return None;
        }
        let c = GetPixel(dc, x, y);
        ReleaseDC(null_mut(), dc);
        (c != u32::MAX).then_some(((c & 0xFF) as u8, ((c >> 8) & 0xFF) as u8, ((c >> 16) & 0xFF) as u8))
    }
}

/// A fullscreen game or video in front (the widget hides then).
pub fn fullscreen_app_active(own: HWND) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_null() || fg == own {
            return false;
        }
        let mut buf = [0u16; 64];
        let n = GetClassNameW(fg, buf.as_mut_ptr(), buf.len() as i32);
        let class = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        if ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"].contains(&class.as_str()) {
            return false;
        }
        let (w, h) = screen_size();
        window_rect(fg).is_some_and(|r| r.left <= 0 && r.top <= 0 && r.right >= w && r.bottom >= h)
    }
}

pub fn bitmap_info(w: usize, h: usize) -> BITMAPINFO {
    let mut bmi: BITMAPINFO = unsafe { std::mem::zeroed() };
    bmi.bmiHeader = BITMAPINFOHEADER {
        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
        biWidth: w as i32,
        biHeight: -(h as i32), // top-down rows
        biPlanes: 1,
        biBitCount: 32,
        biCompression: BI_RGB,
        biSizeImage: 0,
        biXPelsPerMeter: 0,
        biYPelsPerMeter: 0,
        biClrUsed: 0,
        biClrImportant: 0,
    };
    bmi
}

/// Build an icon handle from 32-bit ARGB pixels. Free it with DestroyIcon.
pub fn make_icon(c: &Canvas) -> HICON {
    unsafe {
        let bmi = bitmap_info(c.w, c.h);
        let mut bits: *mut core::ffi::c_void = null_mut();
        let color = CreateDIBSection(null_mut(), &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if color.is_null() || bits.is_null() {
            return null_mut();
        }
        std::ptr::copy_nonoverlapping(c.px.as_ptr(), bits as *mut u32, c.px.len());
        let mask_bits = vec![0u8; c.w.div_ceil(16) * 2 * c.h];
        let mask = CreateBitmap(c.w as i32, c.h as i32, 1, 1, mask_bits.as_ptr() as _);
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color);
        DeleteObject(mask);
        icon
    }
}

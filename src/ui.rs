//! The window side: one borderless always-on-top widget window on the taskbar that also
//! owns the tray icon. A background thread polls the mice and posts WM_APP_UPDATE.

use crate::config::Config;
use crate::render::{self, Canvas, Rgb};
use crate::state::{AppState, Entry, POLL};
use crate::{devices, startup, win, APP_NAME};
use std::cell::RefCell;
use std::ptr::{null, null_mut};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{BeginPaint, EndPaint, InvalidateRect, SetDIBitsToDevice, DIB_RGB_COLORS, PAINTSTRUCT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_APP_TRAY: u32 = WM_APP + 1;
const WM_APP_UPDATE: u32 = WM_APP + 2;
const TIMER_ID: usize = 1;
const CLASS_NAME: &str = "MouseBatteryWidget";

const CMD_REFRESH: usize = 1;
const CMD_TOGGLE_WIDGET: usize = 2;
const CMD_TOGGLE_STARTUP: usize = 3;
const CMD_RESET_POSITION: usize = 4;
const CMD_QUIT: usize = 5;

type DrawKey = (Vec<Entry>, Rgb, usize);

struct Ui {
    hwnd: HWND,
    state: Arc<Mutex<AppState>>,
    refresh: Sender<()>,
    cfg: Config,
    scale: f32,
    bg: Rgb,
    ticks: u32,
    canvas: Option<Canvas>,
    drawn: Option<DrawKey>,
    drag_dx: Option<i32>,
    tray_icon: HICON,
    tray_entry: Option<Option<Entry>>,
    taskbar_created: u32,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

/// Run `f` with the UI state unless it is already borrowed (re-entrant window messages).
fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|cell| cell.try_borrow_mut().ok().and_then(|mut ui| ui.as_mut().map(f)))
}

pub fn run() {
    let state = Arc::new(Mutex::new(AppState::default()));
    let (tx, rx) = mpsc::channel::<()>();

    let hwnd = unsafe { create_window() };
    if hwnd.is_null() {
        win::message_box(APP_NAME, "Could not create the widget window.");
        return;
    }

    // background polling thread
    {
        let state = state.clone();
        let target = hwnd as isize;
        std::thread::spawn(move || {
            let mut api = None;
            loop {
                if api.is_none() {
                    api = hidapi::HidApi::new().ok();
                }
                if let Some(api) = api.as_mut() {
                    let readings = devices::read_all(api);
                    state.lock().unwrap().apply(readings);
                }
                unsafe { PostMessageW(target as HWND, WM_APP_UPDATE, 0, 0) };
                match rx.recv_timeout(POLL) {
                    Ok(()) => while rx.try_recv().is_ok() {},
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
        });
    }

    let taskbar_created = unsafe { RegisterWindowMessageW(win::wide("TaskbarCreated").as_ptr()) };
    UI.with(|cell| {
        *cell.borrow_mut() = Some(Ui {
            hwnd,
            state,
            refresh: tx,
            cfg: Config::load(),
            scale: win::dpi_scale(),
            bg: (32, 32, 32),
            ticks: 0,
            canvas: None,
            drawn: None,
            drag_dx: None,
            tray_icon: null_mut(),
            tray_entry: None,
            taskbar_created,
        })
    });
    with_ui(|ui| {
        ui.update_tray(NIM_ADD);
        ui.update_widget();
    });
    unsafe {
        SetTimer(hwnd, TIMER_ID, 1000, None);
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

unsafe fn create_window() -> HWND {
    let class = win::wide(CLASS_NAME);
    let instance = GetModuleHandleW(null());
    let wc = WNDCLASSW {
        style: CS_DBLCLKS,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: null_mut(),
        hCursor: LoadCursorW(null_mut(), IDC_HAND),
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: class.as_ptr(),
    };
    RegisterClassW(&wc);
    let title = win::wide(APP_NAME);
    CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class.as_ptr(),
        title.as_ptr(),
        WS_POPUP,
        0,
        0,
        1,
        1,
        null_mut(),
        null_mut(),
        instance,
        null(),
    )
}

impl Ui {
    // ---------------------------------------------------------------- tray icon

    fn update_tray(&mut self, action: u32) {
        let (tip, entry) = self.state.lock().unwrap().tray();
        let mut nid = self.nid();
        nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        nid.uCallbackMessage = WM_APP_TRAY;
        win::copy_wide(&mut nid.szTip, &tip);
        let old = self.tray_icon;
        if action == NIM_ADD || self.tray_entry != Some(entry) {
            self.tray_icon = win::make_icon(&render::render_tray_icon(entry.as_ref(), win::tray_icon_size()));
            self.tray_entry = Some(entry);
        }
        nid.hIcon = self.tray_icon;
        unsafe {
            Shell_NotifyIconW(action, &nid);
            if !old.is_null() && old != self.tray_icon {
                DestroyIcon(old);
            }
        }
    }

    fn notify(&self, text: &str) {
        let mut nid = self.nid();
        nid.uFlags = NIF_INFO;
        nid.dwInfoFlags = NIIF_INFO;
        win::copy_wide(&mut nid.szInfo, text);
        win::copy_wide(&mut nid.szInfoTitle, APP_NAME);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &nid) };
    }

    fn nid(&self) -> NOTIFYICONDATAW {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = self.hwnd;
        nid.uID = 1;
        nid
    }

    fn on_state_update(&mut self) {
        let notes: Vec<String> = std::mem::take(&mut self.state.lock().unwrap().notes);
        for note in notes {
            self.notify(&note);
        }
        self.update_tray(NIM_MODIFY);
        self.update_widget();
    }

    // ---------------------------------------------------------------- widget

    fn target_xy(&self, w: i32, h: i32) -> (i32, i32) {
        let tb = win::taskbar_rect();
        let x = match self.cfg.right_offset {
            Some(off) => tb.right - off,
            None => {
                let left = win::tray_left().unwrap_or(tb.right - (260.0 * self.scale) as i32);
                left - w - (4.0 * self.scale) as i32
            }
        };
        let x = x.min(tb.right - w).max(tb.left);
        (x, tb.top + (tb.bottom - tb.top - h) / 2)
    }

    fn size(&self) -> (i32, i32) {
        self.canvas.as_ref().map_or((0, 0), |c| (c.w as i32, c.h as i32))
    }

    /// Read the taskbar colour just beside the widget so the text blends in.
    fn sample_bg(&mut self) {
        let tb = win::taskbar_rect();
        if tb.bottom - tb.top > 200 {
            return; // vertical taskbar: not supported, keep default
        }
        let w = match self.size().0 {
            0 => (60.0 * self.scale) as i32,
            w => w,
        };
        let (x, _) = self.target_xy(w, 1);
        let px = if x - 3 > tb.left { x - 3 } else { (x + w + 3).min(tb.right - 1) };
        if let Some(c) = win::screen_pixel(px, tb.top + (tb.bottom - tb.top) / 2) {
            self.bg = c;
        }
    }

    fn update_widget(&mut self) {
        unsafe {
            if !self.cfg.visible || win::fullscreen_app_active(self.hwnd) {
                ShowWindow(self.hwnd, SW_HIDE);
                return;
            }
        }
        if self.ticks % 10 == 0 {
            self.sample_bg(); // the taskbar colour changes rarely (theme switch)
        }
        self.ticks = self.ticks.wrapping_add(1);

        let entries = self.state.lock().unwrap().entries();
        let tb = win::taskbar_rect();
        let height = ((tb.bottom - tb.top).min((40.0 * self.scale).round() as i32)).max(16) as usize;
        let key = (entries, self.bg, height);
        if self.drawn.as_ref() != Some(&key) {
            self.canvas = Some(render::render_widget(&key.0, self.bg, self.scale, height));
            self.drawn = Some(key);
            unsafe { InvalidateRect(self.hwnd, null(), 0) };
        }

        let (w, h) = self.size();
        unsafe {
            if self.drag_dx.is_none() {
                let (x, y) = self.target_xy(w, h);
                SetWindowPos(self.hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
            } else {
                SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
            }
        }
    }

    fn paint(&self) {
        unsafe {
            let mut ps: PAINTSTRUCT = std::mem::zeroed();
            let dc = BeginPaint(self.hwnd, &mut ps);
            if let Some(c) = &self.canvas {
                let bmi = win::bitmap_info(c.w, c.h);
                SetDIBitsToDevice(
                    dc, 0, 0, c.w as u32, c.h as u32, 0, 0, 0, c.h as u32,
                    c.px.as_ptr() as _, &bmi, DIB_RGB_COLORS,
                );
            }
            EndPaint(self.hwnd, &ps);
        }
    }

    fn forget_position_cache(&mut self) {
        self.drawn = None; // resample the background at the new spot
        self.ticks = 0;
    }

    // ---------------------------------------------------------------- dragging

    fn drag_start(&mut self) {
        let (cx, _) = win::cursor_pos();
        if let Some(r) = win::window_rect(self.hwnd) {
            self.drag_dx = Some(cx - r.left);
            unsafe { SetCapture(self.hwnd) };
        }
    }

    fn drag_move(&mut self) {
        let Some(dx) = self.drag_dx else { return };
        let tb = win::taskbar_rect();
        let (w, h) = self.size();
        let x = (win::cursor_pos().0 - dx).min(tb.right - w).max(tb.left);
        let y = tb.top + (tb.bottom - tb.top - h) / 2;
        unsafe { SetWindowPos(self.hwnd, null_mut(), x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
    }

    fn drag_end(&mut self) {
        if self.drag_dx.take().is_none() {
            return;
        }
        unsafe { ReleaseCapture() };
        if let Some(r) = win::window_rect(self.hwnd) {
            self.cfg.right_offset = Some(win::taskbar_rect().right - r.left);
            self.cfg.save();
        }
        self.forget_position_cache();
    }

    // ---------------------------------------------------------------- menu

    fn menu_lines(&self) -> (Vec<String>, bool) {
        (self.state.lock().unwrap().status_lines(), self.cfg.visible)
    }

    fn run_command(&mut self, cmd: usize) {
        match cmd {
            CMD_REFRESH => {
                let _ = self.refresh.send(());
            }
            CMD_TOGGLE_WIDGET => {
                self.cfg.visible = !self.cfg.visible;
                self.cfg.save();
                self.update_widget();
            }
            CMD_TOGGLE_STARTUP => startup::toggle(),
            CMD_RESET_POSITION => {
                self.cfg.right_offset = None;
                self.cfg.save();
                self.forget_position_cache();
                self.update_widget();
            }
            CMD_QUIT => unsafe {
                Shell_NotifyIconW(NIM_DELETE, &self.nid());
                DestroyWindow(self.hwnd);
            },
            _ => {}
        }
    }
}

/// Shows the menu without holding the UI borrow (the menu runs its own message loop).
fn show_menu(hwnd: HWND) {
    let Some((lines, visible)) = with_ui(|ui| ui.menu_lines()) else { return };
    let cmd = unsafe {
        let menu = CreatePopupMenu();
        for line in &lines {
            AppendMenuW(menu, MF_STRING | MF_GRAYED, 0, win::wide(line).as_ptr());
        }
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        let item = |id: usize, text: &str, checked: bool| {
            let flags = MF_STRING | if checked { MF_CHECKED } else { MF_UNCHECKED };
            AppendMenuW(menu, flags, id, win::wide(text).as_ptr());
        };
        item(CMD_REFRESH, "Refresh now", false);
        item(CMD_TOGGLE_WIDGET, "Show on taskbar", visible);
        item(CMD_TOGGLE_STARTUP, "Start with Windows", startup::enabled());
        item(CMD_RESET_POSITION, "Reset widget position", false);
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        item(CMD_QUIT, "Quit", false);
        SetMenuDefaultItem(menu, CMD_REFRESH as u32, 0);

        let (x, y) = win::cursor_pos();
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, x, y, 0, hwnd, null::<RECT>());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        cmd as usize
    };
    if cmd != 0 {
        with_ui(|ui| ui.run_command(cmd));
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            if with_ui(|ui| ui.paint()).is_some() {
                return 0;
            }
        }
        WM_TIMER if wparam == TIMER_ID => {
            with_ui(|ui| ui.update_widget());
            return 0;
        }
        WM_APP_UPDATE => {
            with_ui(|ui| ui.on_state_update());
            return 0;
        }
        WM_APP_TRAY => {
            match lparam as u32 {
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd),
                WM_LBUTTONDBLCLK => {
                    with_ui(|ui| ui.run_command(CMD_REFRESH));
                }
                _ => {}
            }
            return 0;
        }
        WM_LBUTTONDOWN => {
            with_ui(|ui| ui.drag_start());
            return 0;
        }
        WM_MOUSEMOVE => {
            with_ui(|ui| ui.drag_move());
            return 0;
        }
        WM_LBUTTONUP => {
            with_ui(|ui| ui.drag_end());
            return 0;
        }
        WM_LBUTTONDBLCLK => {
            with_ui(|ui| ui.run_command(CMD_REFRESH));
            return 0;
        }
        WM_RBUTTONUP => {
            show_menu(hwnd);
            return 0;
        }
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
        WM_DESTROY => {
            PostQuitMessage(0);
            return 0;
        }
        _ => {
            // Explorer restarted: put the tray icon back
            if with_ui(|ui| msg == ui.taskbar_created).unwrap_or(false) {
                with_ui(|ui| ui.update_tray(NIM_ADD));
                return 0;
            }
        }
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

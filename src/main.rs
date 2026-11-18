//! Mouse Battery - shows your wireless mouse's battery level on the Windows taskbar.
//!
//! Run normally:          MouseBattery.exe
//! One-shot check:        MouseBattery.exe --debug

#![windows_subsystem = "windows"]

use std::sync::atomic::{AtomicBool, Ordering};

pub static DEBUG: AtomicBool = AtomicBool::new(false);

/// println! that only prints in --debug mode.
#[macro_export]
macro_rules! dlog {
    ($($t:tt)*) => {
        if $crate::DEBUG.load(std::sync::atomic::Ordering::Relaxed) {
            println!($($t)*);
        }
    };
}

mod config;
mod devices;
mod render;
mod startup;
mod state;
mod ui;
mod win;

pub const APP_NAME: &str = "Mouse Battery";

fn main() {
    if std::env::args().any(|a| a == "--debug") {
        win::attach_console();
        DEBUG.store(true, Ordering::Relaxed);
        debug_check();
        return;
    }
    if win::already_running() {
        win::message_box(
            APP_NAME,
            "Mouse Battery is already running.\n\nLook for the battery widget on the taskbar or the mouse icon in the tray.",
        );
        return;
    }
    win::set_dpi_aware();
    startup::migrate();
    ui::run();
}

fn debug_check() {
    println!("Looking for supported mice (move the mouse so it is awake)...");
    let mut api = match hidapi::HidApi::new() {
        Ok(api) => api,
        Err(e) => {
            println!("Could not start HID access: {e}");
            return;
        }
    };
    let readings = devices::read_all(&mut api);
    if readings.is_empty() {
        println!("No supported mouse receiver found.");
    }
    for r in readings {
        match r.level {
            Some(level) => println!(
                "{}: {}%{}",
                r.name,
                level,
                if r.charging { " (charging)" } else { "" }
            ),
            None => println!("{}: no answer", r.name),
        }
    }
}

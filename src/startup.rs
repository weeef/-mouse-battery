//! "Start with Windows" via HKCU\...\Run.

use crate::APP_NAME;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const LEGACY_NAMES: [&str; 1] = ["Beast Battery"]; // v1 name; replaced so only one copy starts

fn launch_command() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    format!("\"{}\"", exe.display())
}

fn run_key() -> std::io::Result<RegKey> {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_SET_VALUE)
}

pub fn enabled() -> bool {
    run_key().is_ok_and(|k| k.get_value::<String, _>(APP_NAME).is_ok())
}

pub fn toggle() {
    let Ok(key) = run_key() else { return };
    let result = if enabled() {
        key.delete_value(APP_NAME)
    } else {
        key.set_value(APP_NAME, &launch_command())
    };
    if let Err(e) = result {
        crate::dlog!("startup toggle failed: {e}");
    }
}

/// Replace an old "Beast Battery" startup entry with this app.
pub fn migrate() {
    let Ok(key) = run_key() else { return };
    let mut found = false;
    for name in LEGACY_NAMES {
        if key.get_value::<String, _>(name).is_ok() && key.delete_value(name).is_ok() {
            found = true;
        }
    }
    if found {
        let _ = key.set_value(APP_NAME, &launch_command());
    }
}

//! Settings stored in %APPDATA%\MouseBattery\settings.json.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// Distance from the right edge of the taskbar to the widget's left edge, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_offset: Option<i32>,
    /// Whether the taskbar widget is shown.
    #[serde(default = "yes")]
    pub visible: bool,
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Config { right_offset: None, visible: true }
    }
}

fn path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("MouseBattery").join("settings.json")
}

impl Config {
    pub fn load() -> Config {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let p = path();
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string_pretty(self) {
            if let Err(e) = std::fs::write(&p, s) {
                crate::dlog!("config save failed: {e}");
            }
        }
    }
}

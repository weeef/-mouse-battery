//! Battery state per mouse, with low-battery and fully-charged alerts.

use crate::devices::Reading;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const POLL: Duration = Duration::from_secs(60);
pub const LOW_BATTERY: u8 = 15;
const STALE: Duration = Duration::from_secs(30 * 60);
/// Some mice (the WLmouse Beast X family) report 100% as soon as the cable goes in, so
/// while charging the level is estimated from the last unplugged reading at this rate.
const FULL_CHARGE_MINUTES: f64 = 90.0;

/// What is remembered across restarts, so a charge in progress keeps its estimate.
#[derive(Serialize, Deserialize, Clone, Copy, Default, Debug, PartialEq)]
struct Saved {
    /// Last level reported while not charging.
    #[serde(default)]
    unplugged: Option<u8>,
    /// Level the current charge started from, and when (unix seconds).
    #[serde(default)]
    charge: Option<(u8, u64)>,
}

fn saved_path() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("MouseBattery").join("battery.json")
}

fn load_saved(key: &str) -> Saved {
    std::fs::read_to_string(saved_path())
        .ok()
        .and_then(|s| serde_json::from_str::<HashMap<String, Saved>>(&s).ok())
        .and_then(|m| m.get(key).copied())
        .unwrap_or_default()
}

fn store_saved(key: &str, saved: Saved) {
    let p = saved_path();
    let mut all: HashMap<String, Saved> = std::fs::read_to_string(&p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    all.insert(key.to_string(), saved);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(s) = serde_json::to_string_pretty(&all) {
        let _ = std::fs::write(&p, s);
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Estimated level `secs` seconds into a charge that started at `base`.
pub fn estimate(base: u8, secs: u64) -> u8 {
    let gained = secs as f64 / 60.0 * 100.0 / FULL_CHARGE_MINUTES;
    (base as f64 + gained).min(100.0) as u8
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub level: Option<u8>,
    pub charging: bool,
    pub asleep: bool,
}

pub struct Mouse {
    pub name: String,
    key: &'static str,
    saved: Saved,
    last: Option<(u8, bool, Instant)>,
    warned_low: bool,
    warned_full: bool,
}

impl Mouse {
    fn new(key: &'static str, name: &str) -> Mouse {
        Mouse {
            name: name.to_string(),
            key,
            saved: load_saved(key),
            last: None,
            warned_low: false,
            warned_full: false,
        }
    }

    fn set_saved(&mut self, saved: Saved) {
        if saved != self.saved {
            self.saved = saved;
            store_saved(self.key, saved);
        }
    }

    /// Turns the raw reading into the level to show, tracking charge sessions.
    fn charge_level(&mut self, reported: Option<u8>, charging: bool) -> Option<u8> {
        let mut saved = self.saved;
        let shown = if charging {
            if saved.charge.is_none() {
                // no unplugged reading to start from yet: trust the mouse
                let Some(base) = saved.unplugged.or(reported) else { return reported };
                saved.charge = Some((base.min(reported.unwrap_or(100)), now_secs()));
            }
            let (base, start) = saved.charge.unwrap();
            let est = estimate(base, now_secs().saturating_sub(start));
            Some(reported.map_or(est, |r| r.min(est)))
        } else {
            if let Some((base, start)) = saved.charge.take() {
                // cable came out: carry the estimate over until the mouse reports again
                saved.unplugged = Some(estimate(base, now_secs().saturating_sub(start)));
            }
            if reported.is_some() {
                saved.unplugged = reported;
            }
            reported
        };
        self.set_saved(saved);
        shown
    }

    fn update(&mut self, r: &Reading, notes: &mut Vec<String>) {
        if !r.name.is_empty() {
            self.name = r.name.clone();
        }
        let level = self.charge_level(r.level, r.charging);
        let Some(level) = level.filter(|_| r.level.is_some() || r.charging) else {
            if let Some((_, _, t)) = self.last {
                if t.elapsed() >= STALE {
                    self.last = None;
                }
            }
            return;
        };
        self.last = Some((level, r.charging, Instant::now()));
        if level <= LOW_BATTERY && !r.charging && !self.warned_low {
            notes.push(format!("{} battery is at {}%. Time to charge.", self.name, level));
            self.warned_low = true;
        }
        if r.charging || level > LOW_BATTERY + 5 {
            self.warned_low = false;
        }
        if r.charging && level >= 100 && !self.warned_full {
            notes.push(format!("{} is fully charged.", self.name));
            self.warned_full = true;
        }
        if !r.charging {
            self.warned_full = false;
        }
    }

    fn asleep(&self) -> bool {
        self.last.is_some_and(|(_, _, t)| t.elapsed() > POLL * 3 / 2)
    }

    pub fn entry(&self) -> Option<Entry> {
        self.last.map(|(level, charging, _)| Entry { level: Some(level), charging, asleep: self.asleep() })
    }

    pub fn text(&self) -> String {
        match self.last {
            None => format!("{}: waiting for the mouse (move it to wake it)", self.name),
            Some((level, charging, _)) => {
                let state = if charging && level < 100 {
                    " - charging (estimated)"
                } else if charging {
                    " - charging"
                } else if self.asleep() {
                    " (asleep, last reading)"
                } else {
                    ""
                };
                format!("{}: {}%{}", self.name, level, state)
            }
        }
    }
}

#[derive(Default)]
pub struct AppState {
    pub mice: Vec<(&'static str, Mouse)>,
    /// Notifications waiting to be shown by the UI thread.
    pub notes: Vec<String>,
}

impl AppState {
    pub fn apply(&mut self, readings: Vec<Reading>) {
        // a receiver that was unplugged disappears right away
        self.mice.retain(|(key, _)| readings.iter().any(|r| r.key == *key));
        for r in &readings {
            let i = match self.mice.iter().position(|(k, _)| *k == r.key) {
                Some(i) => i,
                None => {
                    self.mice.push((r.key, Mouse::new(r.key, &r.name)));
                    self.mice.len() - 1
                }
            };
            let (_, mouse) = &mut self.mice[i];
            mouse.update(r, &mut self.notes);
        }
    }

    pub fn entries(&self) -> Vec<Entry> {
        self.mice.iter().filter_map(|(_, m)| m.entry()).collect()
    }

    pub fn status_lines(&self) -> Vec<String> {
        if self.mice.is_empty() {
            return vec!["No supported mouse found".into()];
        }
        self.mice.iter().map(|(_, m)| m.text()).collect()
    }

    /// Tooltip text and the entry for the tray icon (the emptiest mouse).
    pub fn tray(&self) -> (String, Option<Entry>) {
        let shown: Vec<&Mouse> = self.mice.iter().map(|(_, m)| m).filter(|m| m.last.is_some()).collect();
        if shown.is_empty() {
            return (format!("{}: no mouse found", crate::APP_NAME), None);
        }
        let lowest = shown.iter().min_by_key(|m| m.last.unwrap().0).unwrap();
        let tip = shown.iter().map(|m| m.text()).collect::<Vec<_>>().join("\n");
        (tip, lowest.entry())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charge_estimate() {
        assert_eq!(estimate(60, 0), 60);
        assert_eq!(estimate(60, 5 * 60), 65);
        assert_eq!(estimate(60, 45 * 60), 100);
        assert_eq!(estimate(0, 45 * 60), 50);
    }
}

//! Battery state per mouse, with low-battery and fully-charged alerts.

use crate::devices::Reading;
use std::time::{Duration, Instant};

pub const POLL: Duration = Duration::from_secs(60);
pub const LOW_BATTERY: u8 = 15;
const STALE: Duration = Duration::from_secs(30 * 60);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry {
    pub level: Option<u8>,
    pub charging: bool,
    pub asleep: bool,
}

pub struct Mouse {
    pub name: String,
    last: Option<(u8, bool, Instant)>,
    warned_low: bool,
    warned_full: bool,
}

impl Mouse {
    fn new(name: &str) -> Mouse {
        Mouse { name: name.to_string(), last: None, warned_low: false, warned_full: false }
    }

    fn update(&mut self, r: &Reading, notes: &mut Vec<String>) {
        if !r.name.is_empty() {
            self.name = r.name.clone();
        }
        let Some(level) = r.level else {
            match self.last {
                Some((lvl, _, _)) if r.charging => self.last = Some((lvl, true, Instant::now())), // quiet on the cable
                Some((_, _, t)) if t.elapsed() >= STALE => self.last = None,
                _ => {}
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
                let state = if charging {
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
                    self.mice.push((r.key, Mouse::new(&r.name)));
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

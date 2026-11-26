//! Battery readers for the supported mice. Each reader returns one `Reading` per mouse
//! that is plugged in, with `level: None` when the mouse is present but silent (asleep).
//!
//! WLmouse (Beast X family), VID 0x36A7 - community reverse-engineered, see HaloBattery (MIT):
//!   * request: feature report id 0, payload 00 00 02 02 00 83 (padded to 64 bytes)
//!   * reply:   a1|a2 00 02 02 00 83 <charging> <battery%>
//!   * fallback: the mouse sometimes sends an input report 03 00 <battery%> <charging>
//!
//! Attack Shark X3, VID 0x1D57 - community reverse-engineered (x3ctl, attack-shark-mac-battery):
//!   * the 2.4G dongle (PID 0xFA60) pushes an input report every few seconds while the mouse
//!     is awake, no request needed:  03 xx 40 01 <level>   (level 1-10 = 10%-100%)
//!   * with the USB-C cable plugged in the mouse also shows up as PID 0xFA61 -> charging
//!
//! Pulsar X2H / X2 V2 / X2A, VID 0x3554 - from the Linux hid-pulsar driver:
//!   * vendor collection FF02:0002 on interface 1, 17-byte output/input reports, id 0x08
//!   * request: 08 <cmd> 00.. <checksum>, checksum = 0x55 - sum(bytes 0..15)
//!   * 0x01 info   -> bytes 6-7 model (0x060C/0x060D = X2H); answered even with the mouse off
//!   * 0x03 status -> byte 6: 1 = mouse connected, 0 = off / out of range
//!   * 0x04 power  -> byte 6 battery %, byte 7 = 1 on the cable
//!   * PIDs: 0xF508 1 kHz dongle, 0xF509 4 kHz dongle, 0xF507 cable

use hidapi::HidApi;
use std::ffi::CString;
use std::thread::sleep;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Reading {
    /// Stable id for the mouse ("wlmouse", "attackshark").
    pub key: &'static str,
    pub name: String,
    /// Battery %, None when the mouse did not answer.
    pub level: Option<u8>,
    pub charging: bool,
}

struct Info {
    pid: u16,
    usage_page: u16,
    usage: u16,
    path: CString,
    product: String,
}

fn hex(data: &[u8]) -> String {
    data.iter().take(16).map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

fn enumerate(api: &HidApi, vid: u16) -> Vec<Info> {
    let infos: Vec<Info> = api
        .device_list()
        .filter(|d| d.vendor_id() == vid)
        .map(|d| Info {
            pid: d.product_id(),
            usage_page: d.usage_page(),
            usage: d.usage(),
            path: d.path().to_owned(),
            product: d.product_string().unwrap_or("").trim().to_string(),
        })
        .collect();
    for d in &infos {
        dlog!(
            "  found {vid:04x}:{:04x} usage={:04x}:{:04x} '{}'",
            d.pid, d.usage_page, d.usage, d.product
        );
    }
    infos
}

/// Open several HID collections at once and wait for an input report that `parse` accepts.
fn listen<T>(api: &HidApi, paths: &[&CString], seconds: f32, parse: fn(&[u8]) -> Option<T>) -> Option<T> {
    let mut devs = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        if paths[..i].contains(path) {
            continue;
        }
        match api.open_path(path) {
            Ok(dev) => {
                let _ = dev.set_blocking_mode(false);
                devs.push(dev);
            }
            Err(e) => dlog!("   open failed: {e}"),
        }
    }
    let end = Instant::now() + Duration::from_secs_f32(seconds);
    let mut buf = [0u8; 64];
    while !devs.is_empty() && Instant::now() < end {
        for dev in &devs {
            if let Ok(n) = dev.read(&mut buf) {
                if n > 0 {
                    dlog!("   report: {}", hex(&buf[..n]));
                    if let Some(result) = parse(&buf[..n]) {
                        return Some(result);
                    }
                }
            }
        }
        sleep(Duration::from_millis(20));
    }
    None
}

// ------------------------------------------------------------------ WLmouse (Beast X family)

const WLMOUSE_VID: u16 = 0x36A7;
const WLMOUSE_QUERY: [u8; 6] = [0x00, 0x00, 0x02, 0x02, 0x00, 0x83];

fn wl_known_name(pid: u16) -> Option<&'static str> {
    match pid {
        0xA868 => Some("Beast X Mini Pro"),
        0xA887 => Some("Beast X"),
        0xA880 => Some("Beast X Max"),
        _ => None,
    }
}

pub fn wl_parse_feature(r: &[u8]) -> Option<(u8, bool)> {
    for i in 5..r.len().saturating_sub(2) {
        if r[i] == 0x83 && r[i - 1] == 0x00 && r[i - 2] == 0x02 && matches!(r[i - 5], 0xA1 | 0xA2) {
            let (charging, level) = (r[i + 1], r[i + 2]);
            if level <= 100 {
                return Some((level, charging != 0));
            }
        }
    }
    None
}

pub fn wl_parse_heartbeat(d: &[u8]) -> Option<(u8, bool)> {
    for off in 0..2 {
        if d.len() >= off + 4 && d[off] == 0x03 && d[off + 1] == 0x00 && d[off + 2] <= 100 {
            return Some((d[off + 2], d[off + 3] != 0));
        }
    }
    None
}

fn wl_query(api: &HidApi, path: &CString) -> Option<(u8, bool)> {
    let dev = match api.open_path(path) {
        Ok(dev) => dev,
        Err(e) => {
            dlog!("   open failed: {e}");
            return None;
        }
    };
    let mut req = [0u8; 65]; // report id 0 + 64 bytes
    req[1..1 + WLMOUSE_QUERY.len()].copy_from_slice(&WLMOUSE_QUERY);
    if let Err(e) = dev.send_feature_report(&req) {
        dlog!("   send failed: {e}");
    }
    for _ in 0..15 {
        sleep(Duration::from_millis(50));
        for len in [65usize, 64] {
            let mut buf = vec![0u8; len];
            if let Ok(n) = dev.get_feature_report(&mut buf) {
                if let Some(result) = wl_parse_feature(&buf[..n]) {
                    dlog!("   reply: {}", hex(&buf[..n.min(12)]));
                    return Some(result);
                }
            }
        }
    }
    None
}

fn wl_name(pid: u16, product: &str) -> String {
    if let Some(name) = wl_known_name(pid) {
        return name.to_string();
    }
    let mut name = product.to_string();
    for junk in ["WLMOUSE", " 8K", " 4K", " 1K", " Receiver", " Dongle", " Wireless"] {
        if let Some(i) = name.to_lowercase().find(&junk.to_lowercase()) {
            name.replace_range(i..i + junk.len(), "");
        }
    }
    let name = name.trim();
    if name.is_empty() { "Beast X".to_string() } else { name.to_string() }
}

fn read_wlmouse(api: &HidApi) -> Vec<Reading> {
    let infos = enumerate(api, WLMOUSE_VID);
    if infos.is_empty() {
        return vec![];
    }
    let mut pids: Vec<u16> = Vec::new();
    for d in &infos {
        if !pids.contains(&d.pid) {
            pids.push(d.pid);
        }
    }

    // a mouse on its cable appears with its own PID next to the receiver;
    // prefer the reading that says charging
    let mut best: Option<(u8, bool, String)> = None;
    let mut fallback_name = None;
    for pid in pids {
        let ifaces: Vec<&Info> = infos.iter().filter(|d| d.pid == pid).collect();
        let name = wl_name(pid, &ifaces[0].product);
        fallback_name.get_or_insert_with(|| name.clone());

        let mut vendor: Vec<&Info> = ifaces.iter().copied().filter(|d| d.usage_page == 0xFFFF).collect();
        vendor.sort_by_key(|d| if d.usage == 0 { 0 } else { 1 });
        if vendor.is_empty() {
            vendor = ifaces.iter().copied().filter(|d| d.usage_page >= 0xFF00).collect();
        }
        let mut result = None;
        for d in &vendor {
            dlog!("  asking {pid:04x} usage={:04x}:{:04x}", d.usage_page, d.usage);
            result = wl_query(api, &d.path);
            if result.is_some() {
                break;
            }
        }
        if result.is_none() && !vendor.is_empty() {
            let paths: Vec<&CString> = vendor.iter().take(2).map(|d| &d.path).collect();
            result = listen(api, &paths, 1.0, wl_parse_heartbeat);
        }
        if let Some((level, charging)) = result {
            if best.as_ref().map_or(true, |b| charging && !b.1) {
                best = Some((level, charging, name));
            }
        }
    }
    match best {
        Some((level, charging, name)) => vec![Reading { key: "wlmouse", name, level: Some(level), charging }],
        None => vec![Reading {
            key: "wlmouse",
            name: fallback_name.unwrap_or_else(|| "Beast X".into()),
            level: None,
            charging: false,
        }],
    }
}

// ------------------------------------------------------------------ Attack Shark X3

const SHARK_VID: u16 = 0x1D57;
const SHARK_DONGLE_PIDS: [u16; 1] = [0xFA60];
const SHARK_WIRED_PIDS: [u16; 2] = [0xFA61, 0xFA50];
const SHARK_LISTEN_SECONDS: f32 = 6.0; // the dongle reports every ~2-4 s while the mouse is awake

pub fn shark_parse(d: &[u8]) -> Option<u8> {
    if d.len() >= 5 && d[0] == 0x03 && d[2] == 0x40 && d[3] == 0x01 {
        let raw = d[4];
        return match raw {
            1..=10 => Some(raw * 10), // 1-10 scale, 10 = full
            0 | 11..=100 => Some(raw),
            _ => None,
        };
    }
    None
}

fn shark_priority(d: &Info) -> u8 {
    match (d.usage_page, d.usage) {
        (0x000A, _) => 0,
        (0x0001, 0x0080) => 1,
        (p, _) if p >= 0xFF00 => 2,
        _ => 3,
    }
}

fn read_attack_shark(api: &HidApi) -> Vec<Reading> {
    let infos: Vec<Info> = enumerate(api, SHARK_VID)
        .into_iter()
        .filter(|d| SHARK_DONGLE_PIDS.contains(&d.pid) || SHARK_WIRED_PIDS.contains(&d.pid))
        .collect();
    if infos.is_empty() {
        return vec![];
    }
    let wired = infos.iter().any(|d| SHARK_WIRED_PIDS.contains(&d.pid));
    // Windows keeps the plain mouse/keyboard collections to itself, so skip those
    let mut usable: Vec<&Info> = infos
        .iter()
        .filter(|d| !(d.usage_page == 0x0001 && matches!(d.usage, 0x0002 | 0x0006)))
        .collect();
    usable.sort_by_key(|d| shark_priority(d));
    dlog!("  listening for {SHARK_LISTEN_SECONDS}s on {} collection(s)...", usable.len());
    let paths: Vec<&CString> = usable.iter().map(|d| &d.path).collect();
    let level = listen(api, &paths, SHARK_LISTEN_SECONDS, shark_parse);
    vec![Reading { key: "attackshark", name: "Attack Shark X3".into(), level, charging: wired }]
}

// ------------------------------------------------------------------ Pulsar (X2H, X2 V2, X2A, ...)

const PULSAR_VID: u16 = 0x3554;
const PULSAR_DONGLE_PIDS: [u16; 2] = [0xF508, 0xF509]; // 1 kHz, 4 kHz
const PULSAR_WIRED_PIDS: [u16; 1] = [0xF507];
const PULSAR_INFO: u8 = 0x01;
const PULSAR_STATUS: u8 = 0x03;
const PULSAR_POWER: u8 = 0x04;

fn pulsar_checksum(r: &[u8]) -> u8 {
    0x55u8.wrapping_sub(r[..16].iter().fold(0u8, |s, b| s.wrapping_add(*b)))
}

pub fn pulsar_request(cmd: u8) -> [u8; 17] {
    let mut r = [0u8; 17];
    r[0] = 0x08;
    r[1] = cmd;
    if cmd == PULSAR_INFO {
        r[5] = 8; // all-zero challenge, so the reply carries the device id as is
    }
    r[16] = pulsar_checksum(&r);
    r
}

fn pulsar_valid(r: &[u8], cmd: u8) -> bool {
    r.len() >= 17 && r[0] == 0x08 && r[1] == cmd && r[16] == pulsar_checksum(r)
}

/// Power reply: byte 6 = battery %, byte 7 = 1 while on the cable.
pub fn pulsar_parse_power(r: &[u8]) -> Option<(u8, bool)> {
    if pulsar_valid(r, PULSAR_POWER) && r[6] <= 100 && r[7] <= 1 {
        return Some((r[6], r[7] == 1));
    }
    None
}

pub fn pulsar_name(info: &[u8]) -> &'static str {
    if !pulsar_valid(info, PULSAR_INFO) {
        return "Pulsar";
    }
    match u16::from_be_bytes([info[6], info[7]]) {
        0x060C | 0x060D => "Pulsar X2H",
        _ => "Pulsar",
    }
}

/// Send one command and wait for its reply (the receiver answers within ~100 ms).
fn pulsar_cmd(dev: &hidapi::HidDevice, cmd: u8) -> Option<[u8; 17]> {
    let mut buf = [0u8; 64];
    while matches!(dev.read(&mut buf), Ok(n) if n > 0) {} // drop stale reports
    if let Err(e) = dev.write(&pulsar_request(cmd)) {
        dlog!("   write failed: {e}");
        return None;
    }
    let end = Instant::now() + Duration::from_millis(300);
    while Instant::now() < end {
        if let Ok(n) = dev.read(&mut buf) {
            if n > 0 {
                dlog!("   report: {}", hex(&buf[..n]));
                if pulsar_valid(&buf[..n], cmd) {
                    let mut r = [0u8; 17];
                    r.copy_from_slice(&buf[..17]);
                    return Some(r);
                }
            }
        }
        sleep(Duration::from_millis(10));
    }
    None
}

fn read_pulsar(api: &HidApi) -> Vec<Reading> {
    let infos: Vec<Info> = enumerate(api, PULSAR_VID)
        .into_iter()
        .filter(|d| PULSAR_DONGLE_PIDS.contains(&d.pid) || PULSAR_WIRED_PIDS.contains(&d.pid))
        .filter(|d| d.usage_page == 0xFF02 && d.usage == 0x0002)
        .collect();
    if infos.is_empty() {
        return vec![];
    }
    // the cable connection first: when it is there the mouse is charging on it
    let mut order: Vec<&Info> = infos.iter().collect();
    order.sort_by_key(|d| if PULSAR_WIRED_PIDS.contains(&d.pid) { 0 } else { 1 });

    let mut name = "Pulsar";
    for d in order {
        dlog!("  asking {:04x} usage={:04x}:{:04x}", d.pid, d.usage_page, d.usage);
        let dev = match api.open_path(&d.path) {
            Ok(dev) => dev,
            Err(e) => {
                dlog!("   open failed: {e}");
                continue;
            }
        };
        let _ = dev.set_blocking_mode(false);
        if let Some(info) = pulsar_cmd(&dev, PULSAR_INFO) {
            name = pulsar_name(&info);
        }
        // status byte 6: 1 = mouse connected to the receiver, 0 = off / out of range
        match pulsar_cmd(&dev, PULSAR_STATUS) {
            Some(s) if s[6] == 1 => {}
            Some(_) => {
                dlog!("   mouse not connected");
                continue;
            }
            None => continue,
        }
        if let Some((level, cable)) = pulsar_cmd(&dev, PULSAR_POWER).and_then(|r| pulsar_parse_power(&r)) {
            let charging = cable || PULSAR_WIRED_PIDS.contains(&d.pid);
            return vec![Reading { key: "pulsar", name: name.into(), level: Some(level), charging }];
        }
    }
    vec![Reading { key: "pulsar", name: name.into(), level: None, charging: false }]
}

// ------------------------------------------------------------------ all mice

pub fn read_all(api: &mut HidApi) -> Vec<Reading> {
    if let Err(e) = api.refresh_devices() {
        dlog!("refresh failed: {e}");
    }
    let mut readings = read_wlmouse(api);
    readings.extend(read_attack_shark(api));
    readings.extend(read_pulsar(api));
    readings
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shark() {
        assert_eq!(shark_parse(&[0x03, 0x10, 0x40, 0x01, 0x07]), Some(70));
        assert_eq!(shark_parse(&[0x03, 0x4D, 0x40, 0x01, 0x0A]), Some(100));
        assert_eq!(shark_parse(&[0x03, 0x10, 0x40, 0x01, 55]), Some(55));
        assert_eq!(shark_parse(&[0x03, 0x10, 0x60, 0x01, 0x07]), None);
        assert_eq!(shark_parse(&[0x01, 0x00]), None);
    }

    #[test]
    fn wlmouse() {
        assert_eq!(wl_parse_feature(&[0x00, 0xA1, 0, 2, 2, 0, 0x83, 0, 0x49, 0]), Some((73, false)));
        assert_eq!(wl_parse_feature(&[0xA1, 0, 2, 2, 0, 0x83, 1, 93]), Some((93, true)));
        assert_eq!(wl_parse_heartbeat(&[0x03, 0x00, 80, 0]), Some((80, false)));
        assert_eq!(wl_name(0x1234, "WLMOUSE Foo 8K Receiver"), "Foo");
    }

    #[test]
    fn pulsar() {
        assert_eq!(pulsar_request(PULSAR_POWER)[16], 0x49);
        assert_eq!(pulsar_request(PULSAR_STATUS)[16], 0x4A);
        assert_eq!(pulsar_request(PULSAR_INFO)[16], 0x44);
        // captured from an X2H receiver with the mouse switched off
        let info = [8, 1, 0, 0, 0, 8, 6, 0x0c, 0, 0, 6, 0x0c, 0, 0, 0, 0, 0x20];
        assert_eq!(pulsar_name(&info), "Pulsar X2H");
        let mut power = [8u8, 4, 0, 0, 0, 2, 73, 0, 0x0f, 0xa0, 0, 0, 0, 0, 0, 0, 0];
        power[16] = pulsar_checksum(&power);
        assert_eq!(pulsar_parse_power(&power), Some((73, false)));
        power[7] = 1;
        power[16] = pulsar_checksum(&power);
        assert_eq!(pulsar_parse_power(&power), Some((73, true)));
        power[16] ^= 1;
        assert_eq!(pulsar_parse_power(&power), None);
    }
}

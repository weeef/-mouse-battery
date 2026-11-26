//! Self-update from the project's GitHub Releases.
//!
//! The newest release must have `MouseBattery.exe` attached and a tag like `v2.1.0`.
//! Installing downloads it next to the running exe, renames the running exe to
//! `MouseBattery.exe.old` (Windows allows renaming a running exe), moves the new one into
//! place and starts it with `--updated`; the new copy deletes the `.old` file.

use crate::win::wide;
use serde::Deserialize;
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::time::Duration;
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Networking::WinHttp::*;

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "weeef/-mouse-battery";
const ASSET: &str = "MouseBattery.exe";
const MAX_EXE_BYTES: usize = 32 << 20;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: String,
    url: String,
}

// ------------------------------------------------------------------ checking

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

fn version_parts(v: &str) -> Vec<u32> {
    v.trim().trim_start_matches(['v', 'V']).split('.').map(|p| {
        p.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0)
    }).collect()
}

/// True when version `a` is newer than `b` ("v2.1.0" > "2.0.9", "2.1" == "2.1.0").
pub fn is_newer(a: &str, b: &str) -> bool {
    let (mut a, mut b) = (version_parts(a), version_parts(b));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

fn pick_release(json: &[u8]) -> Result<Option<Release>, String> {
    let r: GhRelease = serde_json::from_slice(json).map_err(|_| "unexpected reply from GitHub".to_string())?;
    if r.draft || r.prerelease || !is_newer(&r.tag_name, CURRENT) {
        return Ok(None);
    }
    let asset = r
        .assets
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(ASSET))
        .or_else(|| r.assets.iter().find(|a| a.name.to_ascii_lowercase().ends_with(".exe")))
        .ok_or_else(|| format!("release {} has no {ASSET} attached", r.tag_name))?;
    Ok(Some(Release {
        version: r.tag_name.trim_start_matches(['v', 'V']).to_string(),
        url: asset.browser_download_url.clone(),
    }))
}

/// The newest release if it is newer than this build.
pub fn check() -> Result<Option<Release>, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    match get(&url, 1 << 20, "Accept: application/vnd.github+json") {
        Ok(json) => pick_release(&json),
        Err(e) if e == "HTTP 404" => Ok(None), // no release published yet
        Err(e) => Err(e),
    }
}

// ------------------------------------------------------------------ installing

fn exe_paths() -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok((exe.clone(), exe.with_extension("exe.new"), exe.with_extension("exe.old")))
}

/// Download and swap in the new exe, then start it. The caller quits right after.
pub fn install(r: &Release) -> Result<(), String> {
    let (exe, new, old) = exe_paths()?;
    let data = get(&r.url, MAX_EXE_BYTES, "")?;
    if data.len() < 64 * 1024 || !data.starts_with(b"MZ") {
        return Err("the download is not a valid program".into());
    }
    std::fs::write(&new, &data).map_err(|e| format!("could not save the download: {e}"))?;
    let _ = std::fs::remove_file(&old);
    if let Err(e) = std::fs::rename(&exe, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(format!("could not replace the program file: {e}"));
    }
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("could not replace the program file: {e}"));
    }
    if let Err(e) = std::process::Command::new(&exe).arg("--updated").spawn() {
        // put the old version back so the app keeps working
        let _ = std::fs::remove_file(&exe);
        let _ = std::fs::rename(&old, &exe);
        return Err(format!("could not start the new version: {e}"));
    }
    Ok(())
}

/// Remove what an earlier update left behind (the old copy may take a moment to exit).
pub fn cleanup() {
    let Ok((_, new, old)) = exe_paths() else { return };
    let _ = std::fs::remove_file(&new);
    for _ in 0..20 {
        if !old.exists() || std::fs::remove_file(&old).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}

// ------------------------------------------------------------------ HTTPS GET via WinHTTP

const ACCESS_AUTOMATIC_PROXY: u32 = 4;
const FLAG_SECURE: u32 = 0x0080_0000;
const QUERY_STATUS_CODE: u32 = 19;
const QUERY_FLAG_NUMBER: u32 = 0x2000_0000;

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn last_error(what: &str) -> String {
    match unsafe { GetLastError() } {
        12002 => "the connection timed out".into(),
        12007 | 12029 => "no internet connection".into(),
        code => format!("{what} failed (error {code})"),
    }
}

/// Blocking HTTPS GET (follows redirects). `headers` is extra "Name: value" lines.
fn get(url: &str, limit: usize, headers: &str) -> Result<Vec<u8>, String> {
    let rest = url.strip_prefix("https://").ok_or("not an https address")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let agent = wide(&format!("MouseBattery/{CURRENT}"));
    let (host_w, path_w, verb) = (wide(host), wide(path), wide("GET"));
    let headers_w = wide(headers);
    unsafe {
        let session = Handle(WinHttpOpen(agent.as_ptr(), ACCESS_AUTOMATIC_PROXY as _, null(), null(), 0));
        if session.0.is_null() {
            return Err(last_error("starting the download"));
        }
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 30_000, 30_000);
        let conn = Handle(WinHttpConnect(session.0, host_w.as_ptr(), 443, 0));
        if conn.0.is_null() {
            return Err(last_error("connecting"));
        }
        let req = Handle(WinHttpOpenRequest(
            conn.0,
            verb.as_ptr(),
            path_w.as_ptr(),
            null(),
            null(),
            null(),
            FLAG_SECURE as _,
        ));
        if req.0.is_null() {
            return Err(last_error("connecting"));
        }
        let (hdr, hdr_len) = if headers.is_empty() { (null(), 0) } else { (headers_w.as_ptr(), u32::MAX) };
        if WinHttpSendRequest(req.0, hdr, hdr_len, null(), 0, 0, 0) == 0
            || WinHttpReceiveResponse(req.0, null_mut()) == 0
        {
            return Err(last_error("downloading"));
        }
        let mut status: u32 = 0;
        let mut len = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            req.0,
            QUERY_STATUS_CODE | QUERY_FLAG_NUMBER,
            null(),
            &mut status as *mut u32 as *mut c_void,
            &mut len,
            null_mut(),
        );
        if status != 200 {
            return Err(format!("HTTP {status}"));
        }
        let mut out = Vec::new();
        loop {
            let mut avail: u32 = 0;
            if WinHttpQueryDataAvailable(req.0, &mut avail) == 0 {
                return Err(last_error("downloading"));
            }
            if avail == 0 {
                break;
            }
            let start = out.len();
            out.resize(start + avail as usize, 0);
            let mut read: u32 = 0;
            if WinHttpReadData(req.0, out[start..].as_mut_ptr() as *mut c_void, avail, &mut read) == 0 {
                return Err(last_error("downloading"));
            }
            out.truncate(start + read as usize);
            if out.len() > limit {
                return Err("the download is too large".into());
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(is_newer("v2.1.0", "2.0.0"));
        assert!(is_newer("2.0.10", "2.0.9"));
        assert!(is_newer("3", "2.9.9"));
        assert!(!is_newer("v2.0.0", "2.0.0"));
        assert!(!is_newer("2.0", "2.0.0"));
        assert!(!is_newer("1.9.9", "2.0.0"));
    }

    #[test]
    fn release_json() {
        let json = br#"{"tag_name":"v99.0.0","draft":false,"prerelease":false,"assets":[
            {"name":"source.zip","browser_download_url":"https://x/source.zip"},
            {"name":"MouseBattery.exe","browser_download_url":"https://x/MouseBattery.exe"}]}"#;
        let r = pick_release(json).unwrap().unwrap();
        assert_eq!(r.version, "99.0.0");
        assert_eq!(r.url, "https://x/MouseBattery.exe");

        let old = br#"{"tag_name":"v0.1.0","assets":[]}"#;
        assert!(pick_release(old).unwrap().is_none());
        let no_exe = br#"{"tag_name":"v99.0.0","assets":[]}"#;
        assert!(pick_release(no_exe).is_err());
    }

    /// Talks to GitHub: `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn https_download() {
        let json = get("https://api.github.com/repos/libusb/hidapi/releases/latest", 1 << 20,
            "Accept: application/vnd.github+json").unwrap();
        let r: GhRelease = serde_json::from_slice(&json).unwrap();
        let asset = r.assets.iter().find(|a| a.name.ends_with(".zip")).unwrap();
        // release assets redirect to another host
        let zip = get(&asset.browser_download_url, MAX_EXE_BYTES, "").unwrap();
        assert!(zip.starts_with(b"PK"), "got {} bytes", zip.len());
        assert_eq!(get("https://api.github.com/repos/weeef/-mouse-battery/nope", 1 << 20, ""), Err("HTTP 404".into()));
        assert!(check().is_ok());
    }
}

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::download::{self, Download};
use crate::{proc, read_json, write_whole};

const URL: &str = "https://github.com/GyanD/codexffmpeg/releases/download/8.1/ffmpeg-8.1-essentials_build.zip";
const SHA256: &str = "8748283d821613d930b0e7be685aaa9df4ca6f0ad4d0c42fd02622b3623463c6";
const SIZE: u64 = 108_771_139;
const PROBE_SOURCE: &str = "testsrc2=size=1280x720:rate=30";
const PROBE_WAIT: Duration = Duration::from_secs(10);
const PROBES: &str = "encoders.json";
const RETRY_FIRST: Duration = Duration::from_secs(10);
const RETRY_MAX: Duration = Duration::from_secs(300);
const DROP_FOR: Duration = Duration::from_secs(600);
const RETRY_POLL: Duration = Duration::from_millis(500);
pub const FRAGMENTED: &str = "frag_keyframe+empty_moov+default_base_moof";
pub const FALLBACK: (&str, &str, f64) = ("libx264", "h264", 1.0);
pub const CANDIDATES: [(&str, &str, f64); 10] = [
    ("av1_nvenc", "av1", 0.55),
    ("av1_amf", "av1", 0.55),
    ("av1_qsv", "av1", 0.55),
    ("hevc_nvenc", "hevc", 0.7),
    ("hevc_amf", "hevc", 0.7),
    ("hevc_qsv", "hevc", 0.7),
    ("h264_nvenc", "h264", 1.0),
    ("h264_amf", "h264", 1.0),
    ("h264_qsv", "h264", 1.0),
    FALLBACK,
];

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
static RESULTS: OnceLock<Vec<(&'static str, bool)>> = OnceLock::new();
static DROPPED: Mutex<Vec<(&'static str, Instant)>> = Mutex::new(Vec::new());
static FAILED: Mutex<Option<String>> = Mutex::new(None);
static RETRY: AtomicBool = AtomicBool::new(false);

pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

pub fn path() -> PathBuf {
    let beside = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("ffmpeg.exe")));
    let downloaded = DATA_DIR.get().map(|dir| dir.join("ffmpeg.exe"));
    [beside, downloaded].into_iter().flatten().find(|p| p.exists()).unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

pub fn present() -> bool {
    path().is_absolute()
}

pub fn ready() -> Result<(), String> {
    if present() {
        Ok(())
    } else if FAILED.lock().unwrap().is_some() {
        Err("Couldn't download the video tools. Check your connection.".into())
    } else {
        Err("The video tool is still downloading. Wait a minute and try again.".into())
    }
}

pub fn failure() -> Option<String> {
    FAILED.lock().unwrap().clone()
}

fn needed() -> Download {
    Download { url: URL.into(), sha256: SHA256, size: SIZE, dest: String::new(), pick: &["ffmpeg.exe"] }
}

pub fn retry_after(failures: u32) -> Duration {
    RETRY_FIRST.saturating_mul(1 << failures.saturating_sub(1).min(5)).min(RETRY_MAX)
}

pub fn prepare(status: &Mutex<String>) {
    let mut failures = 0;
    while !present() {
        let Some(dir) = DATA_DIR.get() else { return };
        *status.lock().unwrap() = "downloading".into();
        tracing::info!("downloading ffmpeg");
        match download::fetch(dir, &needed()) {
            Ok(()) => *FAILED.lock().unwrap() = None,
            Err(e) => {
                failures += 1;
                let wait = retry_after(failures);
                tracing::warn!("ffmpeg download failed: {e}; retrying in {} s", wait.as_secs());
                *status.lock().unwrap() = "failed".into();
                *FAILED.lock().unwrap() = Some(e);
                let until = Instant::now() + wait;
                while Instant::now() < until && !RETRY.swap(false, Ordering::Relaxed) {
                    std::thread::sleep(RETRY_POLL);
                }
            }
        }
    }
    status.lock().unwrap().clear();
    probe_encoders();
}

pub fn retry() {
    RETRY.store(true, Ordering::Relaxed);
}

fn works(name: &str) -> bool {
    let mut command = Command::new(path());
    command.args(["-v", "quiet", "-f", "lavfi", "-i", PROBE_SOURCE, "-t", "1", "-c:v", name, "-f", "null", "-"]).stdout(Stdio::null());
    let ok = proc::run(&mut command, proc::BELOW_NORMAL, PROBE_WAIT);
    if let Err(e) = &ok {
        tracing::info!("encoder {name}: {e}");
    }
    ok.is_ok()
}

fn adapters() -> Vec<String> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIDevice, IDXGIFactory1};
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return Vec::new() };
    let mut out = Vec::new();
    let mut i = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(i) } {
        i += 1;
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else { continue };
        let name = String::from_utf16_lossy(&desc.Description).trim_end_matches('\0').to_string();
        let driver = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }.unwrap_or_default();
        out.push(format!("{name} {:04x}:{:04x} {driver:x}", desc.VendorId, desc.DeviceId));
    }
    out
}

fn probe_key() -> String {
    let file = std::fs::metadata(path()).ok();
    let size = file.as_ref().map_or(0, |m| m.len());
    let modified = file.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    format!("{} {size} {modified} | {}", path().display(), adapters().join(" | "))
}

pub fn cached(saved: &Value, key: &str) -> Option<Vec<(&'static str, bool)>> {
    if saved["key"] != key {
        return None;
    }
    CANDIDATES.iter().map(|(name, _, _)| saved["results"][name].as_bool().map(|ok| (*name, ok))).collect()
}

pub fn probe_encoders() {
    if !present() || RESULTS.get().is_some() {
        return;
    }
    let file = DATA_DIR.get().map(|dir| dir.join(PROBES));
    let key = probe_key();
    let saved: Value = file.as_deref().map(read_json).unwrap_or_default();
    let results = match cached(&saved, &key) {
        Some(results) => results,
        None => {
            let results: Vec<(&'static str, bool)> = CANDIDATES.iter().map(|(name, _, _)| (*name, works(name))).collect();
            let map: serde_json::Map<String, Value> = results.iter().map(|(n, ok)| (n.to_string(), json!(ok))).collect();
            if let Some(file) = &file {
                let _ = write_whole(file, json!({"key": key, "results": map}).to_string().as_bytes());
            }
            results
        }
    };
    let _ = RESULTS.set(results);
}

pub fn encoders_ready() -> bool {
    RESULTS.get().is_some()
}

pub fn encoder_results() -> Vec<(&'static str, bool)> {
    RESULTS.get().cloned().unwrap_or_default()
}

pub fn choose(results: &[(&str, bool)], usable: &[String], dropped: &[&str]) -> (&'static str, &'static str, f64) {
    CANDIDATES
        .iter()
        .find(|(name, codec, _)| usable.iter().any(|u| u == codec) && !dropped.contains(name) && results.iter().any(|(n, ok)| n == name && *ok))
        .copied()
        .unwrap_or(FALLBACK)
}

pub fn pick(usable: &[String]) -> (&'static str, &'static str, f64) {
    choose(RESULTS.get().map(Vec::as_slice).unwrap_or_default(), usable, &dropped())
}

pub fn drop_encoder(name: &'static str) -> bool {
    let mut dropped = DROPPED.lock().unwrap();
    dropped.retain(|(_, at)| at.elapsed() < DROP_FOR);
    let fresh = name != FALLBACK.0 && !dropped.iter().any(|(n, _)| *n == name);
    if fresh {
        dropped.push((name, Instant::now()));
    }
    fresh
}

pub fn dropped() -> Vec<&'static str> {
    DROPPED.lock().unwrap().iter().filter(|(_, at)| at.elapsed() < DROP_FOR).map(|(n, _)| *n).collect()
}

pub fn encoder() -> &'static str {
    pick(&crate::default_codecs()).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_skips_dropped_encoders_down_to_the_fallback() {
        let results = [("av1_nvenc", true), ("hevc_nvenc", true), ("h264_nvenc", true)];
        let all = ["av1", "hevc", "h264"].map(String::from);
        assert_eq!(choose(&results, &all, &[]).0, "av1_nvenc");
        assert_eq!(choose(&results, &all, &["av1_nvenc"]).0, "hevc_nvenc");
        assert_eq!(choose(&results, &all, &["av1_nvenc", "hevc_nvenc"]).0, "h264_nvenc");
        assert_eq!(choose(&results, &all, &["av1_nvenc", "hevc_nvenc", "h264_nvenc"]), FALLBACK);
        assert_eq!(choose(&results, &["h264".to_string()], &[]).0, "h264_nvenc");
    }

    #[test]
    fn a_dropped_backend_falls_to_another_backend_of_the_same_codec() {
        let results = [("h264_nvenc", true), ("h264_amf", false), ("h264_qsv", true), ("libx264", true)];
        assert_eq!(choose(&results, &["h264".to_string()], &["h264_nvenc"]).0, "h264_qsv");
    }

    #[test]
    fn probe_results_are_reused_only_for_the_same_key() {
        let results: serde_json::Map<String, Value> = CANDIDATES.iter().map(|(n, _, _)| (n.to_string(), json!(*n == "libx264"))).collect();
        let saved = json!({"key": "a", "results": results});
        let got = cached(&saved, "a").unwrap();
        assert_eq!(got.len(), CANDIDATES.len());
        assert!(got.iter().all(|(n, ok)| *ok == (*n == "libx264")));
        assert!(cached(&saved, "b").is_none());
        assert!(cached(&json!({"key": "a", "results": {}}), "a").is_none());
    }

    #[test]
    fn download_retries_back_off_up_to_a_limit() {
        assert_eq!(retry_after(1), RETRY_FIRST);
        assert_eq!(retry_after(2), RETRY_FIRST * 2);
        assert_eq!(retry_after(20), RETRY_MAX);
    }
}

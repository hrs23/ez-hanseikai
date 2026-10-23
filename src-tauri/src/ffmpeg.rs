use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::record::FPS;
use crate::{proc, read_json, write_whole};

const URL: &str = "https://github.com/GyanD/codexffmpeg/releases/download/8.1/ffmpeg-8.1-essentials_build.zip";
const SHA256: &str = "8748283d821613d930b0e7be685aaa9df4ca6f0ad4d0c42fd02622b3623463c6";
const SIZE: u64 = 108_771_139;
const DOWNLOAD_WAIT: Duration = Duration::from_secs(6 * 3600);
const WATCH_EVERY: Duration = Duration::from_millis(500);
const DOWNLOAD: &str = r#"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'
trap { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor 3072
$part = $env:EZH_PART
try {
  $request = [Net.HttpWebRequest]::Create($env:EZH_URL)
  $request.Timeout = 60000
  $request.ReadWriteTimeout = 60000
  $response = $request.GetResponse()
  $body = $response.GetResponseStream()
  $file = [IO.File]::Create($part)
  try { $body.CopyTo($file) } finally { $file.Dispose(); $body.Dispose(); $response.Dispose() }
  if ((Get-Item -LiteralPath $part).Length -ne [int64]$env:EZH_SIZE) { throw 'The download has the wrong size.' }
  if ((Get-FileHash -LiteralPath $part -Algorithm SHA256).Hash.ToLower() -ne $env:EZH_SHA) { throw 'The download has the wrong checksum.' }
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $zip = [IO.Compression.ZipFile]::OpenRead($part)
  try {
    $entry = @($zip.Entries | Where-Object { $_.Name -eq 'ffmpeg.exe' })[0]
    if (-not $entry) { throw 'ffmpeg.exe is missing from the download.' }
    $target = $env:EZH_OUT
    [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, "$target.part", $true)
    Move-Item -Force -LiteralPath "$target.part" -Destination $target
  } finally { $zip.Dispose() }
} finally { if (Test-Path -LiteralPath $part) { Remove-Item -LiteralPath $part } }
exit 0
"#;
const PROBE_SOURCE: &str = "testsrc2=size=1280x720:rate=30";
const PROBE_WAIT: Duration = Duration::from_secs(10);
const PROBES: &str = "encoders.json";
const RETRY_FIRST: Duration = Duration::from_secs(10);
const RETRY_MAX: Duration = Duration::from_secs(300);
const DROP_FOR: Duration = Duration::from_secs(600);
const RETRY_POLL: Duration = Duration::from_millis(500);
const KEYFRAME_EVERY: u32 = 2 * FPS;
const PICTURE_SIDE: u32 = 256;
const PICTURE_WAIT: Duration = Duration::from_secs(30);
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
static DOWNLOADING: Mutex<Option<(u64, Instant)>> = Mutex::new(None);
static PICTURES: AtomicU64 = AtomicU64::new(0);

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

pub fn retry_after(failures: u32) -> Duration {
    RETRY_FIRST.saturating_mul(1 << failures.saturating_sub(1).min(5)).min(RETRY_MAX)
}

pub fn prepare(status: &Mutex<String>) {
    let mut failures = 0;
    while !present() {
        let Some(dir) = DATA_DIR.get() else { return };
        *status.lock().unwrap() = "downloading".into();
        tracing::info!("downloading ffmpeg");
        match download(dir) {
            Ok(()) => {
                tracing::info!("ffmpeg downloaded");
                *FAILED.lock().unwrap() = None;
            }
            Err(e) => {
                failures += 1;
                let wait = retry_after(failures);
                tracing::warn!("ffmpeg download failed: {e}; retrying in {} s", wait.as_secs());
                *status.lock().unwrap() = "failed".into();
                *FAILED.lock().unwrap() = Some(e);
                proc::wait_until(wait, RETRY_POLL, || RETRY.swap(false, Ordering::Relaxed));
            }
        }
    }
    status.lock().unwrap().clear();
    probe_encoders();
}

pub fn command(level: &str) -> Command {
    let mut command = Command::new(path());
    command.args(["-hide_banner", "-loglevel", level, "-y"]);
    command
}

fn picture_args(grid: bool) -> Vec<String> {
    let filter = format!("crop='min(iw,ih)':'min(iw,ih)',scale={PICTURE_SIDE}:{PICTURE_SIDE}");
    let pick = if grid { vec!["-filter_complex".into(), format!("[0:g:0]{filter}[out]"), "-map".into(), "[out]".into()] } else { vec!["-vf".into(), filter] };
    [pick, ["-frames:v", "1", "-c:v", "png"].map(String::from).to_vec()].concat()
}

pub fn convert_picture(bytes: &[u8]) -> Result<Vec<u8>, String> {
    ready()?;
    let dir = std::env::temp_dir().join(format!("ezh-picture-{}-{}", std::process::id(), PICTURES.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let (input, output) = (dir.join("in"), dir.join("out.png"));
    let made = std::fs::write(&input, bytes).map_err(|e| e.to_string()).and_then(|_| {
        [false, true]
            .into_iter()
            .find_map(|grid| {
                let mut command = command("error");
                command.arg("-i").arg(&input).args(picture_args(grid)).arg(&output);
                match proc::run(&mut command, proc::BELOW_NORMAL, PICTURE_WAIT) {
                    Ok(_) => std::fs::read(&output).ok(),
                    Err(e) => {
                        tracing::debug!("ffmpeg could not open the picture (grid {grid}): {e}");
                        None
                    }
                }
            })
            .ok_or_else(|| "This file is not a picture the app can open.".to_string())
    });
    let _ = std::fs::remove_dir_all(&dir);
    made
}

pub fn pace(done: u64, total: u64, seconds: f64) -> (u64, Option<u64>) {
    if done == 0 || seconds <= 0.0 {
        return (0, None);
    }
    let speed = done as f64 / seconds;
    (speed as u64, Some((total.saturating_sub(done) as f64 / speed).ceil() as u64))
}

pub fn progress() -> Value {
    let Some((done, started)) = *DOWNLOADING.lock().unwrap() else { return Value::Null };
    let (speed, left) = pace(done, SIZE, started.elapsed().as_secs_f64());
    json!({"done": done, "total": SIZE, "speed": speed, "left_s": left})
}

pub fn download_host() -> Option<&'static str> {
    DOWNLOADING.lock().unwrap().and(URL.split('/').nth(2))
}

fn download(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let part = dir.join(format!("{}.part", URL.rsplit('/').next().unwrap_or_default()));
    *DOWNLOADING.lock().unwrap() = Some((0, Instant::now()));
    let stop = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (stop, part) = (stop.clone(), part.clone());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Some((done, _)) = DOWNLOADING.lock().unwrap().as_mut() {
                    *done = crate::file_len(&part);
                }
                std::thread::sleep(WATCH_EVERY);
            }
        })
    };
    let mut command = proc::powershell(&["-ExecutionPolicy", "Bypass", "-Command", DOWNLOAD]);
    command
        .env("EZH_URL", URL)
        .env("EZH_SHA", SHA256)
        .env("EZH_SIZE", SIZE.to_string())
        .env("EZH_PART", &part)
        .env("EZH_OUT", dir.join("ffmpeg.exe"));
    let result = proc::run(&mut command, 0, DOWNLOAD_WAIT).map(|_| ()).map_err(|e| e.lines().last().unwrap_or("download failed").trim().to_string());
    stop.store(true, Ordering::Relaxed);
    let _ = watcher.join();
    *DOWNLOADING.lock().unwrap() = None;
    result
}

pub fn retry() {
    RETRY.store(true, Ordering::Relaxed);
}

fn works(name: &str) -> bool {
    let mut command = Command::new(path());
    command.args(["-v", "quiet", "-f", "lavfi", "-i", PROBE_SOURCE, "-t", "1", "-c:v", name, "-f", "null", "-"]).stdout(Stdio::null());
    let ok = proc::run(&mut command, proc::BELOW_NORMAL, PROBE_WAIT);
    if let Err(e) = &ok {
        tracing::debug!("encoder {name} is not usable: {e}");
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
            if let Some(file) = &file {
                let _ = write_whole(file, json!({"key": key, "results": results.iter().copied().collect::<HashMap<_, _>>()}).to_string().as_bytes());
            }
            results
        }
    };
    let usable: Vec<&str> = results.iter().filter(|(_, ok)| *ok).map(|(name, _)| *name).collect();
    tracing::info!("usable encoders: {}", if usable.is_empty() { "none".to_string() } else { usable.join(", ") });
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

pub fn video_args(encoder: &str, kbps: u32) -> Vec<String> {
    let mut args = ["-c:v", encoder, "-b:v", &format!("{kbps}k"), "-g", &KEYFRAME_EVERY.to_string(), "-r", &FPS.to_string()].map(String::from).to_vec();
    if encoder == FALLBACK.0 {
        args.extend(["-preset", "veryfast"].map(String::from));
    }
    args
}

pub fn encoder() -> &'static str {
    pick(&crate::default_codecs()).0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_is_cut_to_a_centered_square_from_the_plain_stream_or_the_tile_grid() {
        let plain = picture_args(false);
        assert_eq!(plain[0], "-vf");
        assert_eq!(plain[1], format!("crop='min(iw,ih)':'min(iw,ih)',scale={PICTURE_SIDE}:{PICTURE_SIDE}"));
        let grid = picture_args(true);
        assert_eq!(grid[..4], ["-filter_complex".to_string(), format!("[0:g:0]{}[out]", plain[1]), "-map".into(), "[out]".into()]);
        for args in [plain, grid] {
            assert_eq!(args[args.len() - 4..], ["-frames:v", "1", "-c:v", "png"].map(String::from));
        }
    }

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
    fn pace_gives_speed_and_time_left() {
        assert_eq!(pace(0, 100, 5.0), (0, None));
        assert_eq!(pace(50, 100, 10.0), (5, Some(10)));
        assert_eq!(pace(100, 100, 10.0), (10, Some(0)));
        assert_eq!(pace(120, 100, 10.0).1, Some(0));
    }

    #[test]
    fn download_retries_back_off_up_to_a_limit() {
        assert_eq!(retry_after(1), RETRY_FIRST);
        assert_eq!(retry_after(2), RETRY_FIRST * 2);
        assert_eq!(retry_after(20), RETRY_MAX);
    }
}

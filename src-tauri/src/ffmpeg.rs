use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use crate::NO_WINDOW;

const URL: &str = "https://github.com/GyanD/codexffmpeg/releases/download/8.1/ffmpeg-8.1-essentials_build.zip";
const SHA256: &str = "8748283d821613d930b0e7be685aaa9df4ca6f0ad4d0c42fd02622b3623463c6";
const PROBE_SOURCE: &str = "testsrc2=size=1280x720:rate=30";
pub const FRAGMENTED: &str = "frag_keyframe+empty_moov+default_base_moof";
pub const FALLBACK: (&str, &str, f64) = ("libx264", "h264", 1.0);
const CANDIDATES: [(&str, &str, f64); 10] = [
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
    if present() { Ok(()) } else { Err("The video tool is still downloading. Wait a minute and try again.".into()) }
}

pub fn download() -> Result<(), String> {
    let dir = DATA_DIR.get().ok_or("no data folder")?;
    let script = format!(
        "$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'; \
         $zip = Join-Path '{dir}' 'ffmpeg.zip'; \
         Invoke-WebRequest -UseBasicParsing -Uri '{URL}' -OutFile $zip; \
         if ((Get-FileHash $zip -Algorithm SHA256).Hash.ToLower() -ne '{SHA256}') {{ Remove-Item $zip; throw 'checksum mismatch' }}; \
         Add-Type -AssemblyName System.IO.Compression.FileSystem; \
         $z = [IO.Compression.ZipFile]::OpenRead($zip); \
         $e = $z.Entries | Where-Object {{ $_.Name -eq 'ffmpeg.exe' }} | Select-Object -First 1; \
         [IO.Compression.ZipFileExtensions]::ExtractToFile($e, (Join-Path '{dir}' 'ffmpeg.exe.part'), $true); \
         $z.Dispose(); Remove-Item $zip; \
         Move-Item -Force (Join-Path '{dir}' 'ffmpeg.exe.part') (Join-Path '{dir}' 'ffmpeg.exe')",
        dir = dir.display()
    );
    let out = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .creation_flags(NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn works(name: &str) -> bool {
    Command::new(path())
        .args(["-v", "quiet", "-f", "lavfi", "-i", PROBE_SOURCE, "-t", "1", "-c:v", name, "-f", "null", "-"])
        .creation_flags(NO_WINDOW)
        .status()
        .is_ok_and(|s| s.success())
}

pub fn probe_encoders() {
    if !present() {
        return;
    }
    let mut results: Vec<(&'static str, bool)> = Vec::new();
    for (name, codec, _) in CANDIDATES {
        if results.iter().any(|(n, ok)| *ok && CANDIDATES.iter().any(|c| c.0 == *n && c.1 == codec)) {
            continue;
        }
        results.push((name, works(name)));
    }
    let _ = RESULTS.set(results);
}

pub fn encoders_ready() -> bool {
    RESULTS.get().is_some()
}

pub fn encoder_results() -> Vec<(&'static str, bool)> {
    RESULTS.get().cloned().unwrap_or_default()
}

pub fn pick(usable: &[String]) -> (&'static str, &'static str, f64) {
    let results = RESULTS.get().map(Vec::as_slice).unwrap_or_default();
    CANDIDATES
        .iter()
        .find(|(name, codec, _)| usable.iter().any(|u| u == codec) && results.iter().any(|(n, ok)| n == name && *ok))
        .copied()
        .unwrap_or(FALLBACK)
}

pub fn encoder() -> &'static str {
    pick(&crate::default_codecs()).0
}

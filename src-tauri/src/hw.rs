use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;
use windows_capture::monitor::Monitor;

use crate::{clean_text, debug, ffmpeg, now_us, record};
use crate::Core;

const TEXT_MAX: usize = 80;
pub const SLOW_SEND: Duration = Duration::from_millis(1500);
const SLOW_LOG_EVERY: Duration = Duration::from_secs(30);
const WINDOW_MS: u64 = 5000;
const MIN_SPAN_MS: u64 = 2000;
const BUSY_FPS: f64 = 5.0;
const KEEPING_UP: f64 = 0.5;
const DROP_SHARE: u64 = 4;
const RECOVERED: f64 = 0.8;
const MIN_REPORTED_S: u64 = 5;
const SCRIPT: &str = "[Console]::OutputEncoding=[Text.Encoding]::UTF8;$g=@(Get-CimInstance Win32_VideoController|ForEach-Object{@{n=$_.Name;d=$_.DriverVersion}});$c=(Get-CimInstance Win32_Processor|Select-Object -First 1).Name;$o=Get-CimInstance Win32_OperatingSystem;@{gpu=$g;cpu=$c;os=$o.Caption;ver=$o.Version;kb=$o.TotalVisibleMemorySize}|ConvertTo-Json -Compress";
const VIRTUAL_GPUS: [&str; 4] = ["basic", "remote", "virtual", "mirror"];
const VENDORS: [(&str, &str, &[&str]); 3] = [("nvenc", "NVENC", &["nvidia", "geforce"]), ("amf", "AMF", &["amd", "radeon"]), ("qsv", "QSV", &["intel"])];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hardware {
    pub gpus: Vec<(String, String)>,
    pub cpu: String,
    pub ram_mb: u64,
    pub display: (u32, u32),
    pub os: String,
}

static HARDWARE: OnceLock<Hardware> = OnceLock::new();
static LIVE: Mutex<Option<Live>> = Mutex::new(None);
static RATES: Mutex<Rates> = Mutex::new(Rates::new());
static SLOW: Mutex<Option<Limiter>> = Mutex::new(None);

fn text_of(v: &Value) -> String {
    clean_text(v.as_str().unwrap_or_default(), TEXT_MAX)
}

impl Hardware {
    pub fn parse(text: &str, display: (u32, u32)) -> Self {
        let v: Value = serde_json::from_str(text.trim()).unwrap_or_default();
        let list = match &v["gpu"] {
            Value::Array(a) => a.clone(),
            Value::Object(_) => vec![v["gpu"].clone()],
            _ => Vec::new(),
        };
        let gpus = list.iter().map(|g| (text_of(&g["n"]), text_of(&g["d"]))).filter(|(n, _)| !n.is_empty()).collect();
        let os = [text_of(&v["os"]), text_of(&v["ver"])].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
        Self { gpus, cpu: text_of(&v["cpu"]), ram_mb: v["kb"].as_u64().unwrap_or(0) / 1024, display, os }
    }
}

fn vendor(encoder: &str) -> Option<(&'static str, &'static str, &'static [&'static str])> {
    VENDORS.into_iter().find(|(key, _, _)| encoder.contains(key))
}

pub fn encoder_label(encoder: &str) -> &'static str {
    vendor(encoder).map_or("Software", |v| v.1)
}

pub fn pick_gpu<'a>(gpus: &'a [(String, String)], encoder: &str) -> Option<&'a (String, String)> {
    let named = |g: &&(String, String)| vendor(encoder).is_some_and(|v| v.2.iter().any(|w| g.0.to_lowercase().contains(w)));
    let real = |g: &&(String, String)| !VIRTUAL_GPUS.iter().any(|w| g.0.to_lowercase().contains(w));
    gpus.iter().find(named).or_else(|| gpus.iter().find(real)).or_else(|| gpus.first())
}

fn display_size() -> (u32, u32) {
    Monitor::primary().ok().and_then(|m| Some((m.width().ok()?, m.height().ok()?))).unwrap_or((0, 0))
}

pub fn warm() {
    std::thread::spawn(|| {
        let _ = HARDWARE.set(Hardware::parse(&debug::command_line("powershell", &["-NoProfile", "-NonInteractive", "-Command", SCRIPT]), display_size()));
    });
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Live {
    pub fps: f64,
    pub dropped: u64,
}

pub fn line(hw: Option<&Hardware>, encoder: Option<(&str, &str)>, live: Option<Live>) -> String {
    let Some(hw) = hw else { return "hardware: not read yet".into() };
    let gpu = pick_gpu(&hw.gpus, encoder.map_or("", |e| e.0));
    let (gpu, driver) = gpu.map_or(("?", "?"), |g| (g.0.as_str(), g.1.as_str()));
    let (label, codec) = encoder.map_or(("?", "?"), |(name, codec)| (encoder_label(name), codec));
    let mut out = format!("hardware: gpu {gpu} driver {driver} encoder {label} {codec} cpu {} ram {} MB display {}x{} os {}", hw.cpu, hw.ram_mb, hw.display.0, hw.display.1, hw.os);
    if let Some(l) = live {
        out.push_str(&format!(" capture {:.1} fps dropped {}", l.fps, l.dropped));
    }
    out
}

pub fn report(core: &Core) -> String {
    let encoder = ffmpeg::encoders_ready().then(|| ffmpeg::pick(&record::usable_codecs(core))).map(|(name, codec, _)| (name, codec));
    line(HARDWARE.get(), encoder, *LIVE.lock().unwrap())
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Counts {
    pub taken: u64,
    pub encoded: u64,
    pub dropped: u64,
}

pub struct Rates {
    samples: VecDeque<(u64, Counts)>,
    start: Option<(u64, Counts)>,
    collapsed: bool,
}

fn per_second(from: u64, to: u64, span_ms: u64) -> f64 {
    to.saturating_sub(from) as f64 * 1000.0 / span_ms.max(1) as f64
}

impl Rates {
    pub const fn new() -> Self {
        Self { samples: VecDeque::new(), start: None, collapsed: false }
    }

    pub fn tick(&mut self, now_ms: u64, recording: bool, c: Counts, target: f64) -> (Option<Live>, Option<String>) {
        if !recording {
            self.samples.clear();
            self.collapsed = false;
            let line = self.start.take().and_then(|(at, first)| ended(now_ms.saturating_sub(at) / 1000, first, c));
            return (None, line);
        }
        let (at, first) = *self.start.get_or_insert((now_ms, c));
        self.samples.push_back((now_ms, c));
        while self.samples.len() > 2 && now_ms.saturating_sub(self.samples[1].0) >= WINDOW_MS {
            self.samples.pop_front();
        }
        let (then, old) = self.samples[0];
        let span = now_ms.saturating_sub(then);
        let dropped = c.dropped.saturating_sub(first.dropped);
        if span < MIN_SPAN_MS {
            return (None, None);
        }
        let (captured, encoded) = (per_second(old.taken, c.taken, span), per_second(old.encoded, c.encoded, span));
        let lost = c.dropped.saturating_sub(old.dropped);
        let behind = captured >= BUSY_FPS && (encoded < captured * KEEPING_UP || lost * DROP_SHARE > c.taken.saturating_sub(old.taken));
        let mut line = None;
        if behind && !self.collapsed {
            self.collapsed = true;
            line = Some(format!("capture is falling behind after {} s: captured {captured:.1} fps, encoded {encoded:.1} fps of {target:.0}, dropped {lost} frames in the last {} s", now_ms.saturating_sub(at) / 1000, span / 1000));
        } else if self.collapsed && !behind && encoded >= target * RECOVERED {
            self.collapsed = false;
        }
        (Some(Live { fps: captured, dropped }), line)
    }
}

pub fn ended(seconds: u64, first: Counts, last: Counts) -> Option<String> {
    let encoded = last.encoded.saturating_sub(first.encoded);
    if seconds < MIN_REPORTED_S || encoded == 0 {
        return None;
    }
    let rate = |n: u64| n as f64 / seconds as f64;
    Some(format!("recording of {seconds} s ended: captured {:.1} fps, encoded {:.1} fps, dropped {} frames", rate(last.taken.saturating_sub(first.taken)), rate(encoded), last.dropped.saturating_sub(first.dropped)))
}

pub fn tick(core: &Core) {
    let frames = |i: usize| debug::FRAMES[i].load(Ordering::Relaxed);
    let counts = Counts { taken: frames(debug::TAKEN), encoded: frames(debug::ENCODED), dropped: frames(debug::DROPPED) };
    let recording = core.any_recording();
    let (live, line) = RATES.lock().unwrap().tick((now_us() / 1000) as u64, recording, counts, record::FPS as f64);
    *LIVE.lock().unwrap() = live;
    if let Some(line) = line {
        tracing::warn!("{line}");
    }
}

#[derive(Default)]
pub struct Limiter(HashMap<String, Instant>);

impl Limiter {
    pub fn allow(&mut self, key: &str, now: Instant) -> bool {
        if self.0.get(key).is_some_and(|t| now.saturating_duration_since(*t) < SLOW_LOG_EVERY) {
            return false;
        }
        self.0.insert(key.to_string(), now);
        true
    }
}

pub fn slow_line(peer: &str, took: Duration, queued: usize) -> Option<String> {
    (took >= SLOW_SEND).then(|| format!("slow send to {peer}: one write took {} ms, {queued} sends queued for it", took.as_millis()))
}

pub fn slow_send(peer: &str, took: Duration, queued: usize) {
    let Some(line) = slow_line(peer, took, queued) else { return };
    if SLOW.lock().unwrap().get_or_insert_with(Limiter::default).allow(peer, Instant::now()) {
        tracing::warn!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"gpu":[{"n":"Example Basic Display Adapter","d":"1.0"},{"n":"Test Graphics 9000","d":"31.0.15.1234"},{"n":"Sample Radeon X","d":"23.1.1"}],"cpu":" Fake CPU 3.5GHz ","os":"Microsoft Windows 11 Test","ver":"10.0.99999","kb":16777216}"#;

    fn counts(taken: u64, encoded: u64, dropped: u64) -> Counts {
        Counts { taken, encoded, dropped }
    }

    #[test]
    fn hardware_is_read_from_the_script_output() {
        let hw = Hardware::parse(SAMPLE, (2560, 1440));
        assert_eq!(hw.cpu, "Fake CPU 3.5GHz");
        assert_eq!(hw.ram_mb, 16384);
        assert_eq!(hw.display, (2560, 1440));
        assert_eq!(hw.os, "Microsoft Windows 11 Test 10.0.99999");
        assert_eq!(hw.gpus.len(), 3);
    }

    #[test]
    fn bad_output_gives_empty_hardware() {
        let hw = Hardware::parse("unavailable: nope", (0, 0));
        assert_eq!(hw, Hardware::default());
        let one = Hardware::parse(r#"{"gpu":{"n":"Only One","d":"1"}}"#, (1, 1));
        assert_eq!(one.gpus, [("Only One".to_string(), "1".to_string())]);
    }

    #[test]
    fn the_encoder_names_its_kind_and_the_matching_gpu() {
        assert_eq!(encoder_label("hevc_nvenc"), "NVENC");
        assert_eq!(encoder_label("h264_amf"), "AMF");
        assert_eq!(encoder_label("av1_qsv"), "QSV");
        assert_eq!(encoder_label("libx264"), "Software");
        let gpus = Hardware::parse(SAMPLE, (0, 0)).gpus;
        assert_eq!(pick_gpu(&gpus, "h264_amf").unwrap().0, "Sample Radeon X");
        assert_eq!(pick_gpu(&gpus, "libx264").unwrap().0, "Test Graphics 9000");
        assert_eq!(pick_gpu(&gpus, "h264_qsv").unwrap().0, "Test Graphics 9000");
        assert!(pick_gpu(&[], "libx264").is_none());
    }

    #[test]
    fn the_report_line_names_the_hardware_encoder_and_capture_rate() {
        let hw = Hardware::parse(SAMPLE, (1920, 1080));
        assert_eq!(line(Some(&hw), Some(("libx264", "h264")), None), "hardware: gpu Test Graphics 9000 driver 31.0.15.1234 encoder Software h264 cpu Fake CPU 3.5GHz ram 16384 MB display 1920x1080 os Microsoft Windows 11 Test 10.0.99999");
        assert!(line(Some(&hw), Some(("hevc_nvenc", "hevc")), Some(Live { fps: 29.96, dropped: 3 })).ends_with("capture 30.0 fps dropped 3"));
        assert!(line(Some(&hw), None, None).contains("encoder ? ?"));
        assert_eq!(line(None, None, None), "hardware: not read yet");
    }

    #[test]
    fn a_healthy_recording_logs_nothing_until_it_ends() {
        let mut m = Rates::new();
        for s in 0..=10u64 {
            let (live, line) = m.tick(s * 1000, true, counts(s * 30, s * 30, 0), 30.0);
            assert!(line.is_none());
            assert_eq!(live.is_some(), s >= 2);
        }
        let (live, line) = m.tick(11_000, false, counts(330, 330, 0), 30.0);
        assert!(live.is_none());
        assert_eq!(line.unwrap(), "recording of 11 s ended: captured 30.0 fps, encoded 30.0 fps, dropped 0 frames");
    }

    #[test]
    fn an_idle_screen_is_not_a_collapse() {
        let mut m = Rates::new();
        for s in 0..=20u64 {
            assert!(m.tick(s * 1000, true, counts(0, s * 10, 0), 30.0).1.is_none());
        }
    }

    #[test]
    fn a_stalled_encoder_is_logged_once_until_it_recovers() {
        let mut m = Rates::new();
        let mut lines = Vec::new();
        let (mut taken, mut encoded, mut dropped) = (0, 0, 0);
        for s in 0..=40u64 {
            taken += 30;
            if s < 10 || s > 25 {
                encoded += 30;
            } else {
                dropped += 28;
            }
            if let (_, Some(line)) = m.tick(s * 1000, true, counts(taken, encoded, dropped), 30.0) {
                lines.push((s, line));
            }
        }
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!((10..=13).contains(&lines[0].0), "{lines:?}");
        assert!(lines[0].1.contains("dropped"));
        let mut again = Vec::new();
        for s in 41..=60u64 {
            taken += 30;
            dropped += 28;
            if let (_, Some(line)) = m.tick(s * 1000, true, counts(taken, encoded, dropped), 30.0) {
                again.push(line);
            }
        }
        assert_eq!(again.len(), 1);
    }

    #[test]
    fn short_or_voice_only_recordings_end_quietly() {
        assert!(ended(3, counts(0, 0, 0), counts(90, 90, 0)).is_none());
        assert!(ended(60, counts(0, 0, 0), counts(0, 0, 0)).is_none());
        assert!(ended(60, counts(10, 10, 0), counts(1810, 1810, 4)).unwrap().ends_with("dropped 4 frames"));
    }

    #[test]
    fn slow_sends_are_logged_rarely() {
        assert!(slow_line("peer", Duration::from_millis(200), 1).is_none());
        assert_eq!(slow_line("peer", Duration::from_millis(2500), 3).unwrap(), "slow send to peer: one write took 2500 ms, 3 sends queued for it");
        let mut limiter = Limiter::default();
        let t = Instant::now();
        assert!(limiter.allow("a", t));
        assert!(!limiter.allow("a", t + Duration::from_secs(5)));
        assert!(limiter.allow("b", t + Duration::from_secs(5)));
        assert!(limiter.allow("a", t + SLOW_LOG_EVERY));
    }
}

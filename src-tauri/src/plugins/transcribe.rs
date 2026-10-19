use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

use serde_json::Value;

use super::{ffmpeg, Download, Found, Job, Media, Output, Plugin};
use crate::proc::{IDLE, NO_WINDOW};

pub const PLUGIN: Plugin = Plugin {
    id: "transcribe",
    name: "Transcription",
    summary: "Subtitles on the video and a conversation list you can click.",
    sample: "subtitles",
    version: "1.1.0",
    live: false, per_set: false,
    options: &[("model", &["small", "base", "medium", "large-v3-turbo"]), ("language", &["auto", "en", "ja", "ko", "zh", "es", "fr", "de", "pt", "ru"])],
    needs,
    run,
};

const TOOL_URL: &str = "https://github.com/ggml-org/whisper.cpp/releases/download/b5130/whisper-bin-x64.zip";
const TOOL_SHA256: &str = "f9ec6c52a2e949b62ab51fa21d0d497958f9e41c3010c157c4e42932d5316f3c";
const TOOL_SIZE: u64 = 8_573_270;
const TOOL_DIR: &str = "whisper";
const MODELS: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1";
const MODEL_FILES: [(&str, &str, &str, u64); 4] = [
    ("base", "ggml-base.bin", "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe", 147_951_465),
    ("small", "ggml-small.bin", "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b", 487_601_967),
    ("medium", "ggml-medium.bin", "6c14d5adee5f86394037b4e4e8b59f1673b6cee10e3cf0b11bbdbee79c156208", 1_533_763_059),
    ("large-v3-turbo", "ggml-large-v3-turbo-q5_0.bin", "394221709cd5ad1f40c46e6031ca61bce88931e6e088c188294c6d5a55ffa7e2", 574_041_195),
];
const RATE: &str = "16000";
const EXTRACT_SHARE: f64 = 0.1;
const SILENCE: &str = "silencedetect=noise=-45dB:d=1";
const LEVEL: &str = "speechnorm=e=6.25:r=0.00001:l=1";
const SOUND_MIN_S: f64 = 0.25;
const REPEAT_LOOKBACK: usize = 8;
const REPEAT_MAX: usize = 3;
const PREFIX_MIN_CHARS: usize = 8;

fn model(settings: &Value) -> (&'static str, &'static str, &'static str, u64) {
    let name = settings["model"].as_str().unwrap_or_default();
    let default = PLUGIN.options[0].1[0];
    MODEL_FILES.iter().copied().find(|m| m.0 == name).or_else(|| MODEL_FILES.iter().copied().find(|m| m.0 == default)).unwrap_or(MODEL_FILES[0])
}

fn needs(settings: &Value) -> Vec<Download> {
    let (_, file, sha256, size) = model(settings);
    vec![
        Download { url: TOOL_URL.into(), sha256: TOOL_SHA256, size: TOOL_SIZE, dest: TOOL_DIR.into(), pick: &["whisper-cli.exe", "*.dll"] },
        Download { url: format!("{MODELS}/{file}"), sha256, size, dest: file.into(), pick: &[] },
    ]
}

fn filters() -> String {
    format!("{LEVEL},{SILENCE}")
}

pub fn drop_note(raw: usize, kept: usize) -> String {
    if kept > 0 { String::new() } else { format!("raw {raw}, dropped {}", raw - kept) }
}

pub fn noise(text: &str) -> bool {
    let t = text.trim();
    let wrapped = [('[', ']'), ('(', ')'), ('（', '）'), ('*', '*'), ('♪', '♪')].iter().any(|(a, b)| t.starts_with(*a) && t.ends_with(*b));
    t.is_empty() || wrapped
}

pub fn silences(log: &str) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let mut start = None;
    for line in log.lines() {
        let value = |key: &str| line.split(key).nth(1)?.split_whitespace().next()?.parse::<f64>().ok();
        if let Some(s) = value("silence_start:") {
            start = Some(s.max(0.0));
        } else if let Some(e) = value("silence_end:") {
            out.push((start.take().unwrap_or(0.0), e));
        }
    }
    if let Some(s) = start {
        out.push((s, f64::INFINITY));
    }
    out
}

fn sound(from: f64, to: f64, silences: &[(f64, f64)]) -> f64 {
    let quiet: f64 = silences.iter().map(|(a, b)| (to.min(*b) - from.max(*a)).max(0.0)).sum();
    to - from - quiet
}

fn plain(t: &str) -> String {
    t.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

fn similar(a: &str, b: &str) -> bool {
    let (a, b) = (plain(a), plain(b));
    let (short, long) = if a.len() <= b.len() { (&a, &b) } else { (&b, &a) };
    a == b || (short.chars().count() >= PREFIX_MIN_CHARS && long.starts_with(short.as_str()))
}

pub fn segments(json: &Value, silences: &[(f64, f64)]) -> Vec<Found> {
    let mut out: Vec<Found> = Vec::new();
    for s in json["transcription"].as_array().into_iter().flatten() {
        let (Some(text), Some(from), Some(to)) = (s["text"].as_str().map(str::trim), s["offsets"]["from"].as_f64(), s["offsets"]["to"].as_f64()) else { continue };
        let (from, to) = (from / 1000.0, to / 1000.0);
        if noise(text) || to <= from || sound(from, to, silences) < SOUND_MIN_S {
            continue;
        }
        if out.iter().rev().take(REPEAT_LOOKBACK).any(|f| similar(&f.text, text)) || out.iter().filter(|f| plain(&f.text) == plain(text)).count() >= REPEAT_MAX {
            continue;
        }
        out.push(Found { kind: "speech".into(), from_s: Some(from), to_s: Some(to), text: text.to_string(), ..Default::default() });
    }
    out
}

pub fn percent(line: &str) -> Option<f64> {
    let rest = line.split("progress =").nth(1)?;
    rest.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|p| p / 100.0)
}

pub fn threads(cores: usize) -> usize {
    (cores / 2).max(1)
}

fn speech_source(job: &Job) -> &Media {
    if job.voice.parts.is_empty() { &job.sound } else { &job.voice }
}

fn run(job: &Job, progress: &mut dyn FnMut(f64)) -> Result<Output, String> {
    let speech = speech_source(job);
    if speech.parts.is_empty() {
        return Ok(Output::default());
    }
    std::fs::create_dir_all(&job.work).map_err(|e| e.to_string())?;
    let wav = job.work.join("voice.wav");
    let filters = filters();
    let after = ["-vn", "-af", &filters, "-ac", "1", "-ar", RATE, "-c:a", "pcm_s16le"].iter().map(|s| s.to_string()).chain([wav.display().to_string()]).collect::<Vec<_>>();
    let out = ffmpeg(job, speech, &["-v", "info", "-nostats"], &after, Stdio::null(), Stdio::piped()).and_then(|c| c.wait_with_output()).map_err(|e| e.to_string())?;
    let log = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return if log.contains("matches no streams") { Ok(Output::default()) } else { Err(log.trim().lines().last().unwrap_or_default().to_string()) };
    }
    let silences = silences(&log);
    progress(EXTRACT_SHARE);
    let (_, file, _, _) = model(&job.settings);
    let threads = threads(std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)).to_string();
    let base = job.work.join("voice");
    let mut child = Command::new(job.dir.join(TOOL_DIR).join("whisper-cli.exe"))
        .arg("-m")
        .arg(job.dir.join(file))
        .arg("-f")
        .arg(&wav)
        .args(["-l", job.settings["language"].as_str().unwrap_or("auto"), "-t", &threads, "-mc", "0", "--suppress-nst", "-oj", "-pp", "-of"])
        .arg(&base)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .creation_flags(NO_WINDOW | IDLE)
        .spawn()
        .map_err(|e| e.to_string())?;
    for line in BufReader::new(child.stderr.take().unwrap()).lines().map_while(Result::ok) {
        if let Some(p) = percent(&line) {
            progress(EXTRACT_SHARE + (1.0 - EXTRACT_SHARE) * p);
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("whisper stopped with {status}"));
    }
    let bytes = std::fs::read(base.with_extension("json")).map_err(|e| e.to_string())?;
    let json: Value = serde_json::from_str(&String::from_utf8_lossy(&bytes)).map_err(|e| e.to_string())?;
    let events = segments(&json, &silences);
    let note = drop_note(json["transcription"].as_array().map_or(0, Vec::len), events.len());
    Ok(Output { events, tags: BTreeMap::new(), note })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_file_without_a_voice_series_is_transcribed_from_its_sound() {
        let media = |path: &str| Media { parts: vec![path.into()], stream: "a:0".into() };
        let mut job = Job::default();
        assert!(speech_source(&job).parts.is_empty());
        job.sound = media("picture.mp4");
        assert_eq!(speech_source(&job).parts, [std::path::PathBuf::from("picture.mp4")]);
        job.voice = media("voice.mp4");
        assert_eq!(speech_source(&job).parts, [std::path::PathBuf::from("voice.mp4")]);
    }

    #[test]
    fn segments_become_ranges_without_noise() {
        let out = json!({"transcription": [
            {"offsets": {"from": 1000, "to": 2500}, "text": " Push left now."},
            {"offsets": {"from": 2500, "to": 4000}, "text": " [BLANK_AUDIO]"},
            {"offsets": {"from": 4000, "to": 5000}, "text": " (wind) "},
            {"offsets": {"from": 5000, "to": 5000}, "text": "zero"},
            {"offsets": {"from": 6000, "to": 7000}, "text": "   "}
        ]});
        assert_eq!(segments(&out, &[]), vec![Found { kind: "speech".into(), from_s: Some(1.0), to_s: Some(2.5), text: "Push left now.".into(), ..Default::default() }]);
    }

    #[test]
    fn silences_are_read_from_ffmpeg() {
        let log = "[silencedetect @ 0x1] silence_start: 2.5\n[silencedetect @ 0x1] silence_end: 10 | silence_duration: 7.5\nsize=1kB\n[silencedetect @ 0x1] silence_start: 30.25";
        assert_eq!(silences(log), vec![(2.5, 10.0), (30.25, f64::INFINITY)]);
        assert!(silences("nothing").is_empty());
    }

    #[test]
    fn lines_in_silence_and_repeats_are_dropped() {
        let line = |from: i64, to: i64, text: &str| json!({"offsets": {"from": from, "to": to}, "text": text});
        let out = json!({"transcription": [
            line(0, 2000, "Push left now."),
            line(2000, 4000, "push left now"),
            line(4000, 6000, "Rotate."),
            line(10000, 30000, "Thanks for watching."),
            line(31000, 33000, "Thanks for watching."),
            line(40000, 42000, "Push left now."),
            line(44000, 46000, "Go."),
            line(48000, 50000, "Back."),
            line(52000, 54000, "Push left now."),
            line(60000, 68000, "Hold."),
        ]});
        let texts: Vec<String> = segments(&out, &[(9.0, 30.5), (61.0, 70.0)]).into_iter().map(|f| f.text).collect();
        assert_eq!(texts, ["Push left now.", "Rotate.", "Thanks for watching.", "Go.", "Back.", "Hold."]);
    }

    #[test]
    fn a_looping_line_is_kept_three_times_at_most() {
        let line = |at: i64, text: &str| json!({"offsets": {"from": at * 1000, "to": at * 1000 + 3000}, "text": text});
        let others = ["Rotate east", "Smoke the door", "Reload quickly", "Watch the flank", "Take the ridge", "Heal behind me", "Ping the squad", "Drop to the left", "Check the roof", "Cover my back", "Swap to the rifle", "Wait for the ring", "Third party coming", "Pick up the shield", "Fall back now"];
        let mut all = Vec::new();
        for i in 0..30usize {
            all.push(line(i as i64 * 10, "Hold the high ground now."));
            if i % 2 == 1 {
                all.push(line(i as i64 * 10 + 5, others[i / 2]));
            }
        }
        let texts: Vec<String> = segments(&json!({"transcription": all}), &[]).into_iter().map(|f| f.text).collect();
        let held = texts.iter().filter(|t| t.starts_with("Hold")).count();
        assert!((1..=3).contains(&held), "{held} copies");
        assert_eq!(texts.len(), held + others.len());
    }

    #[test]
    fn a_trailing_particle_is_a_repeat() {
        assert!(similar("Hold the high ground.", "Hold the high ground now"));
        assert!(!similar("Go.", "Go ahead."));
        assert!(!similar("Push left.", "Push right."));
    }

    #[test]
    fn the_level_is_raised_before_silence_is_detected() {
        let filters = filters();
        assert!(filters.find("speechnorm").unwrap() < filters.find("silencedetect").unwrap());
    }

    #[test]
    fn an_empty_result_says_how_much_was_dropped() {
        assert_eq!(drop_note(12, 0), "raw 12, dropped 12");
        assert_eq!(drop_note(0, 0), "raw 0, dropped 0");
        assert_eq!(drop_note(12, 3), "");
    }

    #[test]
    fn a_quiet_voice_survives_when_silence_is_measured_after_the_boost() {
        let line = json!({"transcription": [{"offsets": {"from": 0, "to": 2000}, "text": "Push left now."}]});
        assert!(segments(&line, &[(0.0, f64::INFINITY)]).is_empty());
        assert_eq!(segments(&line, &[]).len(), 1);
    }

    #[test]
    fn whisper_leaves_half_the_cores_free() {
        assert_eq!(threads(16), 8);
        assert_eq!(threads(1), 1);
    }

    #[test]
    fn progress_lines_are_read() {
        assert_eq!(percent("whisper_print_progress_callback: progress =  40%"), Some(0.4));
        assert_eq!(percent("whisper_init: loading"), None);
    }

    #[test]
    fn needs_follow_the_model() {
        let needs = needs(&json!({"model": "base"}));
        assert_eq!(needs.len(), 2);
        assert!(needs[1].url.ends_with("/ggml-base.bin"));
        assert_eq!(needs[1].dest, "ggml-base.bin");
        assert_eq!(model(&json!({})).0, "small");
    }
}

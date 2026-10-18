use std::collections::BTreeMap;
use std::io::{BufRead, BufReader};
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

use serde_json::Value;

use super::{ffmpeg, Download, Found, Job, Output, Plugin};
use crate::proc::{IDLE, NO_WINDOW};

pub const PLUGIN: Plugin = Plugin {
    id: "transcribe",
    name: "Transcription",
    version: "1.0.0",
    live: false,
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

pub fn noise(text: &str) -> bool {
    let t = text.trim();
    let wrapped = [('[', ']'), ('(', ')'), ('（', '）'), ('*', '*'), ('♪', '♪')].iter().any(|(a, b)| t.starts_with(*a) && t.ends_with(*b));
    t.is_empty() || wrapped
}

pub fn segments(json: &Value) -> Vec<Found> {
    json["transcription"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let text = s["text"].as_str()?.trim();
            let (from, to) = (s["offsets"]["from"].as_f64()?, s["offsets"]["to"].as_f64()?);
            (!noise(text) && to > from).then(|| Found { kind: "speech".into(), from_s: Some(from / 1000.0), to_s: Some(to / 1000.0), text: text.to_string(), ..Default::default() })
        })
        .collect()
}

pub fn percent(line: &str) -> Option<f64> {
    let rest = line.split("progress =").nth(1)?;
    rest.trim().trim_end_matches('%').trim().parse::<f64>().ok().map(|p| p / 100.0)
}

pub fn threads(cores: usize) -> usize {
    (cores / 2).max(1)
}

fn run(job: &Job, progress: &mut dyn FnMut(f64)) -> Result<Output, String> {
    if job.voice.parts.is_empty() {
        return Ok(Output::default());
    }
    std::fs::create_dir_all(&job.work).map_err(|e| e.to_string())?;
    let wav = job.work.join("voice.wav");
    let after = ["-vn", "-ac", "1", "-ar", RATE, "-c:a", "pcm_s16le"].iter().map(|s| s.to_string()).chain([wav.display().to_string()]).collect::<Vec<_>>();
    let out = ffmpeg(job, &job.voice, &["-v", "error"], &after, Stdio::null(), Stdio::piped()).and_then(|c| c.wait_with_output()).map_err(|e| e.to_string())?;
    if !out.status.success() {
        let error = String::from_utf8_lossy(&out.stderr);
        return if error.contains("matches no streams") { Ok(Output::default()) } else { Err(error.trim().lines().last().unwrap_or_default().to_string()) };
    }
    progress(EXTRACT_SHARE);
    let (_, file, _, _) = model(&job.settings);
    let threads = threads(std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)).to_string();
    let base = job.work.join("voice");
    let mut child = Command::new(job.dir.join(TOOL_DIR).join("whisper-cli.exe"))
        .arg("-m")
        .arg(job.dir.join(file))
        .arg("-f")
        .arg(&wav)
        .args(["-l", job.settings["language"].as_str().unwrap_or("auto"), "-t", &threads, "-oj", "-pp", "-of"])
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
    Ok(Output { events: segments(&json), tags: BTreeMap::new() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn segments_become_ranges_without_noise() {
        let out = json!({"transcription": [
            {"offsets": {"from": 1000, "to": 2500}, "text": " Push left now."},
            {"offsets": {"from": 2500, "to": 4000}, "text": " [BLANK_AUDIO]"},
            {"offsets": {"from": 4000, "to": 5000}, "text": " (wind) "},
            {"offsets": {"from": 5000, "to": 5000}, "text": "zero"},
            {"offsets": {"from": 6000, "to": 7000}, "text": "   "}
        ]});
        assert_eq!(segments(&out), vec![Found { kind: "speech".into(), from_s: Some(1.0), to_s: Some(2.5), text: "Push left now.".into(), ..Default::default() }]);
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

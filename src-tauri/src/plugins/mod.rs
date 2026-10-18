use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub use crate::download::{fetch, present, Download};
use crate::proc::{self, Process, IDLE, NO_WINDOW};

mod apex;
mod transcribe;

pub const FLAG: &str = "--plugin";

pub struct Plugin {
    pub id: &'static str,
    pub name: &'static str,
    pub version: &'static str,
    pub live: bool,
    pub options: &'static [(&'static str, &'static [&'static str])],
    pub needs: fn(&Value) -> Vec<Download>,
    pub run: fn(&Job, &mut dyn FnMut(f64)) -> Result<Output, String>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct Media {
    pub parts: Vec<PathBuf>,
    pub stream: String,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Job {
    pub plugin: String,
    pub settings: Value,
    pub set: Value,
    pub file: Value,
    pub duration_s: f64,
    pub picture: Media,
    pub sound: Media,
    pub voice: Media,
    pub ffmpeg: PathBuf,
    pub dir: PathBuf,
    pub work: PathBuf,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct Found {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_s: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_s: Option<f64>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub values: Value,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct Output {
    pub events: Vec<Found>,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
}

#[derive(Debug, PartialEq)]
pub enum Reply {
    Progress(f64),
    Done(Output),
    Failed(String),
}

pub const ALL: [Plugin; 2] = [apex::PLUGIN, transcribe::PLUGIN];

pub fn find(id: &str) -> Option<&'static Plugin> {
    ALL.iter().find(|p| p.id == id)
}

pub fn normalize(plugin: &Plugin, settings: &Value) -> Value {
    let mut out = json!({});
    for (key, choices) in plugin.options {
        let value = settings[*key].as_str().filter(|v| choices.contains(v)).unwrap_or(choices[0]);
        out[*key] = json!(value);
    }
    out
}

pub fn manifest(plugin: &Plugin) -> Value {
    let options: Value = plugin.options.iter().map(|(key, choices)| json!({"key": key, "choices": choices, "default": choices[0]})).collect();
    json!({"id": plugin.id, "name": plugin.name, "version": plugin.version, "live": plugin.live, "options": options})
}

pub fn missing(plugin: &Plugin, dir: &Path, settings: &Value) -> Vec<Download> {
    (plugin.needs)(settings).into_iter().filter(|d| !present(dir, d)).collect()
}

pub fn child() -> Option<i32> {
    if std::env::args().nth(1).as_deref() != Some(FLAG) {
        return None;
    }
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    let result = serde_json::from_str::<Job>(&text).map_err(|e| e.to_string()).and_then(|job| {
        let plugin = find(&job.plugin).ok_or("unknown plugin")?;
        let mut stdout = std::io::stdout();
        (plugin.run)(&job, &mut |p| {
            if writeln!(stdout, "{}", json!({"progress": p})).and_then(|_| stdout.flush()).is_err() {
                std::process::exit(1);
            }
        })
    });
    let line = match result {
        Ok(output) => json!({"output": output}),
        Err(e) => json!({"error": e}),
    };
    println!("{line}");
    Some(0)
}

pub fn spawn(job: &Job) -> std::io::Result<Process> {
    let mut process = proc::spawn(Command::new(std::env::current_exe()?).arg(FLAG).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()), IDLE)?;
    let text = serde_json::to_string(job).unwrap_or_default();
    let written = process.child.stdin.take().map_or(Ok(()), |mut stdin| stdin.write_all(text.as_bytes()));
    if let Err(e) = written {
        process.kill();
        return Err(e);
    }
    Ok(process)
}

pub fn reply(line: &str) -> Option<Reply> {
    let v: Value = serde_json::from_str(line).ok()?;
    if let Some(p) = v["progress"].as_f64() {
        return Some(Reply::Progress(p.clamp(0.0, 1.0)));
    }
    if let Some(e) = v["error"].as_str() {
        return Some(Reply::Failed(e.to_string()));
    }
    serde_json::from_value(v["output"].clone()).ok().map(Reply::Done)
}

pub fn replies(stdout: ChildStdout) -> mpsc::Receiver<Reply> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(r) = reply(&line) {
                if tx.send(r).is_err() {
                    break;
                }
            }
        }
    });
    rx
}

pub fn ffmpeg(job: &Job, media: &Media, before: &[&str], after: &[String], stdout: Stdio, stderr: Stdio) -> std::io::Result<Child> {
    let piped = media.parts.len() != 1;
    let input = if piped { "pipe:0".into() } else { media.parts[0].display().to_string() };
    let mut child = Command::new(&job.ffmpeg)
        .args(["-hide_banner", "-y"])
        .args(before)
        .args(["-i", &input, "-map", &format!("0:{}", media.stream)])
        .args(after)
        .stdin(if piped { Stdio::piped() } else { Stdio::null() })
        .stdout(stdout)
        .stderr(stderr)
        .creation_flags(NO_WINDOW | IDLE)
        .spawn()?;
    if piped {
        let mut stdin = child.stdin.take().unwrap();
        let parts = media.parts.clone();
        std::thread::spawn(move || {
            for part in parts {
                let Ok(mut file) = std::fs::File::open(part) else { break };
                if std::io::copy(&mut file, &mut stdin).is_err() {
                    break;
                }
            }
        });
    }
    Ok(child)
}

pub fn pts_time(line: &str) -> Option<f64> {
    let rest = line.split("pts_time:").nth(1)?;
    rest.split_whitespace().next()?.parse().ok()
}

pub fn keyframes(job: &Job, filter: &str, width: usize, height: usize, mut each: impl FnMut(f64, &[u8])) -> Result<(), String> {
    let after = vec![
        "-an".to_string(),
        "-vf".into(),
        format!("{filter},format=yuv444p,showinfo"),
        "-fps_mode".into(),
        "passthrough".into(),
        "-f".into(),
        "rawvideo".into(),
        "-pix_fmt".into(),
        "yuv444p".into(),
        "pipe:1".into(),
    ];
    let mut child = ffmpeg(job, &job.picture, &["-v", "info", "-skip_frame", "nokey"], &after, Stdio::piped(), Stdio::piped()).map_err(|e| e.to_string())?;
    let stderr = child.stderr.take().unwrap();
    let (tx, times) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.contains("Parsed_showinfo") && line.contains("pts_time:") && tx.send(pts_time(&line)).is_err() {
                break;
            }
        }
    });
    let mut stdout = child.stdout.take().unwrap();
    let mut frame = vec![0u8; width * height * 3];
    while stdout.read_exact(&mut frame).is_ok() {
        let Ok(t) = times.recv() else { break };
        if let Some(t) = t {
            each(t, &frame);
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("ffmpeg stopped with {status}")) }
}

pub struct Plane<'a> {
    pub data: &'a [u8],
    pub width: usize,
}

impl Plane<'_> {
    pub fn pixels(&self, x: usize, y: usize, w: usize, h: usize) -> impl Iterator<Item = u8> + '_ {
        (y..y + h).flat_map(move |row| self.data[row * self.width + x..row * self.width + x + w].iter().copied())
    }

    pub fn mean(&self, x: usize, y: usize, w: usize, h: usize) -> f64 {
        self.pixels(x, y, w, h).map(f64::from).sum::<f64>() / (w * h).max(1) as f64
    }
}

pub fn planes(frame: &[u8], width: usize, height: usize) -> [Plane<'_>; 3] {
    let n = width * height;
    [Plane { data: &frame[..n], width }, Plane { data: &frame[n..2 * n], width }, Plane { data: &frame[2 * n..3 * n], width }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_keep_known_choices_and_fill_defaults() {
        let plugin = find("transcribe").unwrap();
        let s = normalize(plugin, &json!({"model": "base", "language": "xx", "other": 1}));
        assert_eq!(s, json!({"model": "base", "language": "auto"}));
        assert_eq!(normalize(find("apex-matches").unwrap(), &json!({"a": 1})), json!({}));
    }

    #[test]
    fn replies_are_read_from_lines() {
        assert_eq!(reply(r#"{"progress": 1.5}"#), Some(Reply::Progress(1.0)));
        assert_eq!(reply(r#"{"error": "no"}"#), Some(Reply::Failed("no".into())));
        let done = reply(r#"{"output": {"events": [{"kind": "x", "at_s": 2.0}]}}"#);
        assert_eq!(done, Some(Reply::Done(Output { events: vec![Found { kind: "x".into(), at_s: Some(2.0), ..Default::default() }], tags: BTreeMap::new() })));
        assert_eq!(reply("noise"), None);
    }

    #[test]
    fn showinfo_times_are_parsed() {
        let line = "[Parsed_showinfo_3 @ 0000] n:   4 pts:  12288 pts_time:4.0    duration:512";
        assert_eq!(pts_time(line), Some(4.0));
        assert_eq!(pts_time("frame= 10"), None);
        assert_eq!(pts_time("[Parsed_showinfo_3 @ 0000] n:   5 pts:NOPTS pts_time:NOPTS duration:512"), None);
    }

    #[test]
    fn planes_give_region_means() {
        let (w, h) = (4, 2);
        let mut frame = vec![0u8; w * h * 3];
        frame[5] = 9;
        frame[w * h + 1] = 4;
        frame[w * h + 2] = 8;
        let [y, u, _] = planes(&frame, w, h);
        assert_eq!(y.mean(0, 1, 4, 1), 9.0 / 4.0);
        assert_eq!(u.mean(1, 0, 2, 1), 6.0);
    }

    #[test]
    fn needs_are_present_only_when_their_files_are() {
        let dir = std::env::temp_dir().join(format!("ezh-test-needs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let plugin = find("transcribe").unwrap();
        let settings = normalize(plugin, &json!({"model": "base"}));
        assert_eq!(missing(plugin, &dir, &settings).len(), 2);
        let needs = (plugin.needs)(&settings);
        std::fs::create_dir_all(dir.join(&needs[0].dest)).unwrap();
        std::fs::write(dir.join(&needs[0].dest).join(needs[0].pick[0]), b"").unwrap();
        assert_eq!(missing(plugin, &dir, &settings).len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::mpsc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::proc::{self, Process, IDLE, NO_WINDOW};

mod apex;
mod apex_fights;
mod excitement;

pub const FLAG: &str = "--plugin";
pub const LINE_MAX: usize = 16 << 20;

pub struct Plugin {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub sample: &'static str,
    pub version: &'static str,
    pub kind: Kind,
    pub options: &'static [Choice],
}

pub struct Choice {
    pub id: &'static str,
    pub name: &'static str,
    pub empty: &'static str,
}

pub enum Kind {
    Analysis(fn(&Job, &mut dyn FnMut(f64)) -> Result<Output, String>),
    View(&'static str),
}

impl Plugin {
    pub fn analyzes(&self) -> bool {
        matches!(self.kind, Kind::Analysis(_))
    }
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct Media {
    pub parts: Vec<PathBuf>,
    pub stream: String,
    #[serde(default)]
    pub start_s: f64,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
pub struct Job {
    pub plugin: String,
    pub duration_s: f64,
    pub picture: Media,
    #[serde(default)]
    pub voice: Media,
    pub ffmpeg: PathBuf,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
pub struct Found {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_s: Option<f64>,
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

pub const ALL: [Plugin; 5] = [excitement::PLUGIN, apex::PLUGIN, apex_fights::DOWNS, apex_fights::KILLS, apex::MINIMAP];

pub fn find(id: &str) -> Option<&'static Plugin> {
    ALL.iter().find(|p| p.id == id)
}

pub fn manifest(plugin: &Plugin) -> Value {
    let view = match plugin.kind {
        Kind::View(view) => Some(view),
        Kind::Analysis(_) => None,
    };
    json!({"id": plugin.id, "name": plugin.name, "summary": plugin.summary, "sample": plugin.sample, "version": plugin.version, "view": view, "options": plugin.options.iter().map(|o| json!({"id": o.id, "name": o.name, "empty": o.empty})).collect::<Vec<_>>()})
}

pub fn child() -> Option<i32> {
    if std::env::args().nth(1).as_deref() != Some(FLAG) {
        return None;
    }
    let mut text = String::new();
    let _ = std::io::stdin().read_to_string(&mut text);
    let result = serde_json::from_str::<Job>(&text).map_err(|e| e.to_string()).and_then(|job| {
        let Some(Kind::Analysis(run)) = find(&job.plugin).map(|p| &p.kind) else { return Err("unknown plugin".to_string()) };
        let mut stdout = std::io::stdout();
        run(&job, &mut |p| {
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

pub fn read_line(reader: &mut impl BufRead, max: usize) -> Option<Result<String, String>> {
    let mut line = Vec::new();
    match reader.take(max as u64 + 1).read_until(b'\n', &mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) if line.len() > max => Some(Err("The plugin wrote more than the app reads in one line.".into())),
        Ok(_) => Some(Ok(String::from_utf8_lossy(&line).trim_end().to_string())),
    }
}

pub fn replies(stdout: ChildStdout) -> mpsc::Receiver<Reply> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        while let Some(line) = read_line(&mut reader, LINE_MAX) {
            let r = match line {
                Ok(line) => reply(&line),
                Err(e) => Some(Reply::Failed(e)),
            };
            let stop = matches!(r, Some(Reply::Failed(_)));
            if r.is_some_and(|r| tx.send(r).is_err()) || stop {
                break;
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

pub fn sound(job: &Job, media: &Media, rate: u32, window: usize, mut each: impl FnMut(&[i16])) -> Result<(), String> {
    let after = ["-vn", "-ac", "1", "-ar", &rate.to_string(), "-f", "s16le", "pipe:1"].map(String::from);
    let mut child = ffmpeg(job, media, &["-v", "error"], &after, Stdio::piped(), Stdio::null()).map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().unwrap();
    let mut bytes = vec![0u8; window * 2];
    let mut samples = vec![0i16; window];
    while stdout.read_exact(&mut bytes).is_ok() {
        for (s, b) in samples.iter_mut().zip(bytes.chunks_exact(2)) {
            *s = i16::from_le_bytes([b[0], b[1]]);
        }
        each(&samples);
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
    fn replies_are_read_from_lines() {
        assert_eq!(reply(r#"{"progress": 1.5}"#), Some(Reply::Progress(1.0)));
        assert_eq!(reply(r#"{"error": "no"}"#), Some(Reply::Failed("no".into())));
        let done = reply(r#"{"output": {"events": [{"kind": "x", "at_s": 2.0}]}}"#);
        assert_eq!(done, Some(Reply::Done(Output { events: vec![Found { kind: "x".into(), at_s: Some(2.0), ..Default::default() }], tags: BTreeMap::new() })));
        assert_eq!(reply("noise"), None);
    }

    #[test]
    fn reply_lines_are_capped() {
        let mut short = std::io::Cursor::new(b"{\"progress\": 0.5}\nnext\n".to_vec());
        assert_eq!(read_line(&mut short, 32), Some(Ok("{\"progress\": 0.5}".to_string())));
        assert_eq!(read_line(&mut short, 32), Some(Ok("next".to_string())));
        assert_eq!(read_line(&mut short, 32), None);
        let mut long = std::io::Cursor::new(vec![b'x'; 100]);
        assert!(read_line(&mut long, 32).unwrap().is_err());
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
}

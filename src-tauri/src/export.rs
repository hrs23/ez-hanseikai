use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{Emitter, Manager};

use crate::ffmpeg;
use crate::proc::{self, Process, BELOW_NORMAL};
use crate::record::{self, AUDIO_KBPS, FPS};
use crate::room::{self, part_of, File, Set, SERIES};
use crate::stream::{self, Piece};
use crate::{tenth, Core, NAME_MAX};

const NOTES: &str = "notes.csv";
const CHAPTERS: &str = "chapters.vtt";
const WATCH: &str = "watch.json";
const TOLERANCE_S: f64 = 0.05;
const DEFAULT_SIZE: (u32, u32) = (1280, 720);
const FEED_CHUNK: u64 = 1 << 20;
const CONNECT_WAIT: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(50);
const ERROR_LINES: usize = 12;
const LIGHT: f64 = 0.5;
const FOLDER_MAX: usize = 80;
const APP_FOLDER: &str = "EZ Hanseikai";
const RESERVED: [&str; 22] = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Range {
    #[default]
    Whole,
    Matches,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Original,
    #[default]
    Standard,
    Light,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Folder,
    DebriefVideo,
}

pub struct Options {
    pub range: Range,
    pub quality: Quality,
    pub member: Option<String>,
    pub kind: Kind,
    pub burn: crate::debrief::Burn,
}

pub struct Running {
    pub set: (String, String),
    child: Process,
    cancelled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Input {
    pub delay_s: f64,
    pub end_s: f64,
    pub video: bool,
    pub audio: bool,
}

#[derive(Clone)]
pub struct Captured {
    pub input: Input,
    pub pieces: Vec<Piece>,
    pub times: Vec<(f64, f64)>,
}

#[derive(Debug, PartialEq)]
pub struct Note {
    pub at_s: f64,
    pub kind: String,
    pub text: String,
    pub tags: Vec<String>,
    pub by: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub from_s: f64,
    pub to_s: f64,
    pub text: String,
}

#[derive(Clone)]
pub struct Output {
    pub name: String,
    pub owner: String,
    pub video: Vec<Captured>,
    pub voice: Vec<Captured>,
    pub size: (u32, u32),
    pub kbps: f64,
}

pub struct Items {
    pub notes: Vec<Note>,
    pub chapters: Vec<Cue>,
    pub matches: Vec<Cue>,
}

#[derive(Clone, Copy)]
struct Take {
    total: f64,
    quality: Quality,
    whole: bool,
}

pub struct Job {
    pub feeds: Vec<Vec<Piece>>,
    pub args: Vec<String>,
    pub partial: PathBuf,
    pub path: PathBuf,
    pub total: f64,
}

fn source_kbps(cap: &Value) -> f64 {
    let width = cap["width"].as_u64().map_or(record::quality_width(""), |w| w as u32);
    let kbps = cap["kbps"].as_f64().unwrap_or_else(|| record::bitrate_kbps(width, 1.0) as f64);
    kbps / ffmpeg::CANDIDATES.iter().find(|c| c.1 == cap["codec"]).map_or(1.0, |c| c.2)
}

fn export_kbps(sources: &[f64], size: (u32, u32), factor: f64) -> u32 {
    let ceiling = record::bitrate_kbps(size.0.max(size.1 * 16 / 9), 1.0) as f64;
    (sources.iter().sum::<f64>().min(ceiling) * factor).round() as u32
}

fn fit(size: (u32, u32), limit: (u32, u32)) -> (u32, u32) {
    let scale = (limit.0 as f64 / size.0.max(1) as f64).min(limit.1 as f64 / size.1.max(1) as f64).min(1.0);
    ((size.0 as f64 * scale).round() as u32 & !1, (size.1 as f64 * scale).round() as u32 & !1)
}

pub fn picture(quality: Quality, size: (u32, u32), source: f64, factor: f64) -> ((u32, u32), u32) {
    match quality {
        Quality::Original => (size, (source * factor).round() as u32),
        Quality::Standard => (size, export_kbps(&[source], size, factor)),
        Quality::Light => {
            let small = fit(size, DEFAULT_SIZE);
            (small, export_kbps(&[source], small, factor * LIGHT))
        }
    }
}

pub fn clip(c: &Captured, from: f64, to: f64) -> Option<Captured> {
    let picked: Vec<usize> = (0..c.times.len()).filter(|&i| c.times[i].1 > from && c.times[i].0 < to).collect();
    let (first, last) = (*picked.first()?, *picked.last()?);
    let pieces = std::iter::once(c.pieces[0].clone()).chain(picked.iter().map(|&i| c.pieces[i + 1].clone())).collect();
    let times = picked.iter().map(|&i| (c.times[i].0 - from, c.times[i].1 - from)).collect();
    Some(Captured { input: Input { delay_s: c.times[first].0 - from, end_s: c.times[last].1 - from, ..c.input.clone() }, pieces, times })
}

fn within(out: &Output, from: f64, to: f64) -> Option<Output> {
    let video: Vec<Captured> = out.video.iter().filter_map(|c| clip(c, from, to)).collect();
    let voice: Vec<Captured> = out.voice.iter().filter_map(|c| clip(c, from, to)).collect();
    (!video.is_empty() || !voice.is_empty()).then(|| Output { name: out.name.clone(), owner: out.owner.clone(), video, voice, size: out.size, kbps: out.kbps })
}

fn match_stem(n: usize, from_s: f64, file: &str) -> String {
    let s = from_s.max(0.0).round() as u64;
    format!("Match {n:02} {}-{:02}-{:02} {file}", s / 3600, s / 60 % 60, s % 60)
}

pub fn cut(a: &str, delay: f64) -> String {
    if delay < 0.0 { format!("{a}setpts=PTS-STARTPTS,{a}trim=start={:.3},", -delay) } else { String::new() }
}

fn delayed_sound(i: usize, delay: f64) -> String {
    format!("[{i}:a:0]{}asetpts=PTS-STARTPTS,adelay=delays={}:all=1[s{i}];", cut("a", delay), (delay.max(0.0) * 1000.0).round() as u64)
}

fn amix(labels: &[String], total: f64) -> String {
    format!("{}amix=inputs={}:normalize=0:duration=longest,apad=whole_dur={total:.3}", labels.concat(), labels.len())
}

fn graph(inputs: &[Input], size: (u32, u32), total: f64) -> (String, bool) {
    let (w, h) = size;
    let mut graph = format!("color=c=black:s={w}x{h}:r={FPS}:d={total:.3}[bg0];");
    let (mut layer, mut sounds) = (0usize, Vec::new());
    for (i, input) in inputs.iter().enumerate() {
        if input.video {
            graph += &format!(
                "[{i}:v]{}setpts=PTS-STARTPTS+{:.3}/TB,scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1[v{i}];[bg{layer}][v{i}]overlay=eof_action=pass[bg{}];",
                cut("", input.delay_s),
                input.delay_s.max(0.0),
                layer + 1
            );
            layer += 1;
        }
        if input.audio {
            graph += &delayed_sound(i, input.delay_s);
            sounds.push(format!("[s{i}]"));
        }
    }
    graph += &format!("[bg{layer}]format=yuv420p[vout]");
    if !sounds.is_empty() {
        graph += &format!(";{}[mix]", amix(&sounds, total));
    }
    (graph, !sounds.is_empty())
}

pub fn voice_graph(inputs: &[Input], total: f64) -> String {
    let sounds: Vec<String> = (0..inputs.len()).map(|i| format!("[s{i}]")).collect();
    inputs.iter().enumerate().map(|(i, input)| delayed_sound(i, input.delay_s)).collect::<String>() + &amix(&sounds, total) + "[mix]"
}

fn copyable(inputs: &[Input], total: f64) -> bool {
    matches!(inputs, [only] if only.delay_s <= TOLERANCE_S && only.end_s >= total - TOLERANCE_S)
}

fn safe_name(name: &str, max: usize, fallback: &str) -> String {
    let fit = |text: &str| text.trim_start().chars().take(max).collect::<String>().trim_end_matches(['.', ' ']).to_string();
    let cleaned = fit(&name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect::<String>());
    let named = if cleaned.is_empty() { fallback.to_string() } else { cleaned };
    let stem = named.split('.').next().unwrap_or_default().trim_end().to_uppercase();
    if RESERVED.contains(&stem.as_str()) { fit(&format!("_{named}")) } else { named }
}

fn file_name(name: &str) -> String {
    safe_name(name, 60, "view")
}

fn unused_name(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut candidate = name.to_string();
    let mut n = 2;
    while taken(&candidate) {
        let suffix = format!(" ({n})");
        let head: String = name.chars().take(FOLDER_MAX.saturating_sub(suffix.len())).collect();
        candidate = format!("{}{suffix}", head.trim_end_matches(['.', ' ']));
        n += 1;
    }
    candidate
}

fn folder_name(room: &str, set_id: &str, title: Option<&str>, debrief_s: Option<f64>) -> String {
    let when = set_id.get(..16).unwrap_or(set_id);
    let raw = match debrief_s {
        Some(s) => format!("{room} debrief {when} {} min", (s / 60.0).round().max(1.0)),
        None => format!("{} {room}", title.map(str::trim).filter(|t| !t.is_empty()).unwrap_or(when)),
    };
    safe_name(raw.trim(), FOLDER_MAX, "Recording")
}

fn default_base(core: &Core) -> Option<PathBuf> {
    if let Some(base) = core.export_base.lock().unwrap().clone() {
        return Some(PathBuf::from(base));
    }
    let path = core.app.path();
    path.video_dir().or_else(|_| path.document_dir()).ok().map(|d| d.join(APP_FOLDER))
}

pub fn target(core: &Core, room_id: &str, set_id: &str, base: Option<PathBuf>, name: Option<String>) -> Result<PathBuf, String> {
    let base = base.or_else(|| default_base(core)).ok_or("Choose a folder.")?;
    let name = match name {
        Some(name) => safe_name(&name, FOLDER_MAX, "Recording"),
        None => {
            let logs = room::logs(core, room_id);
            let set = room::sets(core, room_id, &logs).into_iter().find(|s| s.id == set_id).ok_or("Unknown recording.")?;
            let title = set.files.iter().find_map(|f| f.view.as_ref()).and_then(|v| v.title.clone());
            let debrief_s = room::debrief_of(set_id, &logs).map(|_| (set.end_shown().unwrap_or(set.start()) - set.start()) as f64 / 1e6);
            folder_name(&core.room(room_id).map(|r| r.name).unwrap_or_default(), set_id, title.as_deref(), debrief_s)
        }
    };
    Ok(base.join(unused_name(&name, |n| base.join(n).exists())))
}

fn unique_names(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        let base = if name.is_empty() { "View".to_string() } else { name.clone() };
        let mut candidate = base.clone();
        let mut n = 2;
        while out.iter().any(|o| o.to_lowercase() == candidate.to_lowercase()) {
            let suffix = format!(" {n}");
            candidate = format!("{}{suffix}", base.chars().take(NAME_MAX - suffix.len()).collect::<String>());
            n += 1;
        }
        out.push(candidate);
    }
    out
}

fn export_names(names: &[String]) -> Vec<String> {
    let stems: Vec<String> = names.iter().map(|n| file_name(n)).map(|f| if ["notes", "chapters"].contains(&f.to_lowercase().as_str()) { format!("{f} view") } else { f }).collect();
    unique_names(&stems)
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) { format!("\"{}\"", text.replace('"', "\"\"")) } else { text.to_string() }
}

fn notes_text(notes: &[Note]) -> String {
    let mut out = String::from("seconds,kind,text,tags\n");
    for n in notes {
        out += &format!("{},{},{},{}\n", n.at_s, n.kind, csv_field(&n.text), csv_field(&n.tags.join(" ")));
    }
    out
}

fn vtt_time(seconds: f64) -> String {
    let ms = (seconds.max(0.0) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000)
}

fn vtt_text(cues: &[Cue]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for (i, cue) in cues.iter().enumerate() {
        out += &format!("{}\n{} --> {}\n{}\n\n", i + 1, vtt_time(cue.from_s), vtt_time(cue.to_s), cue.text.replace('\n', " "));
    }
    out
}

pub fn events_to_items(events: &[Value], set: &Set) -> Items {
    let start = set.start();
    let at = |us: i64| tenth((us - start) as f64 / 1e6).max(0.0);
    let (mut notes, mut chapters, mut ends, mut starts): (Vec<Note>, Vec<Cue>, Vec<f64>, Vec<f64>) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for e in events.iter().filter(|e| room::covers(set, e)) {
        let Some(local) = e["local_us"].as_i64() else { continue };
        let text = e["text"].as_str().unwrap_or_default().trim().to_string();
        let tags: Vec<String> = e["tags"].as_array().into_iter().flatten().filter_map(|t| t.as_str().map(String::from)).collect();
        let plugin = !e["plugin"].is_null();
        match (e["kind"].as_str().unwrap_or_default(), e["end_us"].as_i64()) {
            ("drawing" | "board", _) => {}
            (_, Some(end)) if e["file"].is_null() && !text.is_empty() && at(end) > at(local) => chapters.push(Cue { from_s: at(local), to_s: at(end), text }),
            (_, Some(_)) => {}
            (kind @ ("note" | "mark"), None) if !plugin && (kind == "mark" || !text.is_empty()) => notes.push(Note { at_s: at(local), kind: kind.into(), text, tags, by: e["author"].as_str().unwrap_or_default().into() }),
            (_, None) if room::match_end(e).is_some() => ends.push(at(local)),
            (_, None) if room::match_start(e).is_some() => starts.push(at(local)),
            _ => {}
        }
    }
    let matches: Vec<Cue> = room::match_chapters(&ends, &starts).into_iter().enumerate().map(|(i, (from_s, to_s))| Cue { from_s, to_s, text: format!("Match {}", i + 1) }).collect();
    chapters.extend(matches.iter().cloned());
    notes.sort_by(|a, b| a.at_s.total_cmp(&b.at_s));
    chapters.sort_by(|a, b| a.from_s.total_cmp(&b.from_s));
    Items { notes, chapters, matches }
}

fn feed(core: Arc<Core>, listener: TcpListener, pieces: Vec<Piece>) {
    let deadline = Instant::now() + CONNECT_WAIT;
    let Some(mut stream) = proc::accept(&listener, || Instant::now() < deadline && core.export.lock().unwrap().is_some()) else { return };
    let total = stream::size(&pieces);
    let mut at = 0;
    while at < total {
        let Ok(bytes) = stream::read(&pieces, at, FEED_CHUNK.min(total - at)) else { return };
        if stream.write_all(&bytes).is_err() {
            return;
        }
        at += bytes.len() as u64;
    }
}

fn listen(command: &mut Command, inputs: impl IntoIterator<Item = Vec<Piece>>) -> Result<Vec<(TcpListener, Vec<Piece>)>, String> {
    inputs
        .into_iter()
        .map(|pieces| {
            let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
            command.arg("-i").arg(format!("tcp://127.0.0.1:{}", listener.local_addr().map_err(|e| e.to_string())?.port()));
            Ok((listener, pieces))
        })
        .collect()
}

fn serve(core: &Arc<Core>, feeds: Vec<(TcpListener, Vec<Piece>)>) {
    for (listener, pieces) in feeds {
        let core = core.clone();
        std::thread::spawn(move || feed(core, listener, pieces));
    }
}

fn stderr_tail(stderr: ChildStderr, n: usize) -> JoinHandle<String> {
    std::thread::spawn(move || {
        let mut tail = VecDeque::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if tail.len() == n {
                tail.pop_front();
            }
            tail.push_back(line);
        }
        Vec::from(tail).join("\n")
    })
}

fn read_progress(stdout: ChildStdout, mut seconds: impl FnMut(f64)) {
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.parse::<f64>().ok()) {
            seconds(us / 1e6);
        }
    }
}

fn tracks(core: &Core, room_id: &str, set: &Set, f: &File) -> (Vec<Captured>, (u32, u32), f64) {
    let (mut tracks, mut size, mut kbps) = (Vec::new(), (0, 0), 0f64);
    for capture in f.captures() {
        if let Some(cap) = f.caps.get(capture) {
            let side = |key: &str| cap[key].as_u64().unwrap_or(0) as u32;
            size = (size.0.max(side("width")), size.1.max(side("height")));
            kbps = kbps.max(source_kbps(cap));
        }
        for series in SERIES {
            let mut segs = f.segs_of(capture, series);
            let Some(first) = segs.next() else { continue };
            let end_us = segs.fold(first.t_us + first.d_us, |end, s| end.max(s.t_us + s.d_us));
            let pieces = stream::set_pieces(core, room_id, f, capture, series);
            if pieces.len() > 1 {
                let seconds = |us: i64| (us - f.offset_us - set.start()).max(0) as f64 / 1e6;
                let video = series == SERIES[0];
                let audio = !video || has_sound(&pieces[0]);
                let segs: HashMap<u32, (f64, f64)> = f.segs_of(capture, series).map(|s| (s.n, (seconds(s.t_us), seconds(s.t_us + s.d_us)))).collect();
                let mut last = seconds(first.t_us);
                let times = pieces[1..]
                    .iter()
                    .map(|p| {
                        let span = p.path.file_name().and_then(|n| room::seg_number(&n.to_string_lossy())).and_then(|n| segs.get(&n).copied()).unwrap_or((last, last));
                        last = span.1;
                        span
                    })
                    .collect();
                tracks.push(Captured { input: Input { delay_s: seconds(first.t_us), end_s: seconds(end_us), video, audio }, pieces, times });
            }
        }
    }
    (tracks, size, kbps)
}

fn emit(core: &Core, percent: f64, done: bool, error: Option<String>) {
    let _ = core.app.emit("export", json!({"percent": tenth(percent), "done": done, "error": error}));
}

pub fn check(core: &Core) -> Result<(), String> {
    if core.any_recording() {
        return Err("Stop recording before exporting.".into());
    }
    if core.export.lock().unwrap().is_some() {
        return Err("An export is already running.".into());
    }
    ffmpeg::ready()
}

fn finish(partial: &Path, path: &Path, keep_going: impl Fn() -> bool) -> std::io::Result<()> {
    if std::fs::rename(partial, path).is_ok() {
        return Ok(());
    }
    let part = part_of(path);
    let copied = std::fs::copy(partial, &part).and_then(|_| if keep_going() { std::fs::rename(&part, path) } else { Err(std::io::ErrorKind::Interrupted.into()) });
    if copied.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    copied
}

fn cancelled(core: &Core) -> bool {
    core.export.lock().unwrap().as_ref().is_some_and(|r| r.cancelled)
}

fn wait(core: &Core) -> bool {
    loop {
        match core.export.lock().unwrap().as_mut().map(|r| r.child.child.try_wait()) {
            Some(Ok(Some(status))) => return status.success(),
            Some(Ok(None)) => {}
            None | Some(Err(_)) => return false,
        }
        std::thread::sleep(POLL);
    }
}

fn outcome(core: &Core, tail: JoinHandle<String>, partial: &Path, path: &Path) -> Result<(), String> {
    let ok = wait(core);
    let text = tail.join().unwrap_or_default();
    let result = if cancelled(core) {
        Err("Export cancelled".to_string())
    } else if ok {
        finish(partial, path, || !cancelled(core)).map_err(|e| if cancelled(core) { "Export cancelled".to_string() } else { format!("Export failed: {e}") })
    } else {
        tracing::warn!("export failed: {text}");
        Err(format!("Export failed: {}", text.lines().last().unwrap_or("ffmpeg stopped")))
    };
    let _ = std::fs::remove_file(partial);
    result
}

fn begin(core: &Core, set: (String, String), command: &mut Command, replace: bool) -> Result<(ChildStdout, ChildStderr), String> {
    let mut child = proc::spawn(command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()), BELOW_NORMAL).map_err(|e| e.to_string())?;
    let pipes = (child.child.stdout.take().unwrap(), child.child.stderr.take().unwrap());
    let mut slot = core.export.lock().unwrap();
    match slot.as_mut() {
        Some(running) if replace => running.child = child,
        Some(_) => {
            child.kill();
            return Err("An export is already running.".into());
        }
        None => *slot = Some(Running { set, child, cancelled: false }),
    }
    Ok(pipes)
}

fn end(core: &Core) {
    *core.export.lock().unwrap() = None;
}

pub fn cancel(core: &Core) {
    if let Some(running) = core.export.lock().unwrap().as_mut() {
        running.cancelled = true;
        running.child.kill();
    }
}

fn has_sound(piece: &Piece) -> bool {
    std::fs::read(&piece.path).is_ok_and(|bytes| bytes.windows(4).any(|w| w == b"soun"))
}

pub fn view_outputs(core: &Core, room_id: &str, set: &Set, f: &File) -> Option<Output> {
    let (tracks, (w, h), kbps) = tracks(core, room_id, set, f);
    if tracks.is_empty() {
        return None;
    }
    let (video, voice): (Vec<Captured>, Vec<Captured>) = tracks.into_iter().partition(|c| c.input.video);
    let size = (w.max(DEFAULT_SIZE.0) & !1, h.max(DEFAULT_SIZE.1) & !1);
    Some(Output { name: room::display_name(core, &f.owner), owner: f.owner.clone(), video, voice, size, kbps })
}

pub fn encode_args(graph: &str, mapped_audio: bool, video: Option<Vec<String>>, total: f64) -> Vec<String> {
    let mut args: Vec<String> = vec!["-filter_complex".into(), graph.into()];
    if video.is_some() {
        args.extend(["-map", "[vout]"].map(String::from));
    }
    if mapped_audio {
        args.extend(["-map", "[mix]"].map(String::from));
    }
    args.extend(video.into_iter().flatten());
    if mapped_audio {
        args.extend(["-c:a".to_string(), "aac".to_string(), "-b:a".to_string(), format!("{AUDIO_KBPS}k")]);
    }
    args.extend(["-t".to_string(), format!("{total:.3}")]);
    args
}

fn run_job(core: &Arc<Core>, set: &(String, String), job: Job, work: &Path, done: usize, jobs: usize, owned: &mut bool) -> Result<(), String> {
    let mut command = ffmpeg::command("error");
    command.args(["-nostats", "-progress", "pipe:1"]).current_dir(work);
    let feeds = listen(&mut command, job.feeds)?;
    command.args(&job.args).args(["-movflags", "+faststart", "-f", "mp4"]).arg(&job.partial);
    let (stdout, stderr) = begin(core, set.clone(), &mut command, *owned)?;
    *owned = true;
    serve(core, feeds);
    let tail = stderr_tail(stderr, ERROR_LINES);
    read_progress(stdout, |s| emit(core, ((done as f64 + (s / job.total.max(1.0)).clamp(0.0, 1.0)) / jobs as f64) * 100.0, false, None));
    outcome(core, tail, &job.partial, &job.path)
}

fn job(out: &Output, video: bool, path: PathBuf, partial: PathBuf, take: Take) -> Job {
    let total = take.total;
    let captured = if video { &out.video } else { &out.voice };
    let inputs: Vec<Input> = captured.iter().map(|c| c.input.clone()).collect();
    let args = if take.whole && !(video && take.quality == Quality::Light) && copyable(&inputs, total) {
        ["-map", "0", "-c", "copy"].map(String::from).to_vec()
    } else if video {
        let chosen = ffmpeg::pick(&crate::default_codecs());
        let source = if out.kbps > 0.0 { out.kbps } else { record::bitrate_kbps(out.size.0, 1.0) as f64 };
        let (size, kbps) = picture(take.quality, out.size, source, chosen.2);
        let (graph, audio) = graph(&inputs, size, total);
        encode_args(&graph, audio, Some(ffmpeg::video_args(chosen.0, kbps)), total)
    } else {
        encode_args(&voice_graph(&inputs, total), true, None, total)
    };
    Job { feeds: captured.iter().map(|c| c.pieces.clone()).collect(), args, partial, path, total }
}

struct Part {
    out: Output,
    stem: String,
    from: f64,
    to: f64,
}

pub fn run_all(core: &Arc<Core>, key: (String, String), jobs: Vec<Job>, work: &Path, dir: &Path) -> Result<(), String> {
    let count = jobs.len();
    let mut owned = false;
    let failure = jobs.into_iter().enumerate().try_for_each(|(n, job)| run_job(core, &key, job, work, n, count, &mut owned)).err();
    if owned {
        end(core);
    }
    let Some(e) = failure else { return Ok(()) };
    let _ = std::fs::remove_dir_all(dir);
    if owned {
        emit(core, 0.0, true, Some(e.clone()));
    }
    Err(e)
}

pub fn emit_done(core: &Core, dir: &Path) {
    let _ = core.app.emit("export", json!({"percent": 100.0, "done": true, "error": null, "folder": dir.to_string_lossy()}));
}

fn parts(outputs: &[Output], names: &[String], matches: &[Cue], range: Range, total: f64) -> Vec<Part> {
    let pairs = || outputs.iter().zip(names);
    match range {
        Range::Whole => pairs().map(|(out, name)| Part { out: out.clone(), stem: name.clone(), from: 0.0, to: total }).collect(),
        Range::Matches => matches
            .iter()
            .enumerate()
            .flat_map(|(n, m)| pairs().filter_map(move |(out, name)| within(out, m.from_s, m.to_s).map(|out| Part { out, stem: match_stem(n + 1, m.from_s, name), from: m.from_s, to: m.to_s })))
            .collect(),
    }
}

pub fn folder(core: &Arc<Core>, room_id: &str, set_id: &str, dir: &Path, opts: &Options) -> Result<Value, String> {
    check(core)?;
    let logs = room::logs(core, room_id);
    let set = room::sets(core, room_id, &logs).into_iter().find(|s| s.id == set_id).ok_or("Unknown recording.")?;
    let total = (set.end_shown().ok_or("This recording is still being recorded.")? - set.start()) as f64 / 1e6;
    if dir.is_file() || std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()) {
        return Err("Choose a new or empty folder.".into());
    }
    if opts.kind == Kind::DebriefVideo {
        return crate::debrief::video(core, room_id, &logs, &set, total, dir, opts);
    }
    let files: Vec<&File> = set.files.iter().filter(|f| opts.member.as_deref().is_none_or(|m| m == f.owner)).collect();
    if files.is_empty() {
        return Err("That member is not in this recording.".into());
    }
    let outputs: Vec<Output> = files.iter().filter_map(|f| view_outputs(core, room_id, &set, f)).collect();
    if outputs.is_empty() {
        return Err("None of this recording's files are on this PC yet.".into());
    }
    let skipped: Vec<String> = files.iter().filter(|f| !outputs.iter().any(|o| o.owner == f.owner)).map(|f| room::display_name(core, &f.owner)).collect();
    let items = events_to_items(&room::events(&logs, None), &set);
    let names = export_names(&unique_names(&outputs.iter().map(|o| o.name.clone()).collect::<Vec<_>>()));
    let parts = parts(&outputs, &names, &items.matches, opts.range, total);
    if parts.is_empty() {
        return Err("This recording has no matches.".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("The folder cannot be made: {e}"))?;
    let work = core.dir.join("export");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let mut jobs = Vec::new();
    for part in &parts {
        let take = Take { total: part.to - part.from, quality: opts.quality, whole: opts.range == Range::Whole };
        let has_video = !part.out.video.is_empty();
        let ext = if has_video { "mp4" } else { "m4a" };
        jobs.push(job(&part.out, has_video, dir.join(format!("{}.{ext}", part.stem)), work.join(format!("part-{}.{ext}", jobs.len())), take));
        if has_video && !part.out.voice.is_empty() {
            jobs.push(job(&part.out, false, dir.join(format!("{} voice.m4a", part.stem)), work.join(format!("part-{}.m4a", jobs.len())), take));
        }
    }
    tracing::info!("folder export of {set_id} started: {} files", jobs.len());
    run_all(core, (room_id.to_string(), set_id.to_string()), jobs, &work, dir)?;
    let write = |name: &str, text: String| std::fs::write(dir.join(name), text).map_err(|e| format!("Export failed: {e}"));
    if !items.notes.is_empty() {
        write(NOTES, notes_text(&items.notes))?;
    }
    if !items.chapters.is_empty() {
        write(CHAPTERS, vtt_text(&items.chapters))?;
    }
    if let Some(records) = crate::party::records(&logs).remove(set_id) {
        write(WATCH, crate::party::watch_json(&records).to_string())?;
    }
    emit_done(core, dir);
    Ok(json!({"folder": dir.to_string_lossy(), "files": outputs.len(), "skipped": skipped}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(delay_s: f64, video: bool, audio: bool) -> Input {
        Input { delay_s, end_s: 1e6, video, audio }
    }

    #[test]
    fn one_input_without_delay_is_copied_and_the_rest_is_filtered() {
        assert!(copyable(&[input(0.0, true, true)], 120.0));
        assert!(copyable(&[input(0.04, true, true)], 120.0));
        assert!(!copyable(&[input(0.5, true, true)], 120.0));
        assert!(!copyable(&[input(0.0, true, true), input(60.0, true, true)], 120.0));
        assert!(!copyable(&[Input { end_s: 60.0, ..input(0.0, true, true) }], 120.0));
        assert!(!copyable(&[], 120.0));
    }

    #[test]
    fn the_filter_places_each_capture_on_the_timeline() {
        let (g, audio) = graph(&[input(0.0, true, true), input(61.5, true, false)], (1280, 720), 120.0);
        assert!(audio);
        assert!(g.starts_with("color=c=black:s=1280x720:r=30:d=120.000[bg0];"));
        assert!(g.contains("[0:v]setpts=PTS-STARTPTS+0.000/TB"));
        assert!(g.contains("[1:v]setpts=PTS-STARTPTS+61.500/TB"));
        assert!(g.contains("[bg1][v1]overlay=eof_action=pass[bg2];"));
        assert!(g.contains("[0:a:0]asetpts=PTS-STARTPTS,adelay=delays=0:all=1[s0];"));
        assert!(!g.contains("[1:a:0]"));
        assert!(g.ends_with("[bg2]format=yuv420p[vout];[s0]amix=inputs=1:normalize=0:duration=longest,apad=whole_dur=120.000[mix]"));
        let (silent, audio) = graph(&[input(2.0, true, false)], (640, 360), 10.0);
        assert!(!audio && silent.ends_with("[bg1]format=yuv420p[vout]"));
        assert_eq!(voice_graph(&[input(1.25, false, true), input(9.0, false, true)], 30.0), "[0:a:0]asetpts=PTS-STARTPTS,adelay=delays=1250:all=1[s0];[1:a:0]asetpts=PTS-STARTPTS,adelay=delays=9000:all=1[s1];[s0][s1]amix=inputs=2:normalize=0:duration=longest,apad=whole_dur=30.000[mix]");
    }

    #[test]
    fn the_encode_arguments_map_only_what_exists() {
        let video = || Some(ffmpeg::video_args("libx264", 3000));
        let a = encode_args("g", true, video(), 12.0).join(" ");
        assert!(a.contains("-filter_complex g -map [vout] -map [mix] -c:v ") && a.contains("-b:v 3000k") && a.contains("-c:a aac -b:a 128k") && a.ends_with("-t 12.000"));
        let v = encode_args("g", false, video(), 12.0).join(" ");
        assert!(!v.contains("[mix]") && !v.contains("-c:a"));
        let m = encode_args("g", true, None, 12.0).join(" ");
        assert!(m.contains("-map [mix]") && !m.contains("-c:v") && !m.contains("[vout]"));
    }

    fn captured(delay_s: f64, video: bool) -> Captured {
        Captured { input: input(delay_s, video, !video), pieces: vec![Piece { path: PathBuf::from("a"), len: 1 }, Piece { path: PathBuf::from("b"), len: 1 }], times: vec![(delay_s, 1e6)] }
    }

    const WHOLE: Take = Take { total: 120.0, quality: Quality::Standard, whole: true };

    fn args(out: &Output, video: bool, take: Take) -> String {
        job(out, video, PathBuf::from("x"), PathBuf::from("p"), take).args.join(" ")
    }

    #[test]
    fn the_folder_export_always_starts_every_file_at_the_recording_start() {
        let late = Output { name: "Ann".into(), owner: "a".into(), video: vec![captured(61.5, true)], voice: vec![captured(61.5, false)], size: (1280, 720), kbps: 3000.0 };
        let video = args(&late, true, WHOLE);
        assert!(video.contains("color=c=black:s=1280x720:r=30:d=120.000") && video.contains("setpts=PTS-STARTPTS+61.500/TB") && video.ends_with("-t 120.000"));
        let voice = args(&late, false, WHOLE);
        assert!(voice.contains("adelay=delays=61500:all=1") && voice.ends_with("-t 120.000"));
        let first = Output { video: vec![captured(0.0, true)], ..late };
        assert_eq!(args(&first, true, WHOLE), "-map 0 -c copy");
        assert_eq!(args(&first, true, Take { quality: Quality::Original, ..WHOLE }), "-map 0 -c copy");
        let light = args(&first, true, Take { quality: Quality::Light, ..WHOLE });
        assert!(light.contains("color=c=black:s=1280x720") && light.contains("-b:v "));
        assert!(args(&first, true, Take { whole: false, ..WHOLE }).contains("-filter_complex"));
        let early = Output { video: vec![Captured { input: Input { end_s: 60.0, ..input(0.0, true, true) }, ..captured(0.0, true) }], ..first };
        let padded = args(&early, true, WHOLE);
        assert!(padded.contains("color=c=black:s=1280x720:r=30:d=120.000") && padded.contains("duration=longest,apad=whole_dur=120.000[mix]") && padded.ends_with("-t 120.000"));
    }

    fn segmented(video: bool) -> Captured {
        let piece = |n: &str| Piece { path: PathBuf::from(n), len: 1 };
        Captured { input: input(10.0, video, !video), pieces: vec![piece("init"), piece("1"), piece("2"), piece("3")], times: vec![(10.0, 20.0), (20.0, 30.0), (30.0, 40.0)] }
    }

    #[test]
    fn a_match_takes_only_its_segments_and_cuts_into_the_first() {
        let c = clip(&segmented(true), 25.0, 32.0).unwrap();
        assert_eq!(c.pieces.iter().map(|p| p.path.to_string_lossy().into_owned()).collect::<Vec<_>>(), ["init", "2", "3"]);
        assert_eq!((c.input.delay_s, c.input.end_s), (-5.0, 15.0));
        assert!(clip(&segmented(true), 40.0, 50.0).is_none());
        let (g, _) = graph(&[c.input], (1280, 720), 7.0);
        assert!(g.contains("[0:v]setpts=PTS-STARTPTS,trim=start=5.000,setpts=PTS-STARTPTS+0.000/TB"));
        assert!(voice_graph(&[clip(&segmented(false), 25.0, 32.0).unwrap().input], 7.0).starts_with("[0:a:0]asetpts=PTS-STARTPTS,atrim=start=5.000,asetpts=PTS-STARTPTS,adelay=delays=0:all=1[s0];"));
        let late = clip(&segmented(true), 5.0, 32.0).unwrap();
        assert_eq!((late.input.delay_s, late.pieces.len()), (5.0, 4));
    }

    #[test]
    fn per_match_files_are_named_by_match_number_and_start_and_never_copied() {
        let out = Output { name: "Ann".into(), owner: "a".into(), video: vec![segmented(true)], voice: Vec::new(), size: (1920, 1080), kbps: 6000.0 };
        let other = Output { name: "Ben".into(), owner: "b".into(), video: vec![Captured { times: vec![(50.0, 60.0)], ..segmented(true) }], ..out.clone() };
        let matches = [Cue { from_s: 0.0, to_s: 25.0, text: "Match 1".into() }, Cue { from_s: 3725.0, to_s: 3800.0, text: "Match 2".into() }, Cue { from_s: 25.0, to_s: 55.0, text: "Match 3".into() }];
        let names = ["Ann".to_string(), "Ben".to_string()];
        let each = parts(&[out.clone(), other.clone()], &names, &matches, Range::Matches, 4000.0);
        assert_eq!(each.iter().map(|p| (p.stem.as_str(), p.from, p.to)).collect::<Vec<_>>(), [("Match 01 0-00-00 Ann", 0.0, 25.0), ("Match 03 0-00-25 Ann", 25.0, 55.0), ("Match 03 0-00-25 Ben", 25.0, 55.0)]);
        let a = args(&each[0].out, true, Take { total: 25.0, quality: Quality::Original, whole: false });
        assert!(a.contains("d=25.000") && a.contains("setpts=PTS-STARTPTS+10.000/TB") && a.ends_with("-t 25.000"));
        let whole = parts(&[out, other], &names, &matches, Range::Whole, 4000.0);
        assert_eq!(whole.iter().map(|p| (p.stem.as_str(), p.from, p.to)).collect::<Vec<_>>(), [("Ann", 0.0, 4000.0), ("Ben", 0.0, 4000.0)]);
    }

    #[test]
    fn quality_sets_the_picture_size_and_bitrate() {
        let standard = picture(Quality::Standard, (1920, 1080), 6000.0, 1.0);
        assert_eq!(standard, ((1920, 1080), export_kbps(&[6000.0], (1920, 1080), 1.0)));
        let (size, kbps) = picture(Quality::Light, (1920, 1080), 6000.0, 1.0);
        assert_eq!(size, (1280, 720));
        assert_eq!(kbps, (export_kbps(&[6000.0], (1280, 720), 1.0) as f64 * LIGHT).round() as u32);
        assert!(kbps <= standard.1 / 2);
        assert_eq!(picture(Quality::Light, (1280, 1024), 3000.0, 1.0).0, (900, 720));
        assert_eq!(picture(Quality::Original, (2560, 1440), 90000.0, 0.55), ((2560, 1440), 49500));
    }

    fn fake_set() -> Set {
        Set { id: "s1".into(), room: "r".into(), files: vec![File { set: "s1".into(), owner: "a".into(), start_us: 1_000_000_000, end_us: Some(1_100_000_000), offset_us: 0, segs: Vec::new(), caps: HashMap::new(), view: None, present: false }], stopped: false }
    }

    fn event(kind: &str, local: i64, end: Option<i64>, text: &str, tags: Value, plugin: bool) -> Value {
        json!({"kind": kind, "local_us": local, "end_us": end, "text": text, "tags": tags, "plugin": if plugin { json!({"id": "p"}) } else { Value::Null }})
    }

    #[test]
    fn events_become_notes_and_chapters_on_the_recording_clock() {
        let set = fake_set();
        let s = 1_000_000_000i64;
        let events = vec![
            event("note", s + 84_500_000, None, "Late rotate #rotate", json!(["rotate"]), false),
            event("mark", s + 90_000_000, None, "", json!([]), false),
            event("chapter", s, Some(s + 30_000_000), "Warm up", json!([]), false),
            event("match_end", s + 60_000_000, None, "", json!(["match"]), true),
            event("match_end", s + 30_000_000, None, "", json!(["match"]), true),
            event("match_end", s + 30_000_000, None, "", json!(["match"]), true),
            event("drawing", s + 7_000_000, None, "", json!([]), false),
            event("note", s + 500_000_000, None, "Outside", json!([]), false),
            event("note", s + 1_000_000, None, "Plugin note", json!([]), true),
        ];
        let Items { notes, chapters, matches } = events_to_items(&events, &set);
        assert_eq!(matches.iter().map(|m| (m.from_s, m.to_s)).collect::<Vec<_>>(), [(0.0, 30.0), (30.0, 60.0)]);
        let notes: Vec<(f64, &str, &str)> = notes.iter().map(|n| (n.at_s, n.kind.as_str(), n.text.as_str())).collect();
        assert_eq!(notes, [(84.5, "note", "Late rotate #rotate"), (90.0, "mark", "")]);
        let chapters: Vec<(f64, f64, &str)> = chapters.iter().map(|c| (c.from_s, c.to_s, c.text.as_str())).collect();
        assert_eq!(chapters, [(0.0, 30.0, "Warm up"), (0.0, 30.0, "Match 1"), (30.0, 60.0, "Match 2")]);
    }

    #[test]
    fn a_match_found_to_start_runs_from_its_start() {
        let s = 1_000_000_000i64;
        let at = |secs: i64| s + secs * 1_000_000;
        let events = vec![event("match_start", at(40), None, "", json!(["match", "start"]), true), event("match_end", at(300), None, "", json!(["match"]), true), event("match_end", at(500), None, "", json!(["match"]), true)];
        let mut set = fake_set();
        set.files[0].end_us = Some(at(600));
        let matches = events_to_items(&events, &set).matches;
        assert_eq!(matches.iter().map(|m| (m.from_s, m.to_s, m.text.as_str())).collect::<Vec<_>>(), [(40.0, 300.0, "Match 1"), (300.0, 500.0, "Match 2")]);
    }

    #[test]
    fn the_export_is_sized_and_paced_by_its_sources() {
        assert_eq!(source_kbps(&json!({"width": 1280, "kbps": 3000, "codec": "av1"})).round(), (3000.0 / 0.55_f64).round());
        assert_eq!(source_kbps(&json!({"width": 1280})), record::bitrate_kbps(1280, 1.0) as f64);
        assert_eq!(export_kbps(&[5000.0], (1280, 720), 1.0), 5000);
        assert_eq!(export_kbps(&[5000.0], (1280, 720), 0.55), 2750);
        assert_eq!(export_kbps(&[90000.0], (1280, 720), 1.0), record::bitrate_kbps(1280, 1.0));
    }

    #[test]
    fn file_names_are_safe_and_distinct() {
        assert_eq!(file_name("Ann/Ben: \"x\"?"), "Ann_Ben_ _x__");
        assert_eq!(file_name("..."), "view");
        assert_eq!(unique_names(&["Ann".into(), "ann".into(), "".into(), "Ann".into()]), ["Ann", "ann 2", "View", "Ann 3"]);
    }

    #[test]
    fn folder_names_are_safe_on_windows() {
        assert_eq!(safe_name("CON", 80, "x"), "_CON");
        assert_eq!(safe_name("nul.txt", 80, "x"), "_nul.txt");
        assert_eq!(safe_name("Com1 ", 80, "x"), "_Com1");
        assert_eq!(safe_name("CONSOLE", 80, "x"), "CONSOLE");
        assert_eq!(safe_name("Scrim. . ", 80, "x"), "Scrim");
        assert_eq!(safe_name("  a<b>|c  ", 80, "x"), "a_b__c");
        assert_eq!(safe_name("...", 80, "x"), "x");
        assert_eq!(safe_name(&"あ".repeat(100), 80, "x"), "あ".repeat(80));
        assert_eq!(safe_name(&format!("{}.  b", "a".repeat(78)), 80, "x"), "a".repeat(78));
        assert_eq!(folder_name("練習部屋", "2026-10-23 14-05-33", None, None), "2026-10-23 14-05 練習部屋");
        assert_eq!(folder_name("Team", "2026-10-23 14-05-33", Some(" Final: G1 "), None), "Final_ G1 Team");
        assert_eq!(folder_name("Team", "2026-10-23 14-05-33", None, Some(754.0)), "Team debrief 2026-10-23 14-05 13 min");
        assert_eq!(folder_name("", "2026-10-23 14-05-33", None, Some(5.0)), "debrief 2026-10-23 14-05 1 min");
        assert_eq!(folder_name("CON", "", Some(""), None), "_CON");
    }

    #[test]
    fn a_taken_folder_name_gets_a_number() {
        assert_eq!(unused_name("Scrim", |_| false), "Scrim");
        assert_eq!(unused_name("Scrim", |n| ["Scrim", "Scrim (2)"].contains(&n)), "Scrim (3)");
        let long = "あ".repeat(80);
        let next = unused_name(&long, |n| n == long);
        assert_eq!((next.chars().count(), next.ends_with(" (2)")), (80, true));
    }

    #[test]
    fn exported_file_names_never_meet_the_files_beside_them() {
        let names: Vec<String> = ["Ann/Ben", "Chapters", "notes", "ann_ben", "Session"].map(String::from).to_vec();
        assert_eq!(export_names(&names), ["Ann_Ben", "Chapters view", "notes view", "ann_ben 2", "Session"]);
    }

    #[test]
    fn notes_are_written_as_csv() {
        let notes = vec![
            Note { at_s: 84.5, kind: "note".into(), text: "Late, \"very\" late\nrotate".into(), tags: vec!["mistake".into(), "rotate".into()], by: "a".into() },
            Note { at_s: 90.0, kind: "mark".into(), text: String::new(), tags: Vec::new(), by: "a".into() },
        ];
        assert_eq!(notes_text(&notes), "seconds,kind,text,tags\n84.5,note,\"Late, \"\"very\"\" late\nrotate\",mistake rotate\n90,mark,,\n");
    }

    #[test]
    fn webvtt_is_written_with_numbered_cues() {
        let cues = vec![Cue { from_s: 0.0, to_s: 612.3, text: "Match 1".into() }, Cue { from_s: 3725.5, to_s: 3730.0, text: "Drop\nnorth".into() }];
        assert_eq!(vtt_text(&cues), "WEBVTT\n\n1\n00:00:00.000 --> 00:10:12.300\nMatch 1\n\n2\n01:02:05.500 --> 01:02:10.000\nDrop north\n\n");
    }

    #[test]
    fn finishing_replaces_the_destination_and_leaves_no_part() {
        let dir = std::env::temp_dir().join(format!("ezh-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("O'Brien ドキュメント")).unwrap();
        let (partial, path) = (dir.join("export.mp4"), dir.join("O'Brien ドキュメント").join("set.mp4"));
        std::fs::write(&path, b"old").unwrap();
        std::fs::write(&partial, b"new").unwrap();
        finish(&partial, &path, || true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(!partial.exists() && !part_of(&path).exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}

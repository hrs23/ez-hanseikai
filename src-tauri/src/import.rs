use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::UNIX_EPOCH;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::Emitter;

use crate::ffmpeg::{self, FRAGMENTED};
use crate::proc::{self, BELOW_NORMAL};
use crate::record::{AUDIO_KBPS, FRAGMENTS_PER_SEGMENT};
use crate::room::{self, seg_name, INIT, SERIES};
use crate::session::{self, Cue, Item, Session};
use crate::{data, hex, media, now_us, write_whole, Core};

const FRAGMENT_US: &str = "1000000";
const PROBE_WAIT_LINES: usize = 400;
const ITEMS_MAX: usize = 5000;
const ERROR_LINES: usize = 12;
const KEYFRAME_EVERY_S: u32 = 1;
const CRF: &str = "20";
const COPY_AUDIO: [&str; 4] = ["aac", "mp3", "opus", "flac"];
const PLAIN_PIXELS: [&str; 2] = ["yuv420p", "yuvj420p"];

static RUNNING: Mutex<Option<String>> = Mutex::new(None);
static CANCELLED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Picture {
    pub codec: String,
    pub pixels: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Probe {
    pub duration_s: Option<f64>,
    pub picture: Option<Picture>,
    pub audio: Option<String>,
    pub created_ms: Option<i64>,
    pub kbps: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    pub copy_picture: bool,
    pub copy_audio: bool,
    pub tag_hevc: bool,
}

fn top_level_fields(text: &str) -> Vec<&str> {
    let (mut out, mut depth, mut from) = (Vec::new(), 0i32, 0);
    for (i, c) in text.char_indices() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                out.push(text[from..i].trim());
                from = i + 1;
            }
            _ => {}
        }
    }
    out.push(text[from..].trim());
    out
}

fn first_word(text: &str) -> String {
    text.split_whitespace().next().unwrap_or_default().to_string()
}

pub fn parse_probe(text: &str) -> Probe {
    let mut probe = Probe::default();
    let mut seen_stream = false;
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("Duration:") {
            let fields = top_level_fields(rest);
            probe.duration_s = fields.first().and_then(|d| session::parse_clock(d)).filter(|d| *d > 0.0);
            probe.kbps = fields.iter().find_map(|f| f.strip_prefix("bitrate:")).and_then(|b| first_word(b).parse().ok());
        } else if let Some(rest) = line.strip_prefix("creation_time") {
            if !seen_stream && probe.created_ms.is_none() {
                probe.created_ms = session::parse_iso(rest.trim_start_matches([' ', ':']));
            }
        } else if line.starts_with("Stream #") {
            seen_stream = true;
            if let Some((_, rest)) = line.split_once(": Video: ").filter(|_| !line.contains("(attached pic)")) {
                if probe.picture.is_none() {
                    let fields = top_level_fields(rest);
                    let size = fields.iter().find_map(|f| {
                        let (w, h) = f.split_whitespace().next()?.split_once('x')?;
                        Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))
                    });
                    let pixels = fields.get(1).map(|p| p.split('(').next().unwrap_or_default().trim().to_string()).unwrap_or_default();
                    probe.picture = Some(Picture { codec: first_word(fields[0]), pixels, width: size.map_or(0, |s| s.0), height: size.map_or(0, |s| s.1) });
                }
            } else if let Some((_, rest)) = line.split_once(": Audio: ") {
                probe.audio.get_or_insert_with(|| first_word(top_level_fields(rest)[0]));
            }
        }
    }
    probe
}

pub fn decide(probe: &Probe, usable: &[String]) -> Recipe {
    let copy_picture = probe.picture.as_ref().is_some_and(|p| match p.codec.as_str() {
        "h264" | "vp9" => PLAIN_PIXELS.contains(&p.pixels.as_str()),
        "hevc" => usable.iter().any(|c| c == "hevc") && ["yuv420p", "yuv420p10le"].contains(&p.pixels.as_str()),
        "av1" => usable.iter().any(|c| c == "av1") && p.pixels == "yuv420p",
        _ => false,
    });
    Recipe { copy_picture, copy_audio: probe.audio.as_ref().is_some_and(|a| COPY_AUDIO.contains(&a.as_str())), tag_hevc: copy_picture && probe.picture.as_ref().is_some_and(|p| p.codec == "hevc") }
}

pub fn convert_args(input: &str, probe: &Probe, recipe: &Recipe, series: &str) -> Vec<String> {
    let mut args: Vec<String> = ["-hide_banner", "-loglevel", "error", "-nostdin", "-nostats", "-y", "-i", input, "-sn", "-dn", "-map_metadata", "-1"].map(String::from).to_vec();
    if series == SERIES[0] {
        args.extend(["-map", "0:V:0"].map(String::from));
        if probe.audio.is_some() {
            args.extend(["-map", "0:a:0"].map(String::from));
        }
        if recipe.copy_picture {
            args.extend(["-c:v", "copy"].map(String::from));
            if recipe.tag_hevc {
                args.extend(["-tag:v", "hvc1"].map(String::from));
            }
        } else {
            args.extend(["-c:v", "libx264", "-preset", "veryfast", "-crf", CRF, "-pix_fmt", "yuv420p", "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2"].map(String::from));
            args.extend(["-g".to_string(), (KEYFRAME_EVERY_S * 30).to_string(), "-force_key_frames".to_string(), format!("expr:gte(t,n_forced*{KEYFRAME_EVERY_S})")]);
        }
    } else {
        args.extend(["-map", "0:a:0", "-vn"].map(String::from));
    }
    if probe.audio.is_some() {
        if recipe.copy_audio {
            args.extend(["-c:a", "copy"].map(String::from));
        } else {
            args.extend(["-c:a".to_string(), "aac".to_string(), "-b:a".to_string(), format!("{AUDIO_KBPS}k")]);
        }
    }
    args.extend(["-avoid_negative_ts", "make_zero", "-movflags"].map(String::from));
    args.push(if series == SERIES[0] { FRAGMENTED.to_string() } else { "empty_moov+default_base_moof".to_string() });
    args.extend(["-frag_duration", FRAGMENT_US, "-f", "mp4", "pipe:1"].map(String::from));
    args
}

fn ffmpeg_info(path: &Path) -> Result<Probe, String> {
    let mut command = Command::new(ffmpeg::path());
    command.args(["-hide_banner", "-nostdin", "-i"]).arg(path).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let mut process = proc::spawn(&mut command, BELOW_NORMAL).map_err(|e| e.to_string())?;
    let mut text = String::new();
    if let Some(mut stderr) = process.child.stderr.take() {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        text = String::from_utf8_lossy(&bytes).to_string();
    }
    let _ = process.child.wait();
    Ok(parse_probe(&text.lines().take(PROBE_WAIT_LINES).collect::<Vec<_>>().join("\n")))
}

#[derive(Debug)]
pub struct PlanView {
    pub name: String,
    pub file: PathBuf,
    pub voice: Option<PathBuf>,
    pub offset_ms: i64,
    pub subtitles: Vec<Cue>,
}

#[derive(Debug)]
pub struct Plan {
    pub title: String,
    pub start_ms: Option<i64>,
    pub views: Vec<PlanView>,
    pub items: Vec<Item>,
}

fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{} cannot be read: {e}", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()))
}

fn folder_files(dir: &Path) -> Result<Vec<String>, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("The folder cannot be read: {e}"))?;
    Ok(entries.flatten().filter(|e| e.path().is_file()).map(|e| e.file_name().to_string_lossy().to_string()).collect())
}

fn folder_label(dir: &Path) -> String {
    dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

pub fn plan_from_folder(dir: &Path) -> Result<Plan, String> {
    let files = folder_files(dir)?;
    let find = |name: &str| files.iter().find(|f| f.eq_ignore_ascii_case(name)).map(|f| dir.join(f));
    let session = match find(session::FILE) {
        Some(path) => Some(session::parse_session(&read_text(&path)?)?),
        None => None,
    };
    let mut resolved = session::resolve(session, &folder_label(dir), &files)?;
    let mut items = std::mem::take(&mut resolved.items);
    if let Some(path) = find(session::NOTES) {
        items.extend(session::parse_notes(&read_text(&path)?)?);
    }
    if let Some(path) = find(session::CHAPTERS) {
        items.extend(session::chapter_items(session::parse_vtt(&read_text(&path)?)?));
    }
    plan_of(Some(dir), resolved, items)
}

fn plan_of(dir: Option<&Path>, session: Session, items: Vec<Item>) -> Result<Plan, String> {
    let mut views = Vec::new();
    for v in &session.views {
        let at = |name: &str| dir.map_or_else(|| PathBuf::from(name), |d| d.join(name));
        let file = at(&v.file);
        if !file.is_file() {
            return Err(format!("{} is missing.", v.file));
        }
        let voice = v.voice.as_ref().map(|name| at(name));
        if let Some(missing) = voice.as_ref().filter(|p| !p.is_file()) {
            return Err(format!("{} is missing.", missing.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        }
        let sidecar = file.with_extension("vtt");
        let subtitles = if sidecar.is_file() { session::parse_vtt(&read_text(&sidecar)?)? } else { Vec::new() };
        views.push(PlanView { name: v.name.clone(), file, voice, offset_ms: v.offset_ms, subtitles });
    }
    if items.len() > ITEMS_MAX {
        return Err("There are too many notes to import.".into());
    }
    Ok(Plan { title: session.title.unwrap_or_default(), start_ms: session.start_ms, views, items })
}

pub fn plan_from_files(files: &[PathBuf]) -> Result<Plan, String> {
    let names: Vec<String> = files.iter().map(|f| f.to_string_lossy().to_string()).collect();
    let mut session = Session { views: session::views_from_files(&names)?, ..Default::default() };
    session.title = match session.views.as_slice() {
        [only] => Some(session::stem(&only.file)),
        _ => files.first().and_then(|f| f.parent()).map(folder_label),
    }
    .map(|t| crate::clean_text(&t, session::TITLE_MAX))
    .filter(|t| !t.is_empty());
    plan_of(None, session, Vec::new())
}

pub fn plan_from_spec(spec: &Value) -> Result<Plan, String> {
    let folder = spec["folder"].as_str().filter(|f| !f.is_empty()).map(PathBuf::from);
    let files: Vec<PathBuf> = spec["files"].as_array().into_iter().flatten().filter_map(|f| f.as_str()).filter(|f| !f.is_empty()).map(PathBuf::from).collect();
    let mut plan = match (folder, files.as_slice()) {
        (Some(dir), _) if dir.is_dir() => plan_from_folder(&dir)?,
        (Some(_), _) => return Err("That folder does not exist.".into()),
        (None, [one]) if one.is_dir() => plan_from_folder(one)?,
        (None, [one]) if one.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(session::FILE)) => plan_from_folder(one.parent().unwrap_or(Path::new(".")))?,
        (None, []) => return Err("Choose a folder or video files.".into()),
        (None, many) => plan_from_files(many)?,
    };
    if let Some(title) = spec["title"].as_str().map(|t| crate::clean_text(t, session::TITLE_MAX)).filter(|t| !t.is_empty()) {
        plan.title = title;
    }
    if let Some(names) = spec["names"].as_array() {
        let wanted: Vec<String> = plan.views.iter().zip(names.iter().map(Some).chain(std::iter::repeat(None))).map(|(v, n)| n.and_then(|n| n.as_str()).map(crate::clean_name).filter(|n| !n.is_empty()).unwrap_or_else(|| v.name.clone())).collect();
        for (view, name) in plan.views.iter_mut().zip(session::unique_names(&wanted)) {
            view.name = name;
        }
    }
    Ok(plan)
}

pub fn plan_json(plan: &Plan) -> Value {
    json!({
        "title": plan.title, "start": plan.start_ms.map(session::iso), "notes": plan.items.len(),
        "views": plan.views.iter().map(|v| json!({"name": v.name, "file": v.file.file_name().map(|n| n.to_string_lossy().to_string()), "voice": v.voice.is_some(), "offset_ms": v.offset_ms})).collect::<Vec<_>>(),
    })
}

struct Made {
    init: (u64, String),
    segs: Vec<Value>,
    end_us: i64,
    width: u32,
    height: u32,
}

struct Imported {
    id: String,
    name: String,
    records: Vec<Value>,
}

struct Pending {
    n: u32,
    t_us: i64,
    size: u64,
    sha: String,
}

fn seg_record(capture: &str, series: &str, p: &Pending, set: &str, d_us: i64) -> Value {
    json!({"k": "seg", "capture": capture, "series": series, "n": p.n, "set": set, "t_us": p.t_us, "d_us": d_us, "size": p.size, "sha256": p.sha})
}

fn flush(dir: &Path, n: u32, fragments: &[Vec<u8>], t_us: i64) -> Result<Pending, String> {
    let path = dir.join(seg_name(n));
    let mut hasher = Sha256::new();
    let mut bytes = Vec::new();
    for f in fragments {
        hasher.update(f);
        bytes.extend_from_slice(f);
    }
    write_whole(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(Pending { n, t_us, size: bytes.len() as u64, sha: hex(&hasher.finalize()) })
}

fn convert(input: &Path, probe: &Probe, recipe: &Recipe, series: &str, dir: &Path, set: &str, capture: &str, start_us: i64, progress: &dyn Fn(f64)) -> Result<Made, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let args = convert_args(&input.to_string_lossy(), probe, recipe, series);
    let mut command = Command::new(ffmpeg::path());
    command.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut process = proc::spawn(&mut command, BELOW_NORMAL).map_err(|e| e.to_string())?;
    let (mut stdout, stderr) = (process.child.stdout.take().ok_or("ffmpeg did not start")?, process.child.stderr.take().ok_or("ffmpeg did not start")?);
    let errors = std::thread::spawn(move || {
        use std::io::BufRead;
        let mut tail = std::collections::VecDeque::new();
        for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
            if tail.len() == ERROR_LINES {
                tail.pop_front();
            }
            tail.push_back(line);
        }
        Vec::from(tail).join("\n")
    });
    let mut splitter = media::Splitter::default();
    let (mut scales, mut base_us, mut init) = (std::collections::HashMap::new(), None::<i64>, None);
    let (mut current, mut current_t, mut n, mut last_rel, mut step) = (Vec::<Vec<u8>>::new(), 0i64, 0u32, 0i64, 1_000_000i64);
    let mut pendings: Vec<Pending> = Vec::new();
    let mut failure = None;
    let mut buf = vec![0u8; 64 << 10];
    loop {
        if CANCELLED.load(Ordering::Relaxed) {
            failure = Some("Import cancelled".to_string());
            break;
        }
        let read = match stdout.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(k) => k,
        };
        let Some(outs) = splitter.push(&buf[..read]) else {
            failure = Some("The video could not be read.".to_string());
            break;
        };
        for out in outs {
            match out {
                media::Out::Init(bytes) => {
                    write_whole(&dir.join(INIT), &bytes).map_err(|e| e.to_string())?;
                    scales = media::timescales(&bytes);
                    init = Some((bytes.len() as u64, hex(&Sha256::digest(&bytes))));
                }
                media::Out::Fragment(bytes) => {
                    let Some(first) = media::fragment_times(&bytes, 0).into_iter().next() else { continue };
                    let micros = media::micros(&first, &scales);
                    let base = *base_us.get_or_insert(micros);
                    let rel = (micros - base).max(0);
                    step = if rel > last_rel { rel - last_rel } else { step };
                    last_rel = rel;
                    if current.len() as u32 >= FRAGMENTS_PER_SEGMENT {
                        n += 1;
                        pendings.push(flush(dir, n, &current, start_us + current_t)?);
                        current.clear();
                    }
                    if current.is_empty() {
                        current_t = rel;
                    }
                    current.push(bytes);
                    if let Some(total) = probe.duration_s {
                        progress((rel as f64 / 1e6 / total).clamp(0.0, 1.0));
                    }
                }
            }
        }
    }
    if failure.is_some() {
        process.kill();
    }
    let status = process.child.wait().map_err(|e| e.to_string());
    let text = errors.join().unwrap_or_default();
    if let Some(reason) = failure {
        return Err(reason);
    }
    if !status.is_ok_and(|s| s.success()) {
        return Err(format!("ffmpeg could not convert the file: {}", text.lines().last().unwrap_or("it stopped")));
    }
    if !current.is_empty() {
        n += 1;
        pendings.push(flush(dir, n, &current, start_us + current_t)?);
    }
    let Some(init) = init.filter(|_| !pendings.is_empty()) else { return Err("The file has no picture or sound.".to_string()) };
    let mut segs = Vec::new();
    for (i, p) in pendings.iter().enumerate() {
        let d_us = pendings.get(i + 1).map_or(start_us + last_rel + step - p.t_us, |next| next.t_us - p.t_us);
        segs.push(seg_record(capture, series, p, set, d_us));
    }
    let picture = probe.picture.clone().unwrap_or_default();
    Ok(Made { init, segs, end_us: start_us + last_rel + step, width: picture.width, height: picture.height })
}

fn file_start_ms(path: &Path, probe: &Probe) -> Option<i64> {
    probe.created_ms.or_else(|| {
        let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis() as i64;
        Some(modified - (probe.duration_s.unwrap_or(0.0) * 1000.0) as i64)
    })
}

fn tagged(text: &str, tags: &[String]) -> String {
    let mut words = vec![text.to_string()];
    for tag in tags {
        if !room::tags(text).iter().any(|t| t.to_lowercase() == tag.to_lowercase()) {
            words.push(format!("#{tag}"));
        }
    }
    words.into_iter().filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

pub fn item_records(items: &[Item], subtitles: &[(String, Vec<Cue>)], set: &str, start_us: i64, first_id: i64) -> Vec<Value> {
    let at = |s: f64| start_us + (s * 1e6).round() as i64;
    let mut out = Vec::new();
    let mut id = first_id;
    let mut next = || {
        id += 1;
        id
    };
    for item in items {
        let record = match item.kind.as_str() {
            "chapter" => room::user_event(next(), "chapter", &item.text, json!({"from_us": at(item.at_s), "to_us": at(item.to_s.unwrap_or(item.at_s)), "set": set}), Value::Null),
            "mark" => room::user_event(next(), "mark", &tagged(&item.text, &item.tags), json!({"t_us": at(item.at_s), "set": set}), Value::Null),
            _ => room::user_event(next(), "note", &tagged(&item.text, &item.tags), json!({"t_us": at(item.at_s), "set": set}), Value::Null),
        };
        out.push(record);
    }
    for (owner, cues) in subtitles {
        for cue in cues {
            out.push(room::user_event(next(), "speech", &cue.text, json!({"from_us": at(cue.from_s), "to_us": at(cue.to_s), "set": set, "file": owner}), Value::Null));
        }
    }
    out
}

fn emit(core: &Core, percent: f64, done: bool, error: Option<String>, title: &str) {
    let _ = core.app.emit("import", json!({"percent": (percent * 10.0).round() / 10.0, "done": done, "error": error, "title": title}));
}

pub fn busy() -> Option<String> {
    RUNNING.lock().unwrap().clone()
}

pub fn cancel() {
    CANCELLED.store(true, Ordering::Relaxed);
}

struct Guard;

impl Drop for Guard {
    fn drop(&mut self) {
        *RUNNING.lock().unwrap() = None;
    }
}

pub fn run(core: &Arc<Core>, spec: &Value) -> Result<Value, String> {
    ffmpeg::ready()?;
    let room_id = core.room_id().ok_or("Enter a room first.")?;
    if core.active_room().is_some_and(|r| r.invite.is_some()) {
        return Err("Enter a room first.".into());
    }
    let plan = plan_from_spec(spec)?;
    let _guard = {
        let mut running = RUNNING.lock().unwrap();
        if running.is_some() {
            return Err("An import is already running.".into());
        }
        *running = Some(room_id.clone());
        Guard
    };
    CANCELLED.store(false, Ordering::Relaxed);
    emit(core, 0.0, false, None, &plan.title);
    let result = import(core, &room_id, &plan);
    emit(core, 100.0, true, result.as_ref().err().cloned(), &plan.title);
    result
}

fn remove_views(recording: &Path, ids: &[String]) {
    for id in ids {
        let _ = std::fs::remove_dir_all(recording.join(id));
    }
    let _ = std::fs::remove_dir(recording);
}

fn import(core: &Arc<Core>, room_id: &str, plan: &Plan) -> Result<Value, String> {
    let me = core.me();
    let usable = crate::record::usable_codecs(core);
    let mut probes = Vec::new();
    for v in &plan.views {
        let info = ffmpeg_info(&v.file)?;
        if info.picture.is_none() && info.audio.is_none() {
            return Err(format!("{} is not a video or audio file.", v.file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
        }
        let voice = match &v.voice {
            Some(path) => Some(ffmpeg_info(path)?).filter(|p| p.audio.is_some()).ok_or_else(|| format!("{} has no sound.", path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()))?,
            None => Probe::default(),
        };
        probes.push((info, voice));
    }
    let base_ms = plan.start_ms.or_else(|| plan.views.iter().zip(&probes).filter_map(|(v, (p, _))| file_start_ms(&v.file, p)).min()).unwrap_or_else(|| now_us() / 1000);
    let start_us = base_ms * 1000;
    let set = room::unused_set(&room::logs(core, room_id), &core.room_dir(room_id), start_us, core.tz_offset_s);
    let units = plan.views.iter().zip(&probes).map(|(v, (p, _))| usize::from(p.picture.is_some()) + usize::from(v.voice.is_some() || p.picture.is_none())).sum::<usize>().max(1);
    let done = std::cell::Cell::new(0usize);
    let progress = |frac: f64| emit(core, (done.get() as f64 + frac) / units as f64 * 100.0, false, None, &plan.title);
    let ids: Vec<String> = plan.views.iter().map(|_| iroh::SecretKey::generate().public().to_string()).collect();
    let mut made: Vec<Imported> = Vec::new();
    let result = (|| -> Result<(), String> {
        for ((view, (info, voice)), id) in plan.views.iter().zip(&probes).zip(&ids) {
            let view_start = start_us + view.offset_ms * 1000;
            let capture = (view_start / 1000).to_string();
            let dir = |series| core.series_dir(room_id, Some(&set), id, &capture, series);
            let (mut init, mut segs, mut end_us) = (json!({}), Vec::new(), view_start);
            let (mut width, mut height) = (0, 0);
            let has_picture = info.picture.is_some();
            let recipe = decide(info, &usable);
            if has_picture {
                let m = convert(&view.file, info, &recipe, SERIES[0], &dir(SERIES[0]), &set, &capture, view_start, &progress)?;
                done.set(done.get() + 1);
                init[SERIES[0]] = json!({"size": m.init.0, "sha256": m.init.1});
                (width, height, end_us) = (m.width, m.height, end_us.max(m.end_us));
                segs.extend(m.segs);
            }
            let voice_source = match (&view.voice, has_picture) {
                (Some(path), _) => Some((path.as_path(), voice)),
                (None, false) => Some((view.file.as_path(), info)),
                (None, true) => None,
            };
            if let Some((path, probe)) = voice_source {
                let m = convert(path, probe, &decide(probe, &usable), SERIES[1], &dir(SERIES[1]), &set, &capture, view_start, &progress)?;
                done.set(done.get() + 1);
                init[SERIES[1]] = json!({"size": m.init.0, "sha256": m.init.1});
                end_us = end_us.max(m.end_us);
                segs.extend(m.segs);
            }
            let kbps = info.kbps.map(|k| json!(k)).unwrap_or(Value::Null);
            let codec = info.picture.as_ref().map(|p| if recipe.copy_picture { p.codec.clone() } else { "h264".to_string() }).unwrap_or_default();
            let cap = json!({"k": "cap", "room": room_id, "capture": capture, "start_us": view_start, "init": init, "source_kind": "import", "width": width, "height": height, "kbps": kbps, "codec": codec, "version": crate::VERSION});
            let rec = json!({"k": "rec", "room": room_id, "set": set, "start_us": view_start, "by": *core.name.lock().unwrap(), "voice": false, "imported": me, "view": view.name, "title": plan.title});
            let mut records = vec![rec, cap];
            records.extend(segs);
            records.push(json!({"k": "rec_end", "room": room_id, "set": set, "t_us": end_us, "stop": false}));
            made.push(Imported { id: id.clone(), name: view.name.clone(), records });
        }
        Ok(())
    })();
    if let Err(e) = result {
        remove_views(&core.recording_dir(room_id, &set), &ids);
        return Err(e);
    }
    for view in &made {
        let bytes: Vec<u8> = view.records.iter().flat_map(crate::log::encode).collect();
        crate::log::append_bytes(&core.log_path(room_id, &view.id), &bytes).map_err(|e| e.to_string())?;
    }
    data::forget(&core.files, &core.recording_dir(room_id, &set));
    {
        let mut rooms = core.rooms.lock().unwrap();
        if let Some(r) = rooms.list.iter_mut().find(|r| r.id == room_id) {
            for view in &made {
                r.removed.push(view.id.clone());
                r.names.insert(view.id.clone(), view.name.clone());
            }
        }
    }
    core.save_rooms();
    for view in &made {
        core.append_log(room_id, &json!({"k": "remove", "member": view.id, "t_us": now_us()}));
    }
    let subtitles: Vec<(String, Vec<Cue>)> = plan.views.iter().zip(&made).filter(|(v, _)| !v.subtitles.is_empty()).map(|(v, m)| (m.id.clone(), v.subtitles.clone())).collect();
    for record in item_records(&plan.items, &subtitles, &set, start_us, now_us()) {
        core.append_log(room_id, &record);
    }
    room::apply_removals(core);
    core.wake.notify_waiters();
    tracing::info!("imported {set}: {} views, {} notes", made.len(), plan.items.len());
    Ok(json!({"id": set, "views": made.len(), "notes": plan.items.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'a.mp4':
  Metadata:
    major_brand     : isom
    creation_time   : 2026-10-19T12:30:00.000000Z
  Duration: 00:01:10.50, start: 0.000000, bitrate: 2500 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 1280x720 [SAR 1:1 DAR 16:9], 2300 kb/s, 30 fps, 30 tbr, 15360 tbn (default)
    Metadata:
      creation_time   : 2020-01-01T00:00:00.000000Z
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706d), 48000 Hz, stereo, fltp, 128 kb/s (default)
  Stream #0:2: Video: mjpeg (Baseline), yuvj420p(pc, bt470bg/unknown/unknown), 300x300, 90k tbr, 90k tbn (attached pic)
At least one output file must be specified";

    #[test]
    fn ffmpeg_output_is_read_into_a_probe() {
        let p = parse_probe(SAMPLE);
        assert_eq!(p.duration_s, Some(70.5));
        assert_eq!(p.kbps, Some(2500));
        assert_eq!(p.created_ms, Some(1_792_413_000_000));
        assert_eq!(p.picture, Some(Picture { codec: "h264".into(), pixels: "yuv420p".into(), width: 1280, height: 720 }));
        assert_eq!(p.audio.as_deref(), Some("aac"));
        let audio_only = parse_probe("  Duration: N/A, bitrate: N/A\n  Stream #0:0: Audio: mp3, 44100 Hz, stereo, fltp, 128 kb/s\n");
        assert_eq!((audio_only.duration_s, audio_only.picture, audio_only.audio.as_deref()), (None, None, Some("mp3")));
        assert_eq!(parse_probe("x.txt: Invalid data found when processing input"), Probe::default());
    }

    fn probe(codec: &str, pixels: &str, audio: Option<&str>) -> Probe {
        Probe { duration_s: Some(10.0), picture: Some(Picture { codec: codec.into(), pixels: pixels.into(), width: 640, height: 360 }), audio: audio.map(String::from), ..Default::default() }
    }

    #[test]
    fn a_playable_codec_is_copied_and_anything_else_is_converted() {
        let none: Vec<String> = Vec::new();
        let all: Vec<String> = ["hevc", "av1", "h264"].map(String::from).to_vec();
        assert_eq!(decide(&probe("h264", "yuv420p", Some("aac")), &none), Recipe { copy_picture: true, copy_audio: true, tag_hevc: false });
        assert!(!decide(&probe("h264", "yuv444p", Some("aac")), &none).copy_picture);
        assert!(!decide(&probe("h264", "yuv420p10le", None), &all).copy_picture);
        assert!(!decide(&probe("hevc", "yuv420p", None), &none).copy_picture);
        assert_eq!(decide(&probe("hevc", "yuv420p", None), &all), Recipe { copy_picture: true, copy_audio: false, tag_hevc: true });
        assert!(!decide(&probe("av1", "yuv420p", None), &none).copy_picture);
        assert!(decide(&probe("av1", "yuv420p", None), &all).copy_picture);
        assert!(decide(&probe("vp9", "yuv420p", Some("opus")), &none).copy_audio);
        assert!(!decide(&probe("mpeg4", "yuv420p", Some("wmav2")), &all).copy_picture);
        assert!(!decide(&probe("prores", "yuv422p10le", Some("pcm_s16le")), &all).copy_audio);
        assert_eq!(decide(&Probe::default(), &all), Recipe { copy_picture: false, copy_audio: false, tag_hevc: false });
    }

    #[test]
    fn the_command_copies_when_it_can_and_never_reencodes_for_nothing() {
        let copy = convert_args("in.mp4", &probe("h264", "yuv420p", Some("aac")), &Recipe { copy_picture: true, copy_audio: true, tag_hevc: false }, "v");
        let text = copy.join(" ");
        assert!(text.contains("-map 0:V:0 -map 0:a:0 -c:v copy -c:a copy"), "{text}");
        assert!(text.contains("-movflags frag_keyframe+empty_moov+default_base_moof -frag_duration 1000000 -f mp4 pipe:1"));
        assert!(!text.contains("libx264") && !text.contains("-tag:v"));
        let convert = convert_args("in.avi", &probe("mpeg4", "yuv420p", Some("wmav2")), &Recipe { copy_picture: false, copy_audio: false, tag_hevc: false }, "v").join(" ");
        assert!(convert.contains("-c:v libx264") && convert.contains("-c:a aac -b:a 128k") && convert.contains("-force_key_frames expr:gte(t,n_forced*1)"));
        let hevc = convert_args("in.mp4", &probe("hevc", "yuv420p", None), &Recipe { copy_picture: true, copy_audio: false, tag_hevc: true }, "v").join(" ");
        assert!(hevc.contains("-c:v copy -tag:v hvc1") && !hevc.contains("-map 0:a") && !hevc.contains("-c:a"));
        let voice = convert_args("v.m4a", &probe("h264", "yuv420p", Some("aac")), &Recipe { copy_picture: false, copy_audio: true, tag_hevc: false }, "m").join(" ");
        assert!(voice.contains("-map 0:a:0 -vn -c:a copy") && voice.contains("-movflags empty_moov+default_base_moof") && !voice.contains("-c:v"));
    }

    #[test]
    fn notes_chapters_and_subtitles_become_events_of_the_session() {
        let items = vec![
            Item { at_s: 84.5, to_s: None, kind: "note".into(), text: "Late rotate #rotate".into(), tags: vec!["rotate".into(), "mistake".into()] },
            Item { at_s: 90.0, to_s: None, kind: "mark".into(), text: String::new(), tags: Vec::new() },
            Item { at_s: 0.0, to_s: Some(60.0), kind: "chapter".into(), text: "Match 1".into(), tags: Vec::new() },
        ];
        let cues = vec![("owner".to_string(), vec![Cue { from_s: 5.0, to_s: 8.0, text: "Drop north".into() }])];
        let records = item_records(&items, &cues, "s1", 1_000_000_000, 100);
        assert_eq!(records.len(), 4);
        assert_eq!((records[0]["kind"].as_str(), records[0]["text"].as_str(), records[0]["target"]["t_us"].as_i64()), (Some("note"), Some("Late rotate #rotate #mistake"), Some(1_084_500_000)));
        assert_eq!(records[0]["tags"], json!(["rotate", "mistake"]));
        assert_eq!((records[1]["kind"].as_str(), records[1]["target"]["t_us"].as_i64()), (Some("mark"), Some(1_090_000_000)));
        assert_eq!((records[2]["kind"].as_str(), records[2]["target"]["from_us"].as_i64(), records[2]["target"]["to_us"].as_i64()), (Some("chapter"), Some(1_000_000_000), Some(1_060_000_000)));
        assert_eq!((records[3]["kind"].as_str(), records[3]["target"]["file"].as_str(), records[3]["target"]["from_us"].as_i64()), (Some("speech"), Some("owner"), Some(1_005_000_000)));
        let ids: Vec<i64> = records.iter().filter_map(|r| r["id"].as_i64()).collect();
        assert_eq!(ids, [101, 102, 103, 104]);
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ezh-import-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_bare_folder_and_an_exported_folder_both_make_plans() {
        let dir = folder("plans").join("Scrim night");
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["Ann.mp4", "Ben.mkv", "readme.txt"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let plan = plan_from_folder(&dir).unwrap();
        assert_eq!((plan.title.as_str(), plan.start_ms, plan.items.len()), ("Scrim night", None, 0));
        assert_eq!(plan.views.iter().map(|v| (v.name.as_str(), v.offset_ms)).collect::<Vec<_>>(), [("Ann", 0), ("Ben", 0)]);
        std::fs::write(dir.join("session.json"), "{\"title\": \"Review\", \"start\": \"2026-10-19T12:30:00Z\", \"views\": [{\"name\": \"Coach\", \"file\": \"Ben.mkv\", \"offset_ms\": 2000}]}").unwrap();
        std::fs::write(dir.join("notes.csv"), "seconds,text\n5,Good call #callout\n").unwrap();
        std::fs::write(dir.join("chapters.vtt"), "WEBVTT\n\n00:00:00.000 --> 00:01:00.000\nMatch 1\n").unwrap();
        std::fs::write(dir.join("Ben.vtt"), "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nPush\n").unwrap();
        let plan = plan_from_folder(&dir).unwrap();
        assert_eq!((plan.title.as_str(), plan.start_ms, plan.views.len(), plan.views[0].offset_ms, plan.items.len(), plan.views[0].subtitles.len()), ("Review", Some(1_792_413_000_000), 1, 2000, 2, 1));
        assert_eq!(plan_json(&plan)["views"][0]["name"], "Coach");
        let spec = plan_from_spec(&json!({"folder": dir.to_string_lossy(), "title": "Renamed", "names": ["Mia"]})).unwrap();
        assert_eq!((spec.title.as_str(), spec.views[0].name.as_str()), ("Renamed", "Mia"));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn a_folder_with_problems_is_refused_with_a_reason() {
        let dir = folder("problems");
        assert!(plan_from_folder(&dir).unwrap_err().contains("no video"));
        std::fs::write(dir.join("session.json"), "{\"views\": [{\"file\": \"gone.mp4\"}]}").unwrap();
        assert!(plan_from_folder(&dir).unwrap_err().contains("gone.mp4 is missing"));
        std::fs::write(dir.join("session.json"), "{oops").unwrap();
        assert!(plan_from_folder(&dir).unwrap_err().contains("is not valid"));
        std::fs::write(dir.join("session.json"), "{}").unwrap();
        std::fs::write(dir.join("a.mp4"), b"x").unwrap();
        std::fs::write(dir.join("notes.csv"), "seconds,text\nsoon,x\n").unwrap();
        assert!(plan_from_folder(&dir).unwrap_err().contains("Row 2"));
        std::fs::write(dir.join("notes.csv"), "").unwrap();
        std::fs::write(dir.join("chapters.vtt"), "nope").unwrap();
        assert!(plan_from_folder(&dir).unwrap_err().contains("WEBVTT"));
        assert!(plan_from_spec(&json!({})).unwrap_err().contains("Choose"));
        assert!(plan_from_spec(&json!({"folder": dir.join("nothing").to_string_lossy()})).unwrap_err().contains("does not exist"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failed_import_removes_only_its_views_and_the_folder_once_empty() {
        let dir = folder("failed");
        let names = ["x/view1/5/v/init.mp4", "x/view2/5/m/000001.m4s.part", "x/me/7/v/000002.m4s.part", "x/peer/8/v/000001.m4s", "y/view1/5/v/000001.m4s"];
        crate::log::tests::seed(&dir, names);
        let ids = ["view1", "view2", "view3"].map(String::from);
        remove_views(&dir.join("x"), &ids);
        remove_views(&dir.join("y"), &ids);
        assert_eq!((crate::log::tests::present(&dir, &names), dir.join("y").exists()), (names[2..4].to_vec(), false));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn video_files_chosen_one_by_one_make_a_plan_too() {
        let dir = folder("files");
        let (a, b) = (dir.join("Ann POV.mp4"), dir.join("Ben POV.mp4"));
        std::fs::write(&a, b"x").unwrap();
        std::fs::write(&b, b"x").unwrap();
        let one = plan_from_spec(&json!({"files": [a.to_string_lossy()]})).unwrap();
        assert_eq!((one.title.as_str(), one.views[0].name.as_str()), ("Ann POV", "Ann POV"));
        let two = plan_from_spec(&json!({"files": [b.to_string_lossy(), a.to_string_lossy()]})).unwrap();
        assert_eq!(two.views.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(), ["Ann POV", "Ben POV"]);
        assert!(two.title.starts_with("ezh-import-files"));
        assert!(plan_from_spec(&json!({"files": [dir.join("missing.mp4").to_string_lossy()]})).unwrap_err().contains("is missing"));
        let _ = std::fs::remove_dir_all(dir);
    }
}

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::Emitter;

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
const MATCH_TAG: &str = "match";
const FEED_CHUNK: u64 = 1 << 20;
const CONNECT_WAIT: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(50);
const ERROR_LINES: usize = 12;

pub struct Running {
    pub set: (String, String),
    child: Process,
    cancelled: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Input {
    delay_s: f64,
    end_s: f64,
    video: bool,
    audio: bool,
}

struct Captured {
    input: Input,
    pieces: Vec<Piece>,
}

#[derive(Debug, PartialEq)]
struct Note {
    at_s: f64,
    kind: String,
    text: String,
    tags: Vec<String>,
}

#[derive(Debug, PartialEq)]
struct Cue {
    from_s: f64,
    to_s: f64,
    text: String,
}

struct Output {
    name: String,
    owner: String,
    video: Vec<Captured>,
    voice: Vec<Captured>,
    size: (u32, u32),
    kbps: f64,
}

struct Job {
    feeds: Vec<Vec<Piece>>,
    args: Vec<String>,
    partial: PathBuf,
    path: PathBuf,
    total: f64,
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

fn delayed_sound(i: usize, delay: f64) -> String {
    format!("[{i}:a:0]asetpts=PTS-STARTPTS,adelay=delays={}:all=1[s{i}];", (delay * 1000.0).round() as u64)
}

fn amix(labels: &[String]) -> String {
    format!("{}amix=inputs={}:normalize=0:duration=longest,apad", labels.concat(), labels.len())
}

fn graph(inputs: &[Input], size: (u32, u32), total: f64) -> (String, bool) {
    let (w, h) = size;
    let mut graph = format!("color=c=black:s={w}x{h}:r={FPS}:d={total:.3}[bg0];");
    let (mut layer, mut sounds) = (0usize, Vec::new());
    for (i, input) in inputs.iter().enumerate() {
        if input.video {
            graph += &format!(
                "[{i}:v]setpts=PTS-STARTPTS+{:.3}/TB,scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1[v{i}];[bg{layer}][v{i}]overlay=eof_action=pass[bg{}];",
                input.delay_s,
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
        graph += &format!(";{}[mix]", amix(&sounds));
    }
    (graph, !sounds.is_empty())
}

fn voice_graph(inputs: &[Input]) -> String {
    let sounds: Vec<String> = (0..inputs.len()).map(|i| format!("[s{i}]")).collect();
    inputs.iter().enumerate().map(|(i, input)| delayed_sound(i, input.delay_s)).collect::<String>() + &amix(&sounds) + "[mix]"
}

fn copyable(inputs: &[Input], total: f64) -> bool {
    matches!(inputs, [only] if only.delay_s <= TOLERANCE_S && only.end_s >= total - TOLERANCE_S)
}

fn file_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_control() || "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() { "view".into() } else { trimmed.chars().take(60).collect() }
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

fn events_to_items(events: &[Value], set: &Set, owners: &[String]) -> (Vec<Note>, Vec<Cue>, Vec<(String, Vec<Cue>)>) {
    let start = set.start();
    let at = |us: i64| tenth((us - start) as f64 / 1e6).max(0.0);
    let (mut notes, mut chapters, mut ends): (Vec<Note>, Vec<Cue>, Vec<f64>) = (Vec::new(), Vec::new(), Vec::new());
    let mut speech: HashMap<String, Vec<Cue>> = HashMap::new();
    for e in events.iter().filter(|e| room::covers(set, e)) {
        let Some(local) = e["local_us"].as_i64() else { continue };
        let text = e["text"].as_str().unwrap_or_default().trim().to_string();
        let tags: Vec<String> = e["tags"].as_array().into_iter().flatten().filter_map(|t| t.as_str().map(String::from)).collect();
        let plugin = !e["plugin"].is_null();
        match (e["kind"].as_str().unwrap_or_default(), e["end_us"].as_i64()) {
            ("speech", Some(end)) => {
                if let Some(owner) = e["file"].as_str().filter(|o| owners.iter().any(|x| x == o)) {
                    speech.entry(owner.to_string()).or_default().push(Cue { from_s: at(local), to_s: at(end).max(at(local) + 0.1), text });
                }
            }
            ("drawing" | "board", _) => {}
            (_, Some(end)) if e["file"].is_null() && !text.is_empty() && at(end) > at(local) => chapters.push(Cue { from_s: at(local), to_s: at(end), text }),
            (_, Some(_)) => {}
            (kind @ ("note" | "mark"), None) if !plugin && (kind == "mark" || !text.is_empty()) => notes.push(Note { at_s: at(local), kind: kind.into(), text, tags }),
            (_, None) if plugin && tags.iter().any(|t| t == MATCH_TAG) => ends.push(at(local)),
            _ => {}
        }
    }
    ends.sort_by(|a, b| a.total_cmp(b));
    let (mut from, mut n) = (0.0, 0);
    for end in ends {
        if end > from {
            chapters.push(Cue { from_s: from, to_s: end, text: format!("Match {}", n + 1) });
            (from, n) = (end, n + 1);
        }
    }
    notes.sort_by(|a, b| a.at_s.total_cmp(&b.at_s));
    chapters.sort_by(|a, b| a.from_s.total_cmp(&b.from_s));
    let mut subtitles: Vec<(String, Vec<Cue>)> = speech.into_iter().collect();
    for (_, cues) in subtitles.iter_mut() {
        cues.sort_by(|a, b| a.from_s.total_cmp(&b.from_s));
    }
    (notes, chapters, subtitles)
}

fn accept_until(listener: &TcpListener, deadline: Instant, keep_going: impl Fn() -> bool) -> Option<std::net::TcpStream> {
    listener.set_nonblocking(true).ok()?;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return stream.set_nonblocking(false).ok().map(|_| stream),
            Err(_) if Instant::now() < deadline && keep_going() => std::thread::sleep(POLL),
            Err(_) => return None,
        }
    }
}

fn feed(core: Arc<Core>, listener: TcpListener, pieces: Vec<Piece>) {
    let Some(mut stream) = accept_until(&listener, Instant::now() + CONNECT_WAIT, || core.export.lock().unwrap().is_some()) else { return };
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

fn ffmpeg_command() -> Command {
    let mut command = Command::new(ffmpeg::path());
    command.args(["-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1", "-y"]);
    command
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
            let mut segs = f.segs.iter().filter(|s| s.capture == capture && s.series == series);
            let Some(first) = segs.next() else { continue };
            let end_us = segs.fold(first.t_us + first.d_us, |end, s| end.max(s.t_us + s.d_us));
            let pieces = stream::set_pieces(core, room_id, f, capture, series);
            if pieces.len() > 1 {
                let seconds = |us: i64| (us - f.offset_us - set.start()).max(0) as f64 / 1e6;
                let video = series == SERIES[0];
                let audio = !video || has_sound(&pieces[0]);
                tracks.push(Captured { input: Input { delay_s: seconds(first.t_us), end_s: seconds(end_us), video, audio }, pieces });
            }
        }
    }
    (tracks, size, kbps)
}

fn emit(core: &Core, percent: f64, done: bool, error: Option<String>) {
    let _ = core.app.emit("export", json!({"percent": tenth(percent), "done": done, "error": error}));
}

pub fn check(core: &Core) -> Result<(), String> {
    if core.active_set.lock().unwrap().is_some() {
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

fn view_outputs(core: &Core, room_id: &str, set: &Set, f: &File) -> Option<Output> {
    let (tracks, (w, h), kbps) = tracks(core, room_id, set, f);
    if tracks.is_empty() {
        return None;
    }
    let (video, voice): (Vec<Captured>, Vec<Captured>) = tracks.into_iter().partition(|c| c.input.video);
    let size = (w.max(DEFAULT_SIZE.0) & !1, h.max(DEFAULT_SIZE.1) & !1);
    Some(Output { name: room::display_name(core, &f.owner), owner: f.owner.clone(), video, voice, size, kbps })
}

fn encode_args(graph: &str, mapped_audio: bool, video: Option<Vec<String>>, total: f64) -> Vec<String> {
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
    let mut command = ffmpeg_command();
    command.current_dir(work);
    let feeds = listen(&mut command, job.feeds)?;
    command.args(&job.args).args(["-movflags", "+faststart", "-f", "mp4"]).arg(&job.partial);
    let (stdout, stderr) = begin(core, set.clone(), &mut command, *owned)?;
    *owned = true;
    serve(core, feeds);
    let tail = stderr_tail(stderr, ERROR_LINES);
    read_progress(stdout, |s| emit(core, ((done as f64 + (s / job.total.max(1.0)).clamp(0.0, 1.0)) / jobs as f64) * 100.0, false, None));
    outcome(core, tail, &job.partial, &job.path)
}

fn job(out: &Output, video: bool, path: PathBuf, work: &Path, n: usize, total: f64) -> Job {
    let captured = if video { &out.video } else { &out.voice };
    let inputs: Vec<Input> = captured.iter().map(|c| c.input.clone()).collect();
    let args = if copyable(&inputs, total) {
        ["-map", "0", "-c", "copy"].map(String::from).to_vec()
    } else if video {
        let (graph, audio) = graph(&inputs, out.size, total);
        let chosen = ffmpeg::pick(&crate::default_codecs());
        let source = if out.kbps > 0.0 { out.kbps } else { record::bitrate_kbps(out.size.0, 1.0) as f64 };
        encode_args(&graph, audio, Some(ffmpeg::video_args(chosen.0, export_kbps(&[source], out.size, chosen.2))), total)
    } else {
        encode_args(&voice_graph(&inputs), true, None, total)
    };
    let partial = work.join(format!("part-{n}.{}", if video { "mp4" } else { "m4a" }));
    Job { feeds: captured.iter().map(|c| c.pieces.clone()).collect(), args, partial, path, total }
}

pub fn folder(core: &Arc<Core>, room_id: &str, set_id: &str, dir: &Path) -> Result<Value, String> {
    check(core)?;
    let logs = room::logs(core, room_id);
    let set = room::sets(core, room_id, &logs).into_iter().find(|s| s.id == set_id).ok_or("Unknown recording.")?;
    let total = (set.end_shown().ok_or("This recording is still being recorded.")? - set.start()) as f64 / 1e6;
    if dir.is_file() || std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some()) {
        return Err("Choose a new or empty folder.".into());
    }
    let outputs: Vec<Output> = set.files.iter().filter_map(|f| view_outputs(core, room_id, &set, f)).collect();
    if outputs.is_empty() {
        return Err("None of this recording's files are on this PC yet.".into());
    }
    let skipped: Vec<String> = set.files.iter().filter(|f| !outputs.iter().any(|o| o.owner == f.owner)).map(|f| room::display_name(core, &f.owner)).collect();
    std::fs::create_dir_all(dir).map_err(|e| format!("The folder cannot be made: {e}"))?;
    let work = core.dir.join("export");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let files = export_names(&unique_names(&outputs.iter().map(|o| o.name.clone()).collect::<Vec<_>>()));
    let mut jobs = Vec::new();
    for (out, file) in outputs.iter().zip(&files) {
        let has_video = !out.video.is_empty();
        jobs.push(job(out, has_video, dir.join(format!("{file}.{}", if has_video { "mp4" } else { "m4a" })), &work, jobs.len(), total));
        if has_video && !out.voice.is_empty() {
            jobs.push(job(out, false, dir.join(format!("{file} voice.m4a")), &work, jobs.len(), total));
        }
    }
    let owners: Vec<String> = outputs.iter().map(|o| o.owner.clone()).collect();
    let (notes, chapters, subtitles) = events_to_items(&room::events(&logs, None), &set, &owners);
    let count = jobs.len();
    tracing::info!("folder export of {set_id} started: {count} files");
    let key = (room_id.to_string(), set_id.to_string());
    let mut owned = false;
    let failure = jobs.into_iter().enumerate().try_for_each(|(n, job)| run_job(core, &key, job, &work, n, count, &mut owned)).err();
    if owned {
        end(core);
    }
    if let Some(e) = failure {
        let _ = std::fs::remove_dir_all(dir);
        if owned {
            emit(core, 0.0, true, Some(e.clone()));
        }
        return Err(e);
    }
    let write = |name: &str, text: String| std::fs::write(dir.join(name), text).map_err(|e| format!("Export failed: {e}"));
    if !notes.is_empty() {
        write(NOTES, notes_text(&notes))?;
    }
    if !chapters.is_empty() {
        write(CHAPTERS, vtt_text(&chapters))?;
    }
    if let Some(records) = crate::party::records(&logs).remove(set_id) {
        write(WATCH, crate::party::watch_json(&records).to_string())?;
    }
    for (owner, cues) in &subtitles {
        if let Some(i) = outputs.iter().position(|o| o.owner == *owner) {
            write(&format!("{}.vtt", files[i]), vtt_text(cues))?;
        }
    }
    emit(core, 100.0, true, None);
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
        assert!(g.ends_with("[bg2]format=yuv420p[vout];[s0]amix=inputs=1:normalize=0:duration=longest,apad[mix]"));
        let (silent, audio) = graph(&[input(2.0, true, false)], (640, 360), 10.0);
        assert!(!audio && silent.ends_with("[bg1]format=yuv420p[vout]"));
        assert_eq!(voice_graph(&[input(1.25, false, true), input(9.0, false, true)]), "[0:a:0]asetpts=PTS-STARTPTS,adelay=delays=1250:all=1[s0];[1:a:0]asetpts=PTS-STARTPTS,adelay=delays=9000:all=1[s1];[s0][s1]amix=inputs=2:normalize=0:duration=longest,apad[mix]");
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
        Captured { input: input(delay_s, video, !video), pieces: vec![Piece { path: PathBuf::from("a"), len: 1 }, Piece { path: PathBuf::from("b"), len: 1 }] }
    }

    #[test]
    fn the_folder_export_always_starts_every_file_at_the_recording_start() {
        let work = Path::new("w");
        let late = Output { name: "Ann".into(), owner: "a".into(), video: vec![captured(61.5, true)], voice: vec![captured(61.5, false)], size: (1280, 720), kbps: 3000.0 };
        let video = job(&late, true, PathBuf::from("Ann.mp4"), work, 0, 120.0).args.join(" ");
        assert!(video.contains("color=c=black:s=1280x720:r=30:d=120.000") && video.contains("setpts=PTS-STARTPTS+61.500/TB") && video.ends_with("-t 120.000"));
        let voice = job(&late, false, PathBuf::from("Ann voice.m4a"), work, 1, 120.0).args.join(" ");
        assert!(voice.contains("adelay=delays=61500:all=1") && voice.ends_with("-t 120.000"));
        let first = Output { video: vec![captured(0.0, true)], ..late };
        assert_eq!(job(&first, true, PathBuf::from("Ann.mp4"), work, 0, 120.0).args.join(" "), "-map 0 -c copy");
        let early = Output { video: vec![Captured { input: Input { end_s: 60.0, ..input(0.0, true, true) }, ..captured(0.0, true) }], ..first };
        let padded = job(&early, true, PathBuf::from("Ann.mp4"), work, 0, 120.0).args.join(" ");
        assert!(padded.contains("color=c=black:s=1280x720:r=30:d=120.000") && padded.contains("duration=longest,apad[mix]") && padded.ends_with("-t 120.000"));
    }

    fn fake_set() -> Set {
        Set { id: "s1".into(), room: "r".into(), files: vec![File { set: "s1".into(), owner: "a".into(), start_us: 1_000_000_000, end_us: Some(1_100_000_000), offset_us: 0, segs: Vec::new(), caps: HashMap::new(), view: None, present: false }], stopped: false }
    }

    fn event(kind: &str, local: i64, end: Option<i64>, text: &str, tags: Value, plugin: bool, file: Option<&str>) -> Value {
        json!({"kind": kind, "local_us": local, "end_us": end, "text": text, "tags": tags, "plugin": if plugin { json!({"id": "p"}) } else { Value::Null }, "file": file})
    }

    #[test]
    fn events_become_notes_chapters_and_subtitles_on_the_recording_clock() {
        let set = fake_set();
        let s = 1_000_000_000i64;
        let events = vec![
            event("note", s + 84_500_000, None, "Late rotate #rotate", json!(["rotate"]), false, None),
            event("mark", s + 90_000_000, None, "", json!([]), false, None),
            event("chapter", s, Some(s + 30_000_000), "Warm up", json!([]), false, None),
            event("match_end", s + 60_000_000, None, "", json!(["match"]), true, None),
            event("match_end", s + 30_000_000, None, "", json!(["match"]), true, None),
            event("match_end", s + 30_000_000, None, "", json!(["match"]), true, None),
            event("speech", s + 5_000_000, Some(s + 8_000_000), "Drop north", json!([]), true, Some("a")),
            event("speech", s + 6_000_000, Some(s + 7_000_000), "Elsewhere", json!([]), true, Some("zz")),
            event("drawing", s + 7_000_000, None, "", json!([]), false, None),
            event("note", s + 500_000_000, None, "Outside", json!([]), false, None),
            event("note", s + 1_000_000, None, "Plugin note", json!([]), true, None),
        ];
        let (notes, chapters, subtitles) = events_to_items(&events, &set, &["a".to_string()]);
        let notes: Vec<(f64, &str, &str)> = notes.iter().map(|n| (n.at_s, n.kind.as_str(), n.text.as_str())).collect();
        assert_eq!(notes, [(84.5, "note", "Late rotate #rotate"), (90.0, "mark", "")]);
        let chapters: Vec<(f64, f64, &str)> = chapters.iter().map(|c| (c.from_s, c.to_s, c.text.as_str())).collect();
        assert_eq!(chapters, [(0.0, 30.0, "Warm up"), (0.0, 30.0, "Match 1"), (30.0, 60.0, "Match 2")]);
        assert_eq!(subtitles, vec![("a".to_string(), vec![Cue { from_s: 5.0, to_s: 8.0, text: "Drop north".into() }])]);
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
    fn exported_file_names_never_meet_the_files_beside_them() {
        let names: Vec<String> = ["Ann/Ben", "Chapters", "notes", "ann_ben", "Session"].map(String::from).to_vec();
        assert_eq!(export_names(&names), ["Ann_Ben", "Chapters view", "notes view", "ann_ben 2", "Session"]);
    }

    #[test]
    fn notes_are_written_as_csv() {
        let notes = vec![
            Note { at_s: 84.5, kind: "note".into(), text: "Late, \"very\" late\nrotate".into(), tags: vec!["mistake".into(), "rotate".into()] },
            Note { at_s: 90.0, kind: "mark".into(), text: String::new(), tags: Vec::new() },
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

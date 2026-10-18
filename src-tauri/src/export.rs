use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use tauri::Emitter;

use crate::ffmpeg::{self, FRAGMENTED};
use crate::record::FPS;
use crate::room::{LEGACY, SERIES};
use crate::stream::{self, Piece};
use crate::{room, strings, Core, BELOW_NORMAL, NO_WINDOW};

const CANVAS: (u32, u32) = (2560, 1440);
const VIDEO_KBPS: u32 = 12000;
const AUDIO_KBPS: u32 = 160;
const KEYFRAME_EVERY: u32 = 2 * FPS;
const LABEL_PX: u32 = 28;
const LABEL_MARGIN: u32 = 16;
const PARTIAL: &str = "export.mp4";
const FONTS: [&str; 3] = ["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"];
const FEED_CHUNK: u64 = 1 << 20;
const CONNECT_WAIT: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(50);

pub struct Running {
    child: Child,
    cancelled: bool,
}

pub fn grid(n: usize, (width, height): (u32, u32)) -> Vec<(u32, u32, u32, u32)> {
    let cols = (n.max(1) as f64).sqrt().ceil() as usize;
    let rows = n.max(1).div_ceil(cols);
    let unit = (width as f64 / cols as f64).min(height as f64 / rows as f64 * 16.0 / 9.0);
    let (w, h) = (unit as u32 & !1, (unit * 9.0 / 16.0) as u32 & !1);
    let (left, top) = ((width - w * cols as u32) / 2, (height - h * rows as u32) / 2);
    (0..n).map(|i| (left + (i % cols) as u32 * w, top + (i / cols) as u32 * h, w, h)).collect()
}

fn feed(core: Arc<Core>, listener: TcpListener, pieces: Vec<Piece>) {
    let deadline = Instant::now() + CONNECT_WAIT;
    if listener.set_nonblocking(true).is_err() {
        return;
    }
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(_) if Instant::now() < deadline && core.export.lock().unwrap().is_some() => std::thread::sleep(POLL),
            Err(_) => return,
        }
    };
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let total = stream::size(&pieces);
    let mut at = 0;
    while at < total {
        let Ok(bytes) = stream::read(&core, &pieces, at, FEED_CHUNK.min(total - at)) else { return };
        if stream.write_all(&bytes).is_err() {
            return;
        }
        at += bytes.len() as u64;
    }
}

fn quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\\', "/").replace(':', "\\:"))
}

fn emit(core: &Core, percent: f64, done: bool, error: Option<String>) {
    let _ = core.app.emit("export", json!({"percent": (percent * 10.0).round() / 10.0, "done": done, "error": error}));
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

pub fn start(core: &Arc<Core>, room_id: &str, set_id: &str, mut path: PathBuf) -> Result<Vec<String>, String> {
    check(core)?;
    let set = room::sets(core, room_id, &room::logs(core, room_id)).into_iter().find(|s| s.id == set_id).ok_or("Unknown set.")?;
    let end = set.end().ok_or("This set is still being recorded.")?;
    let total = (end - set.start()) as f64 / 1e6;
    if path.extension().is_none() {
        path.set_extension("mp4");
    }
    let logs = room::logs(core, room_id);
    let order: Vec<String> = strings(&room::set_layout(&logs, set_id)["order"]);
    let mut files: Vec<&room::File> = set.files.iter().collect();
    files.sort_by_key(|f| order.iter().position(|o| *o == f.owner).unwrap_or(usize::MAX));
    let work = core.dir.join("export");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let fonts = PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts");
    let font = FONTS.iter().map(|f| fonts.join(f)).find(|f| f.exists()).ok_or("No Japanese font found in Windows.")?;
    let mut command = Command::new(ffmpeg::path());
    command.args(["-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1", "-y"]);
    let mut graph = format!("color=c=black:s={}x{}:r={FPS}:d={total:.3}[bg0];", CANVAS.0, CANVAS.1);
    let (mut labels, mut mixes, mut names, mut skipped, mut feeds) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut players: Vec<(&room::File, Vec<(Vec<Piece>, f64, bool)>)> = Vec::new();
    for f in &files {
        let mut inputs = Vec::new();
        for capture in f.captures() {
            for series in SERIES {
                let Some(first) = f.segs.iter().find(|s| s.capture == capture && s.series == series) else { continue };
                let pieces = stream::set_pieces(core, room_id, &set, f, capture, series);
                if pieces.len() > usize::from(capture != LEGACY) {
                    let delay = ((first.t_us - f.offset_us) - set.start()).max(0) as f64 / 1e6;
                    inputs.push((pieces, delay, series == SERIES[0]));
                }
            }
        }
        if inputs.is_empty() {
            skipped.push(room::display_name(core, &f.owner));
        } else {
            players.push((f, inputs));
        }
    }
    let rects = grid(players.len(), CANVAS);
    let (mut input, mut layer) = (0usize, 0usize);
    for (i, ((f, inputs), (x, y, w, h))) in players.into_iter().zip(rects).enumerate() {
        let name = room::display_name(core, &f.owner);
        let mut sounds = Vec::new();
        for (pieces, delay, video) in inputs {
            let legacy = pieces.first().is_some_and(|p| p.raw);
            let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
            let port = listener.local_addr().map_err(|e| e.to_string())?.port();
            command.arg("-i").arg(format!("tcp://127.0.0.1:{port}"));
            feeds.push((listener, pieces));
            let j = input;
            input += 1;
            if video {
                graph += &format!(
                    "[{j}:v]setpts=PTS-STARTPTS+{delay:.3}/TB,scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1[v{j}];\
                     [bg{layer}][v{j}]overlay=x={x}:y={y}:eof_action=pass[bg{}];",
                    layer + 1
                );
                layer += 1;
            }
            for k in 0..if legacy { 2 } else { 1 } {
                graph += &format!("[{j}:a:{k}]asetpts=PTS-STARTPTS,adelay=delays={}:all=1[s{j}_{k}];", (delay * 1000.0).round() as u64);
                sounds.push(format!("[s{j}_{k}]"));
            }
        }
        graph += &format!("{}amix=inputs={}:normalize=0:duration=longest,asplit=2[p{i}][m{i}];", sounds.concat(), sounds.len());
        let label = work.join(format!("label-{i}.txt"));
        std::fs::write(&label, &name).map_err(|e| e.to_string())?;
        labels.push(format!(
            "drawtext=fontfile={}:textfile={}:expansion=none:fontsize={LABEL_PX}:fontcolor=white:box=1:boxcolor=black@0.6:boxborderw=8:x={}:y={}-th",
            quoted(&font),
            quoted(&label),
            x + LABEL_MARGIN,
            y + h - LABEL_MARGIN
        ));
        mixes.push(format!("[m{i}]"));
        names.push(name);
    }
    if names.is_empty() {
        return Err("None of this set's files are on this PC yet.".into());
    }
    labels.push("format=yuv420p".into());
    graph += &format!("[bg{layer}]{}[vout];{}amix=inputs={}:normalize=0:duration=longest[mix]", labels.join(","), mixes.concat(), names.len());
    let encoder = ffmpeg::encoder();
    command
        .args(["-filter_complex", &graph, "-map", "[vout]", "-map", "[mix]"])
        .args((0..names.len()).flat_map(|i| ["-map".to_string(), format!("[p{i}]")]))
        .args(["-c:v", encoder, "-b:v", &format!("{VIDEO_KBPS}k"), "-g", &KEYFRAME_EVERY.to_string(), "-r", &FPS.to_string()]);
    if encoder == ffmpeg::FALLBACK.0 {
        command.args(["-preset", "veryfast"]);
    }
    command
        .args(["-c:a", "aac", "-b:a", &format!("{AUDIO_KBPS}k")])
        .args(std::iter::once("Mix").chain(names.iter().map(String::as_str)).enumerate().flat_map(|(i, n)| {
            [format!("-metadata:s:a:{i}"), format!("title={n}"), format!("-metadata:s:a:{i}"), format!("handler_name={n}")]
        }))
        .args(["-t", &format!("{total:.3}"), "-movflags", FRAGMENTED, "-f", "mp4"])
        .arg(work.join(PARTIAL))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(NO_WINDOW | BELOW_NORMAL);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let (stdout, stderr) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    *core.export.lock().unwrap() = Some(Running { child, cancelled: false });
    for (listener, pieces) in feeds {
        let core = core.clone();
        std::thread::spawn(move || feed(core, listener, pieces));
    }
    let core = core.clone();
    std::thread::spawn(move || watch(&core, stdout, stderr, &work.join(PARTIAL), &path, total));
    Ok(skipped)
}

fn finish(partial: &Path, path: &Path) -> std::io::Result<()> {
    if std::fs::rename(partial, path).is_err() {
        std::fs::copy(partial, path)?;
        std::fs::remove_file(partial)?;
    }
    Ok(())
}

fn watch(core: &Core, stdout: ChildStdout, mut stderr: ChildStderr, partial: &Path, path: &Path, total: f64) {
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    emit(core, 0.0, false, None);
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.parse::<f64>().ok()) {
            emit(core, (us / 1e6 / total * 100.0).clamp(0.0, 100.0), false, None);
        }
    }
    let Some(mut running) = core.export.lock().unwrap().take() else { return };
    let ok = running.child.wait().is_ok_and(|s| s.success());
    let text = errors.join().unwrap_or_default();
    if running.cancelled {
        let _ = std::fs::remove_file(partial);
        emit(core, 0.0, true, Some("Export cancelled".into()));
    } else if ok {
        match finish(partial, path) {
            Ok(()) => emit(core, 100.0, true, None),
            Err(e) => emit(core, 0.0, true, Some(format!("Export failed: {e}"))),
        }
    } else {
        tracing::warn!("export failed: {text}");
        emit(core, 0.0, true, Some(format!("Export failed: {}", text.lines().last().unwrap_or("ffmpeg stopped"))));
    }
}

pub fn cancel(core: &Core) {
    if let Some(running) = core.export.lock().unwrap().as_mut() {
        running.cancelled = true;
        let _ = running.child.kill();
    }
}

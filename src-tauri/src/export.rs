use std::io::{BufRead, BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::Emitter;

use crate::{record, room, short, Core};

const CANVAS: (u32, u32) = (2560, 1440);
const FPS: u32 = 30;
const VIDEO_KBPS: u32 = 12000;
const AUDIO_KBPS: u32 = 160;
const KEYFRAME_EVERY: u32 = 2 * FPS;
const LABEL_PX: u32 = 28;
const LABEL_MARGIN: u32 = 16;
const FONTS: [&str; 3] = ["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"];
const NO_WINDOW: u32 = 0x08000000;
const BELOW_NORMAL: u32 = 0x00004000;

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

fn quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\\', "/").replace(':', "\\:"))
}

fn name_of(core: &Core, id: &str) -> String {
    if id == core.ep.id().to_string() {
        return core.name.lock().unwrap().clone();
    }
    let peer = core.peers.lock().unwrap().iter().find(|p| p.id.to_string() == id && !p.name.is_empty()).map(|p| p.name.clone());
    peer.or_else(|| core.active_room().and_then(|r| r.names.get(id).cloned())).unwrap_or_else(|| id.parse().map(|i| short(&i)).unwrap_or_default())
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
    if !record::ffmpeg_present() {
        return Err("The video tool is still downloading.".into());
    }
    Ok(())
}

pub fn start(core: &Arc<Core>, room_id: &str, set_id: &str, mut path: PathBuf) -> Result<Vec<String>, String> {
    check(core)?;
    let set = room::sets(core, room_id, &room::logs(core, room_id)).into_iter().find(|s| s.id == set_id).ok_or("Unknown set.")?;
    let end = set.end().ok_or("This set is still being recorded.")?;
    let total = (end - set.start()) as f64 / 1e6;
    if path.extension().is_none() {
        path.set_extension("mp4");
    }
    let dir = core.set_dir(room_id, set_id);
    let saved: Value = std::fs::read_to_string(dir.join("set.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let order: Vec<&str> = saved["layout"]["order"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect();
    let mut files: Vec<&room::File> = set.files.iter().collect();
    files.sort_by_key(|f| order.iter().position(|o| *o == f.owner).unwrap_or(usize::MAX));
    let work = core.dir.join("export");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let fonts = PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts");
    let font = FONTS.iter().map(|f| fonts.join(f)).find(|f| f.exists()).ok_or("No Japanese font found in Windows.")?;
    let rects = grid(files.len(), CANVAS);
    let mut command = Command::new(record::ffmpeg());
    command.args(["-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1", "-y"]);
    let mut graph = format!("color=c=black:s={}x{}:r={FPS}:d={total:.3}[bg0];", CANVAS.0, CANVAS.1);
    let (mut labels, mut mixes, mut names, mut skipped) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for (f, (x, y, w, h)) in files.iter().zip(rects) {
        let name = name_of(core, &f.owner);
        let file = dir.join(format!("{}.mp4", f.owner));
        if std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0) == 0 {
            skipped.push(name);
            continue;
        }
        let i = names.len();
        let delay = ((f.start_us - f.offset_us) - set.start()).max(0) as f64 / 1e6;
        command.arg("-i").arg(&file);
        graph += &format!(
            "[{i}:v]setpts=PTS-STARTPTS+{delay:.3}/TB,scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1[v{i}];\
             [bg{i}][v{i}]overlay=x={x}:y={y}:eof_action=pass[bg{}];\
             [{i}:a:0][{i}:a:1]amix=inputs=2:normalize=0:duration=longest,asetpts=PTS-STARTPTS,adelay=delays={}:all=1,asplit=2[p{i}][m{i}];",
            i + 1,
            (delay * 1000.0).round() as u64
        );
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
    graph += &format!("[bg{}]{}[vout];{}amix=inputs={}:normalize=0:duration=longest[mix]", names.len(), labels.join(","), mixes.concat(), names.len());
    let encoder = record::encoder();
    command
        .args(["-filter_complex", &graph, "-map", "[vout]", "-map", "[mix]"])
        .args((0..names.len()).flat_map(|i| ["-map".to_string(), format!("[p{i}]")]))
        .args(["-c:v", encoder, "-b:v", &format!("{VIDEO_KBPS}k"), "-g", &KEYFRAME_EVERY.to_string(), "-r", &FPS.to_string()])
        .args(if encoder == "libx264" { ["-preset", "veryfast"] } else { ["-v", "error"] })
        .args(["-c:a", "aac", "-b:a", &format!("{AUDIO_KBPS}k")])
        .args(std::iter::once("Mix").chain(names.iter().map(String::as_str)).enumerate().flat_map(|(i, n)| {
            [format!("-metadata:s:a:{i}"), format!("title={n}"), format!("-metadata:s:a:{i}"), format!("handler_name={n}")]
        }))
        .args(["-t", &format!("{total:.3}"), "-movflags", "frag_keyframe+empty_moov+default_base_moof", "-f", "mp4"])
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(NO_WINDOW | BELOW_NORMAL);
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let (stdout, mut stderr) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    *core.export.lock().unwrap() = Some(Running { child, cancelled: false });
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let core = core.clone();
    std::thread::spawn(move || {
        emit(&core, 0.0, false, None);
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.parse::<f64>().ok()) {
                emit(&core, (us / 1e6 / total * 100.0).clamp(0.0, 100.0), false, None);
            }
        }
        let Some(mut running) = core.export.lock().unwrap().take() else { return };
        let ok = running.child.wait().is_ok_and(|s| s.success());
        let text = errors.join().unwrap_or_default();
        if running.cancelled {
            let _ = std::fs::remove_file(&path);
            emit(&core, 0.0, true, Some("Export cancelled".into()));
        } else if ok {
            emit(&core, 100.0, true, None);
        } else {
            tracing::warn!("export failed: {text}");
            emit(&core, 0.0, true, Some(format!("Export failed: {}", text.lines().last().unwrap_or("ffmpeg stopped"))));
        }
    });
    Ok(skipped)
}

pub fn cancel(core: &Core) {
    if let Some(running) = core.export.lock().unwrap().as_mut() {
        running.cancelled = true;
        let _ = running.child.kill();
    }
}

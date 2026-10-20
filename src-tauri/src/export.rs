use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::Emitter;

use crate::ffmpeg::{self, FRAGMENTED};
use crate::proc::{self, Process, BELOW_NORMAL};
use crate::record::{self, AUDIO_KBPS, FPS};
use crate::room::{self, part_of, SERIES};
use crate::stream::{self, Piece};
use crate::{bundle, strings, Core};

const CANVAS: (u32, u32) = (2560, 1440);
const LABEL_PX: u32 = 28;
const LABEL_MARGIN: u32 = 16;
const PARTIAL: &str = "export.mp4";
const FONT: &str = "font.ttc";
const FONTS: [&str; 3] = ["YuGothM.ttc", "meiryo.ttc", "msgothic.ttc"];
const FEED_CHUNK: u64 = 1 << 20;
const CONNECT_WAIT: Duration = Duration::from_secs(120);
const POLL: Duration = Duration::from_millis(50);
const ERROR_LINES: usize = 20;

pub struct Running {
    pub set: (String, String),
    child: Process,
    cancelled: bool,
}

pub struct Track {
    pub pieces: Vec<Piece>,
    pub delay: f64,
    pub video: bool,
}

pub struct Player {
    pub name: String,
    pub inputs: Vec<Track>,
    pub width: u32,
    pub kbps: f64,
}

pub fn source_kbps(cap: &Value) -> f64 {
    let width = cap["width"].as_u64().map_or(record::quality_width(""), |w| w as u32);
    let kbps = cap["kbps"].as_f64().unwrap_or_else(|| record::bitrate_kbps(width, 1.0) as f64);
    kbps / ffmpeg::CANDIDATES.iter().find(|c| c.1 == cap["codec"]).map_or(1.0, |c| c.2)
}

pub fn canvas(n: usize, widest: u32) -> (u32, u32) {
    let cols = (n.max(1) as f64).sqrt().ceil() as u32;
    let rows = (n.max(1) as u32).div_ceil(cols);
    let (w, h) = (cols * widest.max(2), rows * (widest.max(2) * 9 / 16));
    let scale = (CANVAS.0 as f64 / w as f64).min(CANVAS.1 as f64 / h as f64).min(1.0);
    ((w as f64 * scale) as u32 & !1, (h as f64 * scale) as u32 & !1)
}

pub fn export_kbps(sources: &[f64], size: (u32, u32), factor: f64) -> u32 {
    let ceiling = record::bitrate_kbps(size.0.max(size.1 * 16 / 9), 1.0) as f64;
    (sources.iter().sum::<f64>().min(ceiling) * factor).round() as u32
}

pub fn grid(n: usize, (width, height): (u32, u32)) -> Vec<(u32, u32, u32, u32)> {
    let cols = (n.max(1) as f64).sqrt().ceil() as usize;
    let rows = n.max(1).div_ceil(cols);
    let unit = (width as f64 / cols as f64).min(height as f64 / rows as f64 * 16.0 / 9.0);
    let (w, h) = (unit as u32 & !1, (unit * 9.0 / 16.0) as u32 & !1);
    let (left, top) = ((width - w * cols as u32) / 2, (height - h * rows as u32) / 2);
    (0..n).map(|i| (left + (i % cols) as u32 * w, top + (i / cols) as u32 * h, w, h)).collect()
}

pub fn label_file(i: usize) -> String {
    format!("label-{i}.txt")
}

pub fn delayed_sound(i: usize, delay: f64) -> String {
    format!("[{i}:a:0]asetpts=PTS-STARTPTS,adelay=delays={}:all=1[s{i}];", (delay * 1000.0).round() as u64)
}

pub fn amix(labels: &[String]) -> String {
    format!("{}amix=inputs={}:normalize=0:duration=longest", labels.concat(), labels.len())
}

pub fn graph(players: &[Vec<(f64, bool)>], rects: &[(u32, u32, u32, u32)], size: (u32, u32), total: f64) -> String {
    let mut graph = format!("color=c=black:s={}x{}:r={FPS}:d={total:.3}[bg0];", size.0, size.1);
    let (mut labels, mut mixes) = (Vec::new(), Vec::new());
    let (mut input, mut layer) = (0usize, 0usize);
    for (i, (inputs, (x, y, w, h))) in players.iter().zip(rects).enumerate() {
        let mut sounds = Vec::new();
        for (delay, video) in inputs {
            let j = input;
            input += 1;
            if *video {
                graph += &format!(
                    "[{j}:v]setpts=PTS-STARTPTS+{delay:.3}/TB,scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1[v{j}];\
                     [bg{layer}][v{j}]overlay=x={x}:y={y}:eof_action=pass[bg{}];",
                    layer + 1
                );
                layer += 1;
            }
            graph += &delayed_sound(j, *delay);
            sounds.push(format!("[s{j}]"));
        }
        graph += &format!("{},asplit=2[p{i}][m{i}];", amix(&sounds));
        labels.push(format!(
            "drawtext=fontfile={FONT}:textfile={}:expansion=none:fontsize={LABEL_PX}:fontcolor=white:box=1:boxcolor=black@0.6:boxborderw=8:x={}:y={}-th",
            label_file(i),
            x + LABEL_MARGIN,
            y + h - LABEL_MARGIN
        ));
        mixes.push(format!("[m{i}]"));
    }
    labels.push("format=yuv420p".into());
    graph + &format!("[bg{layer}]{}[vout];{}[mix]", labels.join(","), amix(&mixes))
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
        let Ok(bytes) = stream::read(&core, &pieces, at, FEED_CHUNK.min(total - at)) else { return };
        if stream.write_all(&bytes).is_err() {
            return;
        }
        at += bytes.len() as u64;
    }
}

pub fn ffmpeg_command() -> Command {
    let mut command = Command::new(ffmpeg::path());
    command.args(["-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1", "-y"]);
    command
}

pub fn listen(command: &mut Command, inputs: impl IntoIterator<Item = Vec<Piece>>) -> Result<Vec<(TcpListener, Vec<Piece>)>, String> {
    inputs
        .into_iter()
        .map(|pieces| {
            let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
            command.arg("-i").arg(format!("tcp://127.0.0.1:{}", listener.local_addr().map_err(|e| e.to_string())?.port()));
            Ok((listener, pieces))
        })
        .collect()
}

pub fn serve(core: &Arc<Core>, feeds: Vec<(TcpListener, Vec<Piece>)>) {
    for (listener, pieces) in feeds {
        let core = core.clone();
        std::thread::spawn(move || feed(core, listener, pieces));
    }
}

pub fn stderr_tail(stderr: ChildStderr, n: usize) -> JoinHandle<String> {
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

pub fn read_progress(stdout: ChildStdout, mut seconds: impl FnMut(f64)) {
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.parse::<f64>().ok()) {
            seconds(us / 1e6);
        }
    }
}

pub fn tracks(core: &Core, room_id: &str, set: &room::Set, f: &room::File) -> (Vec<Track>, (u32, u32), f64) {
    let (mut tracks, mut size, mut kbps) = (Vec::new(), (0, 0), 0f64);
    for capture in f.captures() {
        if let Some(cap) = f.caps.get(capture) {
            let side = |key: &str| cap[key].as_u64().unwrap_or(0) as u32;
            size = (size.0.max(side("width")), size.1.max(side("height")));
            kbps = kbps.max(source_kbps(cap));
        }
        for series in SERIES {
            let Some(first) = f.segs.iter().find(|s| s.capture == capture && s.series == series) else { continue };
            let pieces = stream::set_pieces(core, room_id, f, capture, series);
            if pieces.len() > 1 {
                tracks.push(Track { pieces, delay: ((first.t_us - f.offset_us) - set.start()).max(0) as f64 / 1e6, video: series == SERIES[0] });
            }
        }
    }
    (tracks, size, kbps)
}

pub fn emit(core: &Core, percent: f64, done: bool, error: Option<String>) {
    let _ = core.app.emit("export", json!({"percent": crate::session::tenth(percent), "done": done, "error": error}));
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

fn players(core: &Core, room_id: &str, set: &room::Set) -> (Vec<Player>, Vec<String>) {
    let logs = room::logs(core, room_id);
    let order: Vec<String> = strings(&room::set_layout(&logs, &set.id)["order"]);
    let mut files: Vec<&room::File> = set.files.iter().collect();
    files.sort_by_key(|f| order.iter().position(|o| *o == f.owner).unwrap_or(usize::MAX));
    let (mut players, mut skipped) = (Vec::new(), Vec::new());
    for f in files {
        let (inputs, (width, _), kbps) = tracks(core, room_id, set, f);
        let name = room::display_name(core, &f.owner);
        if inputs.is_empty() {
            skipped.push(name);
        } else {
            players.push(Player { name, inputs, width: if width > 0 { width } else { record::quality_width("") }, kbps });
        }
    }
    (players, skipped)
}

pub fn start(core: &Arc<Core>, room_id: &str, set_id: &str, mut path: PathBuf) -> Result<Vec<String>, String> {
    check(core)?;
    let set = room::sets(core, room_id, &room::logs(core, room_id)).into_iter().find(|s| s.id == set_id).ok_or("Unknown set.")?;
    let end = set.end().ok_or("This set is still being recorded.")?;
    let total = (end - set.start()) as f64 / 1e6;
    if path.extension().is_none() {
        path.set_extension("mp4");
    }
    let (players, skipped) = players(core, room_id, &set);
    let players = bundle::found(players)?;
    let work = core.dir.join("export");
    std::fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let fonts = PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into())).join("Fonts");
    let font = FONTS.iter().map(|f| fonts.join(f)).find(|f| f.exists()).ok_or("No Japanese font found in Windows.")?;
    if !work.join(FONT).exists() {
        std::fs::copy(&font, work.join(FONT)).map_err(|e| e.to_string())?;
    }
    for (i, player) in players.iter().enumerate() {
        std::fs::write(work.join(label_file(i)), &player.name).map_err(|e| e.to_string())?;
    }
    let shape: Vec<Vec<(f64, bool)>> = players.iter().map(|p| p.inputs.iter().map(|i| (i.delay, i.video)).collect()).collect();
    let size = canvas(players.len(), players.iter().map(|p| p.width).max().unwrap_or(0));
    let graph = graph(&shape, &grid(players.len(), size), size, total);
    let chosen = ffmpeg::pick(&crate::default_codecs());
    let kbps = export_kbps(&players.iter().map(|p| p.kbps).collect::<Vec<_>>(), size, chosen.2);
    tracing::info!("export of {set_id} started: {} players, {}x{}, {kbps} kbit/s, {total:.0} s", players.len(), size.0, size.1);
    let names: Vec<String> = players.iter().map(|p| p.name.clone()).collect();
    let mut command = ffmpeg_command();
    command.current_dir(&work);
    let feeds = listen(&mut command, players.into_iter().flat_map(|p| p.inputs).map(|i| i.pieces))?;
    command
        .args(["-filter_complex", &graph, "-map", "[vout]", "-map", "[mix]"])
        .args((0..names.len()).flat_map(|i| ["-map".to_string(), format!("[p{i}]")]))
        .args(ffmpeg::video_args(chosen.0, kbps))
        .args(["-c:a", "aac", "-b:a", &format!("{AUDIO_KBPS}k")])
        .args(std::iter::once("Mix").chain(names.iter().map(String::as_str)).enumerate().flat_map(|(i, n)| {
            [format!("-metadata:s:a:{i}"), format!("title={n}"), format!("-metadata:s:a:{i}"), format!("handler_name={n}")]
        }))
        .args(["-t", &format!("{total:.3}"), "-movflags", FRAGMENTED, "-f", "mp4", PARTIAL]);
    let (stdout, stderr) = begin(core, (room_id.to_string(), set_id.to_string()), &mut command, false)?;
    serve(core, feeds);
    let core = core.clone();
    std::thread::spawn(move || watch(&core, stdout, stderr, &work.join(PARTIAL), &path, total));
    Ok(skipped)
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

fn watch(core: &Core, stdout: ChildStdout, stderr: ChildStderr, partial: &Path, path: &Path, total: f64) {
    let tail = stderr_tail(stderr, ERROR_LINES);
    emit(core, 0.0, false, None);
    read_progress(stdout, |s| emit(core, (s / total * 100.0).clamp(0.0, 100.0), false, None));
    let result = outcome(core, tail, partial, path);
    end(core);
    match result {
        Ok(()) => {
            tracing::info!("export finished: {} ({} B)", path.display(), crate::file_len(path));
            emit(core, 100.0, true, None)
        }
        Err(e) => {
            tracing::warn!("export ended: {e}");
            emit(core, 0.0, true, Some(e))
        }
    }
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

pub fn outcome(core: &Core, tail: JoinHandle<String>, partial: &Path, path: &Path) -> Result<(), String> {
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

pub fn begin(core: &Core, set: (String, String), command: &mut Command, replace: bool) -> Result<(ChildStdout, ChildStderr), String> {
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

pub fn end(core: &Core) {
    *core.export.lock().unwrap() = None;
}

pub fn cancel(core: &Core) {
    if let Some(running) = core.export.lock().unwrap().as_mut() {
        running.cancelled = true;
        running.child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_graph_places_each_player_and_mixes_every_sound() {
        let players = vec![vec![(0.0, true), (0.0, false)], vec![(1.5, true), (2.0, true), (1.5, false)]];
        let g = graph(&players, &grid(2, CANVAS), CANVAS, 60.0);
        assert!(g.starts_with("color=c=black:s=2560x1440:r=30:d=60.000[bg0];"));
        assert!(g.contains("[0:v]setpts=PTS-STARTPTS+0.000/TB"));
        assert!(g.contains("[2:v]setpts=PTS-STARTPTS+1.500/TB"));
        assert!(g.contains("[bg2][v3]overlay=x=1280:y=360:eof_action=pass[bg3]"));
        assert!(!g.contains("[1:v]") && !g.contains("[4:v]"));
        assert!(g.contains("[4:a:0]asetpts=PTS-STARTPTS,adelay=delays=1500:all=1[s4]"));
        assert!(g.contains("[s0][s1]amix=inputs=2:normalize=0:duration=longest,asplit=2[p0][m0];"));
        assert!(g.contains("[s2][s3][s4]amix=inputs=3"));
        assert!(g.contains("drawtext=fontfile=font.ttc:textfile=label-1.txt"));
        assert!(g.ends_with("[bg3]drawtext=fontfile=font.ttc:textfile=label-0.txt:expansion=none:fontsize=28:fontcolor=white:box=1:boxcolor=black@0.6:boxborderw=8:x=16:y=1064-th,drawtext=fontfile=font.ttc:textfile=label-1.txt:expansion=none:fontsize=28:fontcolor=white:box=1:boxcolor=black@0.6:boxborderw=8:x=1296:y=1064-th,format=yuv420p[vout];[m0][m1]amix=inputs=2:normalize=0:duration=longest[mix]"));
        assert!(!g.contains('\''));
    }

    #[test]
    fn the_export_is_sized_and_paced_by_its_sources() {
        assert_eq!(canvas(1, 1280), (1280, 720));
        assert_eq!(canvas(2, 1280), (2560, 720));
        assert_eq!(canvas(3, 1280), (2560, 1440));
        assert_eq!(canvas(4, 1920), CANVAS);
        assert_eq!(source_kbps(&json!({"width": 1280, "kbps": 3000, "codec": "av1"})).round(), (3000.0 / 0.55_f64).round());
        assert_eq!(source_kbps(&json!({"width": 1280})), record::bitrate_kbps(1280, 1.0) as f64);
        let three = [5000.0; 3];
        assert_eq!(export_kbps(&three, canvas(3, 1280), 1.0), 15000);
        assert_eq!(export_kbps(&three, canvas(3, 1280), 0.55), 8250);
        assert_eq!(export_kbps(&[90000.0], canvas(1, 1280), 1.0), record::bitrate_kbps(1280, 1.0));
    }

    #[test]
    fn a_grid_fits_players_in_sixteen_by_nine_cells() {
        assert_eq!(grid(1, CANVAS), vec![(0, 0, 2560, 1440)]);
        assert_eq!(grid(4, CANVAS)[3], (1280, 720, 1280, 720));
        let three = grid(3, CANVAS);
        assert_eq!(three[2], (0, 720, 1280, 720));
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

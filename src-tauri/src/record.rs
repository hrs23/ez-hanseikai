use std::collections::HashMap;
use std::io::Write;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::Emitter;
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, GraphicsCaptureItemType,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use crate::audio::{self, Audio, CHANNELS, SAMPLE_RATE};
use crate::ffmpeg::{self, FRAGMENTED};
use crate::sources::{self, Found, Item};
use crate::{merge, now_us, Core, BELOW_NORMAL, NO_WINDOW};

pub type Err = Box<dyn std::error::Error + Send + Sync>;

pub const FPS: u32 = 30;
const BASE_BITRATE_KBPS: f64 = 6000.0;
const AUDIO_BITRATE: &str = "128k";
const QUEUE_SIZE: &str = "512";
const FRAME_GAP: Duration = Duration::from_millis(30);
const REPEAT_AFTER: Duration = Duration::from_millis(250);
const REPEAT_POLL: Duration = Duration::from_millis(100);
const PCM_FRAME_BYTES: u64 = 4;
const PAD_AFTER_FRAMES: u64 = SAMPLE_RATE as u64 / 50;
const SHARED_MAX_WIDTH: u32 = 1280;
const LEVELS_PER_SECOND: u64 = 30;
const LIVE_SEGMENT_S: u32 = 10;
const LIVE_KEEP: usize = 3;
const SUPERVISE_EVERY: Duration = Duration::from_millis(500);
pub const RESCAN_EVERY: Duration = Duration::from_secs(5);
const CLEAN_EVERY: Duration = Duration::from_secs(LIVE_SEGMENT_S as u64);
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);
const STABLE_AFTER: Duration = Duration::from_secs(60);
const FINISH_WAIT: Duration = Duration::from_secs(10);
const PROBE_WAIT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(20);
pub const GAME: usize = 0;
pub const MIC: usize = 1;

#[derive(Clone, PartialEq)]
pub struct Target {
    pub room: String,
    pub set: Option<String>,
}

#[derive(Default)]
struct Track {
    stream: Option<TcpStream>,
    written: u64,
}

pub struct Sink {
    pub target: Target,
    live_dir: PathBuf,
    live_prefix: String,
    segment: AtomicU32,
    size: (u32, u32),
    meta: Value,
    child: Mutex<Child>,
    video: Mutex<Option<ChildStdin>>,
    tracks: [Mutex<Track>; 2],
    closers: Mutex<Vec<TcpStream>>,
    start: OnceLock<Instant>,
    closed: AtomicBool,
    dead: AtomicBool,
    born: Instant,
}

#[derive(Default, Clone)]
pub struct Names {
    pub kind: String,
    pub source: String,
    pub game: String,
    pub mic_id: String,
    pub mic: String,
}

struct Video {
    control: CaptureControl<Capture, Err>,
    key: usize,
}

pub struct Engine {
    pub source: Mutex<Value>,
    pub mic: Mutex<String>,
    pub mix: Mutex<Value>,
    pub source_lost: AtomicBool,
    pub status: Mutex<String>,
    pub names: Mutex<Names>,
    pub sink: Mutex<Option<Arc<Sink>>>,
    pub audio: [Mutex<Option<Audio>>; 2],
    pub peaks: [AtomicU32; 2],
    video: Mutex<Option<Video>>,
    canvas: Mutex<Option<(u32, u32)>>,
    last: Mutex<(Vec<u8>, (u32, u32), Instant)>,
    failures: AtomicU32,
    retry_at: Mutex<Instant>,
}

impl Engine {
    pub fn new(settings: &Value) -> Self {
        Self {
            source: Mutex::new(settings["source"].clone()),
            mic: Mutex::new(settings["mic"].as_str().unwrap_or_default().to_string()),
            mix: Mutex::new(if settings["mix"].is_object() { settings["mix"].clone() } else { json!({}) }),
            source_lost: AtomicBool::new(false),
            status: Mutex::new(String::new()),
            names: Mutex::new(Names::default()),
            sink: Mutex::new(None),
            audio: [Mutex::new(None), Mutex::new(None)],
            peaks: [AtomicU32::new(0), AtomicU32::new(0)],
            video: Mutex::new(None),
            canvas: Mutex::new(None),
            last: Mutex::new((Vec::new(), (0, 0), Instant::now())),
            failures: AtomicU32::new(0),
            retry_at: Mutex::new(Instant::now()),
        }
    }

    pub fn mix_key(&self, track: usize) -> String {
        if track == GAME { "game".into() } else { format!("mic:{}", self.names.lock().unwrap().mic_id) }
    }

    pub fn gain(&self, track: usize) -> (f32, bool) {
        let key = self.mix_key(track);
        let mix = self.mix.lock().unwrap();
        (mix[&key]["volume"].as_f64().unwrap_or(1.0) as f32, mix[&key]["muted"].as_bool().unwrap_or(false))
    }

    pub fn live_file(&self) -> Option<String> {
        let sink = self.sink.lock().unwrap().clone()?;
        sink.live_file()
    }

    fn failed(&self, born: Instant, why: &str) {
        let count = if born.elapsed() > STABLE_AFTER { 1 } else { self.failures.load(Ordering::Relaxed) + 1 };
        self.failures.store(count, Ordering::Relaxed);
        let delay = RETRY_FIRST.saturating_mul(1 << (count - 1).min(5)).min(RETRY_MAX);
        *self.retry_at.lock().unwrap() = Instant::now() + delay;
        *self.status.lock().unwrap() = format!("Capture problem: {why}. Retrying in {} s.", delay.as_secs());
        tracing::warn!("capture problem: {why}");
    }
}

pub fn capture_json(core: &Core, live: Option<Value>) -> Value {
    let e = &core.engine;
    let mix = |track| {
        let key = e.mix_key(track);
        let m = e.mix.lock().unwrap()[&key].clone();
        json!({"volume": m["volume"].as_f64().unwrap_or(1.0), "muted": m["muted"].as_bool().unwrap_or(false)})
    };
    json!({
        "source": e.source.lock().unwrap().clone(),
        "source_kind": e.names.lock().unwrap().kind.clone(),
        "source_lost": e.source_lost.load(Ordering::Relaxed),
        "sharing": core.sharing_live(),
        "status": e.status.lock().unwrap().clone(),
        "mic": e.mic.lock().unwrap().clone(),
        "mix": {"game": mix(GAME), "mic": mix(MIC)},
        "live": live,
    })
}

pub fn live_name(name: &str) -> Option<(String, i64, u32)> {
    let (rest, n) = name.strip_suffix(".mp4")?.rsplit_once('-')?;
    let (who, ms) = rest.rsplit_once('-')?;
    Some((who.to_string(), ms.parse().ok()?, n.parse().ok()?))
}

fn slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn want(core: &Core) -> Option<Target> {
    let e = &core.engine;
    if e.source.lock().unwrap().is_null() || core.disk_full() {
        return None;
    }
    if let Some(s) = core.active_set.lock().unwrap().as_ref() {
        return Some(Target { room: s.room.clone(), set: Some(s.id.clone()) });
    }
    let room = core.room_id()?;
    (core.share.load(Ordering::Relaxed) && !e.source_lost.load(Ordering::Relaxed)).then_some(Target { room, set: None })
}

fn usable_codecs(core: &Core) -> Vec<String> {
    let mut usable = core.codecs.lock().unwrap().clone();
    for p in core.peers.lock().unwrap().iter().filter(|p| p.conn.is_some()) {
        usable.retain(|c| p.codecs.contains(c));
    }
    usable
}

fn encode_command(size: (u32, u32), chosen: (&str, &str, f64), inputs: [u16; 2], outputs: &str) -> Command {
    let (rate, channels, fps) = (SAMPLE_RATE.to_string(), CHANNELS.to_string(), FPS.to_string());
    let mut command = Command::new(ffmpeg::path());
    command
        .args(["-hide_banner", "-loglevel", "warning", "-y"])
        .args(["-use_wallclock_as_timestamps", "1", "-thread_queue_size", QUEUE_SIZE])
        .args(["-f", "rawvideo", "-pix_fmt", "bgra", "-video_size", &format!("{}x{}", size.0, size.1), "-i", "-"]);
    for port in inputs {
        command.args(["-thread_queue_size", QUEUE_SIZE, "-f", "s16le", "-ar", &rate, "-ac", &channels, "-i", &format!("tcp://127.0.0.1:{port}")]);
    }
    command
        .args(["-map", "0:v", "-map", "1:a", "-map", "2:a"])
        .args(["-vf", &format!("scale={}:-2:flags=fast_bilinear,format=yuv420p", SHARED_MAX_WIDTH.min(size.0) & !1), "-r", &fps])
        .args(["-c:v", chosen.0, "-bf", "0", "-g", &fps, "-b:v", &format!("{}k", (BASE_BITRATE_KBPS * chosen.2) as u32)]);
    if chosen.0 == ffmpeg::FALLBACK.0 {
        command.args(["-preset", "ultrafast"]);
    }
    if chosen.1 == "hevc" {
        command.args(["-tag:v", "hvc1"]);
    }
    command
        .args(["-force_key_frames", "expr:gte(t,n_forced*1)"])
        .args(["-c:a", "aac", "-b:a", AUDIO_BITRATE, "-metadata:s:a:0", "title=Game", "-metadata:s:a:1", "title=Microphone"])
        .args(["-flags", "+global_header", "-f", "tee"])
        .arg(outputs)
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(NO_WINDOW | BELOW_NORMAL);
    command
}

impl Sink {
    fn start(core: &Arc<Core>, target: Target, size: (u32, u32)) -> Result<Arc<Self>, Err> {
        if !ffmpeg::present() {
            return Err("ffmpeg is not installed yet".into());
        }
        let id = core.ep.id().to_string();
        let live_dir = core.live_dir(&target.room);
        let live_prefix = format!("{id}-{}-", now_us() / 1000);
        std::fs::create_dir_all(&live_dir)?;
        let mut outputs = format!(
            "[f=segment:segment_time={LIVE_SEGMENT_S}:reset_timestamps=1:segment_format=mp4:segment_format_options=movflags={FRAGMENTED}]{}/{live_prefix}%d.mp4",
            slashes(&live_dir)
        );
        if let Some(set) = &target.set {
            std::fs::create_dir_all(core.set_dir(&target.room, set))?;
            outputs += &format!("|[f=mp4:movflags={FRAGMENTED}:flush_packets=1]{}", slashes(&core.own_file(&target.room, set)));
        }
        let chosen = ffmpeg::pick(&usable_codecs(core));
        let listeners = [TcpListener::bind("127.0.0.1:0")?, TcpListener::bind("127.0.0.1:0")?];
        let inputs = [listeners[0].local_addr()?.port(), listeners[1].local_addr()?.port()];
        let mut child = encode_command(size, chosen, inputs, &outputs).spawn()?;
        if let Some(stderr) = child.stderr.take() {
            thread::spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
                    tracing::warn!("ffmpeg: {line}");
                }
            });
        }
        let names = core.engine.names.lock().unwrap().clone();
        let mut meta = json!({
            "source_kind": names.kind, "source": names.source, "width": size.0, "height": size.1, "fps": FPS,
            "encoder": chosen.0, "codec": chosen.1, "audio": names.game, "mic": names.mic, "version": crate::VERSION,
        });
        if let Some(set) = &target.set {
            let by = core.active_set.lock().unwrap().as_ref().map(|s| s.by.clone()).unwrap_or_default();
            merge(&mut meta, &json!({"set": set, "room": target.room, "file": format!("{id}.mp4"), "by": by}));
        }
        let sink = Arc::new(Self {
            target,
            live_dir,
            live_prefix,
            segment: AtomicU32::new(0),
            size,
            meta,
            video: Mutex::new(child.stdin.take()),
            child: Mutex::new(child),
            tracks: [Mutex::new(Track::default()), Mutex::new(Track::default())],
            closers: Mutex::new(Vec::new()),
            start: OnceLock::new(),
            closed: AtomicBool::new(false),
            dead: AtomicBool::new(false),
            born: Instant::now(),
        });
        for (track, listener) in listeners.into_iter().enumerate() {
            let sink = sink.clone();
            thread::spawn(move || sink.accept(track, listener));
        }
        Ok(sink)
    }

    fn live_file(&self) -> Option<String> {
        let name = |n: u32| format!("{}{n}.mp4", self.live_prefix);
        let mut n = self.segment.load(Ordering::Relaxed);
        while self.live_dir.join(name(n + 1)).exists() {
            n += 1;
        }
        self.segment.store(n, Ordering::Relaxed);
        Some(name(n)).filter(|f| self.live_dir.join(f).exists())
    }

    fn accept(&self, track: usize, listener: TcpListener) {
        if listener.set_nonblocking(true).is_err() {
            return;
        }
        while !self.closed.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    if let Ok(closer) = stream.try_clone() {
                        self.closers.lock().unwrap().push(closer);
                    }
                    self.tracks[track].lock().unwrap().stream = Some(stream);
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(POLL),
                Err(_) => return,
            }
        }
    }

    fn write_video(&self, core: &Core, data: &[u8]) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        let mut pipe = self.video.lock().unwrap();
        let Some(out) = pipe.as_mut() else { return };
        if self.start.set(Instant::now()).is_ok() && self.target.set.is_some() {
            let mut record = json!({"k": "rec", "start_us": now_us()});
            merge(&mut record, &self.meta);
            core.append_log(&record);
            if let Some(active) = core.active_set.lock().unwrap().clone() {
                crate::room::write_set_json(core, &active, false);
            }
        }
        if out.write_all(data).is_err() {
            *pipe = None;
            self.dead.store(true, Ordering::Relaxed);
        }
    }

    pub fn write_audio(&self, track: usize, pcm: &[u8]) {
        let Some(start) = self.start.get() else { return };
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        let mut t = self.tracks[track].lock().unwrap();
        let due = (start.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as u64;
        let lag = due.saturating_sub(t.written);
        let pad = if (t.written == 0 || pcm.is_empty()) && lag > PAD_AFTER_FRAMES { lag } else { 0 };
        let Some(stream) = t.stream.as_mut() else { return };
        if pad == 0 && pcm.is_empty() {
            return;
        }
        let sent = (pad == 0 || stream.write_all(&vec![0u8; (pad * PCM_FRAME_BYTES) as usize]).is_ok()) && stream.write_all(pcm).is_ok();
        if !sent {
            t.stream = None;
            if !self.closed.load(Ordering::Relaxed) {
                self.dead.store(true, Ordering::Relaxed);
            }
            return;
        }
        t.written += pad + pcm.len() as u64 / PCM_FRAME_BYTES;
    }

    fn finish(&self) {
        self.closed.store(true, Ordering::Relaxed);
        for stream in self.closers.lock().unwrap().drain(..) {
            let _ = stream.shutdown(Shutdown::Both);
        }
        let deadline = Instant::now() + FINISH_WAIT;
        while Instant::now() < deadline {
            if let Ok(mut pipe) = self.video.try_lock() {
                *pipe = None;
                break;
            }
            thread::sleep(POLL);
        }
        let mut child = self.child.lock().unwrap();
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(POLL);
        }
        tracing::warn!("ffmpeg did not finish in time, killing it");
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn current_sink(core: &Arc<Core>, canvas: Option<(u32, u32)>) -> Option<Arc<Sink>> {
    let e = &core.engine;
    let want = want(core);
    let mut slot = e.sink.lock().unwrap();
    let stale = slot.as_ref().map(|sink| {
        let resized = sink.target.set.is_none() && canvas.is_some_and(|c| c != sink.size);
        (sink.dead.load(Ordering::Relaxed), resized || Some(&sink.target) != want.as_ref())
    });
    if let Some((dead, changed)) = stale.filter(|(dead, changed)| *dead || *changed) {
        let old = slot.take().unwrap();
        if dead && !changed {
            e.failed(old.born, "the video encoder stopped");
        }
        thread::spawn(move || old.finish());
    }
    if slot.is_none() {
        let (target, size) = (want?, canvas?);
        let done = target.set.as_ref().is_some_and(|set| core.own_file(&target.room, set).exists());
        if done || Instant::now() < *e.retry_at.lock().unwrap() {
            return None;
        }
        match Sink::start(core, target, size) {
            Ok(sink) => *slot = Some(sink),
            Err(err) => e.failed(Instant::now(), &err.to_string()),
        }
    }
    slot.clone()
}

pub fn end_set(core: &Core) -> Option<crate::room::ActiveSet> {
    let (active, sink) = {
        let mut slot = core.engine.sink.lock().unwrap();
        let active = core.active_set.lock().unwrap().take();
        let sink = if slot.as_ref().is_some_and(|s| s.target.set.is_some()) { slot.take() } else { None };
        (active, sink)
    };
    if let Some(sink) = sink {
        sink.finish();
    }
    active
}

fn fit(src: &[u8], (w, h): (u32, u32), (cw, ch): (u32, u32), out: &mut Vec<u8>) {
    out.clear();
    out.resize((cw * ch * 4) as usize, 0);
    let scale = (cw as f64 / w as f64).min(ch as f64 / h as f64);
    let (dw, dh) = (((w as f64 * scale) as u32).clamp(1, cw) as usize, ((h as f64 * scale) as u32).clamp(1, ch) as usize);
    let (ox, oy) = ((cw as usize - dw) / 2, (ch as usize - dh) / 2);
    let (w, h) = (w as usize, h as usize);
    for y in 0..dh {
        let row = y * h / dh * w * 4;
        let base = ((oy + y) * cw as usize + ox) * 4;
        for x in 0..dw {
            let from = row + x * w / dw * 4;
            out[base + x * 4..base + x * 4 + 4].copy_from_slice(&src[from..from + 4]);
        }
    }
}

struct Probe(Arc<AtomicBool>);

impl GraphicsCaptureApiHandler for Probe {
    type Flags = Arc<AtomicBool>;
    type Error = Err;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self(ctx.flags))
    }

    fn on_frame_arrived(&mut self, _frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), Self::Error> {
        self.0.store(true, Ordering::Relaxed);
        Ok(())
    }
}

struct Capture {
    core: Arc<Core>,
    raw: Vec<u8>,
    fitted: Vec<u8>,
    last: Instant,
}

impl GraphicsCaptureApiHandler for Capture {
    type Flags = Arc<Core>;
    type Error = Err;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self { core: ctx.flags, raw: Vec::new(), fitted: Vec::new(), last: Instant::now() - FRAME_GAP })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), Self::Error> {
        let (w, h) = (frame.width(), frame.height());
        if self.last.elapsed() < FRAME_GAP || w < 2 || h < 2 {
            return Ok(());
        }
        self.last = Instant::now();
        let buffer = frame.buffer()?;
        let data = buffer.as_nopadding_buffer(&mut self.raw);
        let e = &self.core.engine;
        let canvas = *e.canvas.lock().unwrap().get_or_insert((w & !1, h & !1));
        let sink = current_sink(&self.core, Some(canvas));
        let size = sink.as_ref().map_or(canvas, |s| s.size);
        let same = (w, h) == size;
        if !same {
            fit(data, (w, h), size, &mut self.fitted);
        }
        if let Some(sink) = &sink {
            sink.write_video(&self.core, if same { data } else { self.fitted.as_slice() });
        }
        let mut last = e.last.lock().unwrap();
        if same {
            last.0.clear();
            last.0.extend_from_slice(data);
        } else {
            std::mem::swap(&mut last.0, &mut self.fitted);
        }
        last.1 = size;
        last.2 = Instant::now();
        Ok(())
    }
}

fn settings<F, T: TryInto<GraphicsCaptureItemType>>(item: T, border: DrawBorderSettings, flags: F) -> Settings<F, T> {
    Settings::new(
        item,
        CursorCaptureSettings::Default,
        border,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    )
}

type Started<H> = (Option<String>, Result<CaptureControl<H, Err>, String>);

fn start_capture<H, T>(item: T, flags: H::Flags) -> Started<H>
where
    H: GraphicsCaptureApiHandler<Error = Err> + Send + 'static,
    H::Flags: Send + Clone,
    T: TryInto<GraphicsCaptureItemType> + Send + Copy + 'static,
{
    match H::start_free_threaded(settings(item, DrawBorderSettings::WithoutBorder, flags.clone())) {
        Ok(control) => (None, Ok(control)),
        Err(e) => (Some(e.to_string()), H::start_free_threaded(settings(item, DrawBorderSettings::Default, flags)).map_err(|e| e.to_string())),
    }
}

pub fn check_capture() -> Vec<(&'static str, String)> {
    let mut lines = Vec::new();
    let got = Arc::new(AtomicBool::new(false));
    let Ok(primary) = Monitor::primary() else {
        return vec![("fail", "no display found".to_string())];
    };
    let (refused, control) = start_capture::<Probe, _>(primary, got.clone());
    if let Some(e) = refused {
        lines.push(("warn", format!("capture without border was refused ({e}); falling back to the default border")));
    }
    match control {
        Err(e) => lines.push(("fail", format!("screen capture could not start: {e}"))),
        Ok(control) => {
            let until = Instant::now() + PROBE_WAIT;
            while !got.load(Ordering::Relaxed) && Instant::now() < until {
                thread::sleep(POLL);
            }
            let _ = control.stop();
            if got.load(Ordering::Relaxed) {
                lines.push(("ok", "screen capture started and a frame arrived".into()));
            } else {
                lines.push(("warn", format!("screen capture started but no frame arrived in {} s (idle or locked screen)", PROBE_WAIT.as_secs())));
            }
        }
    }
    lines
}

fn repeat(core: Arc<Core>) {
    loop {
        thread::sleep(REPEAT_POLL);
        let canvas = *core.engine.canvas.lock().unwrap();
        let sink = current_sink(&core, canvas);
        let mut last = core.engine.last.lock().unwrap();
        if last.0.is_empty() || last.2.elapsed() < REPEAT_AFTER {
            continue;
        }
        if let Some(sink) = sink.filter(|s| s.size == last.1) {
            sink.write_video(&core, &last.0);
        }
        last.2 = Instant::now() - (REPEAT_AFTER - REPEAT_POLL);
    }
}

fn meters(core: Arc<Core>) {
    let mut quiet = false;
    loop {
        thread::sleep(Duration::from_millis(1000 / LEVELS_PER_SECOND));
        let [game, mic] = [GAME, MIC].map(|t| f32::from_bits(core.engine.peaks[t].swap(0, Ordering::Relaxed)));
        let silent = game == 0.0 && mic == 0.0;
        if !(silent && quiet) {
            let _ = core.app.emit("levels", json!({"game": game, "mic": mic}));
        }
        quiet = silent;
    }
}

fn clean_live(core: &Core) {
    let Ok(rooms) = std::fs::read_dir(core.dir.join("rooms")) else { return };
    for room in rooms.flatten() {
        let Ok(files) = std::fs::read_dir(room.path().join("live")) else { continue };
        let mut all: Vec<(String, i64, u32, String)> = files
            .flatten()
            .map(|f| f.file_name().to_string_lossy().to_string())
            .filter_map(|n| live_name(&n).map(|(who, ms, i)| (who, ms, i, n)))
            .collect();
        all.sort_by(|a, b| (&b.0, b.1, b.2).cmp(&(&a.0, a.1, a.2)));
        let mut kept: HashMap<String, usize> = HashMap::new();
        for (who, _, _, name) in &all {
            let count = kept.entry(who.clone()).or_default();
            *count += 1;
            if *count > LIVE_KEEP {
                let _ = std::fs::remove_file(room.path().join("live").join(name));
            }
        }
    }
}

fn rescan(core: &Core, source: &Value) -> Option<Found> {
    let e = &core.engine;
    let found = sources::find(core, source);
    e.source_lost.store(!source.is_null() && found.is_none(), Ordering::Relaxed);
    let mut names = e.names.lock().unwrap();
    names.kind = found.as_ref().map(|f| f.kind.to_string()).unwrap_or_default();
    names.source = found.as_ref().map(|f| f.name.clone()).unwrap_or_default();
    names.game = found.as_ref().map(|f| f.game.0.clone()).unwrap_or_default();
    found
}

fn ensure_video(core: &Arc<Core>, wanted: Option<&Found>) {
    let e = &core.engine;
    let mut video = e.video.lock().unwrap();
    if video.as_ref().is_some_and(|v| v.control.is_finished() || wanted.is_none_or(|f| f.key != v.key)) {
        let v = video.take().unwrap();
        let closed = v.control.is_finished();
        let _ = v.control.stop();
        if !closed {
            e.last.lock().unwrap().0.clear();
            *e.canvas.lock().unwrap() = None;
        }
    }
    let Some(f) = wanted.filter(|_| video.is_none()) else { return };
    if Instant::now() < *e.retry_at.lock().unwrap() {
        return;
    }
    let (refused, started) = match f.item {
        Item::Window(w) => start_capture::<Capture, _>(w, core.clone()),
        Item::Display(m) => start_capture::<Capture, _>(m, core.clone()),
    };
    if let Some(why) = refused {
        tracing::warn!("capture without border refused ({why}), using the default border");
    }
    match started {
        Ok(control) => {
            *e.canvas.lock().unwrap() = None;
            *video = Some(Video { control, key: f.key });
        }
        Err(err) => e.failed(Instant::now(), &format!("screen capture could not start: {err}")),
    }
}

fn check_sink(core: &Core) {
    let e = &core.engine;
    let Some(sink) = e.sink.lock().unwrap().clone() else { return };
    if matches!(sink.child.lock().unwrap().try_wait(), Ok(Some(_))) {
        sink.dead.store(true, Ordering::Relaxed);
    }
    if sink.born.elapsed() > STABLE_AFTER {
        e.failures.store(0, Ordering::Relaxed);
        e.status.lock().unwrap().clear();
    }
}

pub fn supervise(core: Arc<Core>) {
    let c = core.clone();
    thread::spawn(move || repeat(c));
    let c = core.clone();
    thread::spawn(move || meters(c));
    let e = &core.engine;
    let mut scanned: Option<(Value, Instant)> = None;
    let mut found: Option<Found> = None;
    let mut cleaned: Option<Instant> = None;
    loop {
        let source = e.source.lock().unwrap().clone();
        let ended = e.video.lock().unwrap().as_ref().is_some_and(|v| v.control.is_finished());
        if ended || scanned.as_ref().is_none_or(|(s, at)| *s != source || at.elapsed() > RESCAN_EVERY) {
            found = rescan(&core, &source);
            scanned = Some((source, Instant::now()));
        }
        audio::ensure(&core, GAME, found.as_ref().map_or(0, |f| f.game.1).to_string());
        let mic = e.mic.lock().unwrap().clone();
        audio::ensure(&core, MIC, mic);
        ensure_video(&core, found.as_ref().filter(|_| want(&core).is_some() && ffmpeg::encoders_ready()));
        check_sink(&core);
        if cleaned.is_none_or(|t| t.elapsed() > CLEAN_EVERY) {
            clean_live(&core);
            cleaned = Some(Instant::now());
        }
        thread::sleep(SUPERVISE_EVERY);
    }
}

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::Emitter;
use windows_capture::capture::CaptureControl;

use crate::audio::{self, Audio, CHANNELS, SAMPLE_RATE};
use crate::capture::{start_capture, Capture};
use crate::ffmpeg::{self, FRAGMENTED};
use crate::room::{self, capture_id, part_of, seg_name, INIT, RECORDINGS, SERIES};
use crate::sources::{self, Found, Item};
use crate::noise::Repeats;
use crate::proc::{self, Process, BELOW_NORMAL};
use crate::{clock, data, debug, hex, hw, media, merge, net, now_us, write_whole, Core};

pub type Err = Box<dyn std::error::Error + Send + Sync>;

pub const FPS: u32 = 30;
pub(crate) const FRAME_GAP: Duration = Duration::from_millis(30);
pub const FRAGMENTS_PER_SEGMENT: u32 = 4;
const H264_BITS_PER_PIXEL: f64 = 0.2;
pub const AUDIO_KBPS: u32 = 128;
const QUEUE_SIZE: &str = "512";
const VOICE_FRAGMENT_US: &str = "1000000";
const REPEAT_AFTER: Duration = Duration::from_millis(250);
const STALLED_AFTER: Duration = Duration::from_secs(5);
const FFMPEG_REPEAT_EVERY: Duration = Duration::from_secs(60);
const REPEAT_POLL: Duration = Duration::from_millis(100);
const PCM_FRAME_BYTES: u64 = 4;
const PAD_AFTER_FRAMES: u64 = SAMPLE_RATE as u64 / 50;
pub const QUALITIES: [(&str, u32); 3] = [("light", 960), ("standard", 1280), ("high", 1920)];
const WIDE: (u32, u32) = (16, 9);
const LIVE_LAG: u32 = 2;
pub const THIN_WINDOW: Duration = Duration::from_secs(60);
const STEP_EVERY: Duration = Duration::from_secs(60);
const LEVELS_PER_SECOND: u64 = 30;
const SPEAK_LEVEL: f32 = 0.02;
const SPEAK_HANG: Duration = Duration::from_millis(300);
const READ_CHUNK: usize = 64 << 10;
const SUPERVISE_EVERY: Duration = Duration::from_millis(500);
pub const RESCAN_EVERY: Duration = Duration::from_secs(5);
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);
const STABLE_AFTER: Duration = Duration::from_secs(60);
const STALL_AFTER: Duration = Duration::from_secs(10);
const FEED_GAP: Duration = Duration::from_secs(2);
const DROP_AFTER: u32 = 3;
const FINISH_WAIT: Duration = Duration::from_secs(10);
const CUT_WAIT: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(20);
const RENAME_WAIT: Duration = Duration::from_millis(500);
const RENAME_GAP: Duration = Duration::from_millis(50);
pub const APP: usize = 0;
pub const MIC: usize = 1;
pub const MIX_KEYS: [&str; 2] = ["app", "mic"];
const TEST_SIZE: (u32, u32) = (1280, 720);
const TEST_TONES_HZ: [u32; 2] = [440, 880];

fn level(id: &str) -> usize {
    QUALITIES.iter().position(|q| q.0 == id).unwrap_or(1)
}

pub fn quality_width(id: &str) -> u32 {
    QUALITIES[level(id)].1
}

pub fn scaled(size: (u32, u32), width: u32) -> (u32, u32) {
    let w = width.min(size.0) & !1;
    let h = ((size.1 as u64 * w as u64 + size.0 as u64 / 2) / size.0.max(1) as u64) as u32 & !1;
    (w, h.max(2))
}

pub fn bitrate_kbps(width: u32, factor: f64) -> u32 {
    let pixels = width as f64 * width as f64 * WIDE.1 as f64 / WIDE.0 as f64;
    (pixels * FPS as f64 * H264_BITS_PER_PIXEL * factor / 1000.0).round() as u32
}

#[derive(Default)]
struct Track {
    stream: Option<TcpStream>,
    written: u64,
}

struct Open {
    n: u32,
    set: Option<String>,
    dir: PathBuf,
    file: std::fs::File,
    hasher: Sha256,
    size: u64,
    t_us: i64,
    count: u32,
    stopped: bool,
}

#[derive(Default, Clone, Copy)]
struct Watch {
    owed: Option<Instant>,
    fed: Option<Instant>,
    writing: bool,
    answered: Option<Instant>,
}

impl Watch {
    fn fed(&mut self, now: Instant) {
        if self.fed.is_none_or(|t| now.saturating_duration_since(t) > FEED_GAP) {
            self.owed = Some(now);
        } else {
            self.owed.get_or_insert(now);
        }
        self.fed = Some(now);
        self.writing = true;
    }

    fn wrote(&mut self) {
        self.writing = false;
    }

    fn answer(&mut self, now: Instant) {
        self.owed = None;
        self.answered.get_or_insert(now);
    }

    fn stalled(&self, now: Instant) -> bool {
        let feeding = self.writing || self.fed.is_some_and(|t| now.saturating_duration_since(t) <= FEED_GAP);
        feeding && self.owed.is_some_and(|t| now.saturating_duration_since(t) > STALL_AFTER)
    }

    fn working(&self, now: Instant) -> bool {
        self.answered.is_some() && !self.stalled(now)
    }

    fn stable(&self, now: Instant) -> bool {
        self.working(now) && self.answered.is_some_and(|t| now.saturating_duration_since(t) > STABLE_AFTER)
    }
}

fn strike(last: (&'static str, u32), encoder: &'static str) -> (&'static str, u32) {
    (encoder, if last.0 == encoder { last.1 + 1 } else { 1 })
}

#[derive(Default)]
struct Series {
    scales: HashMap<u32, u32>,
    n: u32,
    open: Option<Open>,
    recent: VecDeque<(u32, Option<String>)>,
    last_us: i64,
    step_us: i64,
}

pub struct Sink {
    pub room: String,
    pub capture: String,
    pub voice: bool,
    out: (u32, u32),
    quality: String,
    kbps: u32,
    encoder: &'static str,
    codec: &'static str,
    meta: Value,
    child: Mutex<Process>,
    frame: Mutex<Option<Arc<Vec<u8>>>>,
    ready: Condvar,
    watch: Mutex<Watch>,
    tracks: [Mutex<Track>; 2],
    outputs: [Mutex<Series>; 2],
    inits: Mutex<[Value; 2]>,
    init: Mutex<[Option<Vec<u8>>; 2]>,
    readers: Mutex<Vec<JoinHandle<()>>>,
    closers: Mutex<Vec<TcpStream>>,
    start: OnceLock<(Instant, i64)>,
    cap_logged: Mutex<Option<[Value; 2]>>,
    closed: AtomicBool,
    dead: AtomicBool,
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
    control: Option<CaptureControl<Capture, Err>>,
    key: usize,
}

impl Video {
    fn finished(&self) -> bool {
        self.control.as_ref().is_some_and(|c| c.is_finished())
    }
}

pub struct Live {
    pub room: String,
    pub capture: String,
    pub start_us: Option<i64>,
    pub recent: [Vec<(u32, Option<String>)>; 2],
}

#[derive(Debug, PartialEq)]
pub enum LiveStep {
    Wait,
    Serve,
    Stop,
    Behind,
}

pub fn live_step(recent: &[(u32, Option<String>)], wanted: Option<&str>, n: u32) -> LiveStep {
    match recent.last() {
        Some((newest, _)) if n <= *newest => {
            if n + LIVE_LAG <= *newest {
                LiveStep::Behind
            } else if recent.iter().any(|(k, set)| *k == n && set.as_deref() == wanted) {
                LiveStep::Serve
            } else {
                LiveStep::Stop
            }
        }
        _ => LiveStep::Wait,
    }
}

pub struct Engine {
    pub source: Mutex<Value>,
    pub mic: Mutex<String>,
    pub mix: Mutex<Value>,
    pub quality: Mutex<String>,
    pub cap: Mutex<Option<(usize, Instant)>>,
    pub source_lost: AtomicBool,
    pub status: Mutex<String>,
    pub names: Mutex<Names>,
    pub sink: Mutex<Option<Arc<Sink>>>,
    pub audio: [Mutex<Option<Audio>>; 2],
    pub peaks: [AtomicU32; 2],
    video: Mutex<Option<Video>>,
    pub(crate) canvas: Mutex<Option<(u32, u32)>>,
    pub(crate) last: Mutex<(Arc<Vec<u8>>, (u32, u32), Instant)>,
    pub(crate) fresh: Mutex<Instant>,
    failures: AtomicU32,
    strikes: Mutex<(&'static str, u32)>,
    retry_at: Mutex<Instant>,
    rec_lock: Mutex<()>,
    finishing: Mutex<Vec<JoinHandle<()>>>,
}

impl Engine {
    pub fn new(settings: &Value) -> Self {
        Self {
            source: Mutex::new(Value::Null),
            mic: Mutex::new(settings["mic"].as_str().unwrap_or_default().to_string()),
            mix: Mutex::new(starting_mix(&settings["mix"])),
            quality: Mutex::new(settings["quality"].as_str().unwrap_or(QUALITIES[1].0).to_string()),
            cap: Mutex::new(None),
            source_lost: AtomicBool::new(false),
            status: Mutex::new(String::new()),
            names: Mutex::new(Names::default()),
            sink: Mutex::new(None),
            audio: [Mutex::new(None), Mutex::new(None)],
            peaks: [AtomicU32::new(0), AtomicU32::new(0)],
            video: Mutex::new(None),
            canvas: Mutex::new(None),
            last: Mutex::new((Arc::new(Vec::new()), (0, 0), Instant::now())),
            fresh: Mutex::new(Instant::now()),
            failures: AtomicU32::new(0),
            strikes: Mutex::new(("", 0)),
            retry_at: Mutex::new(Instant::now()),
            rec_lock: Mutex::new(()),
            finishing: Mutex::new(Vec::new()),
        }
    }

    pub fn gain(&self, track: usize) -> (f64, bool) {
        let mix = self.mix.lock().unwrap();
        (mix[MIX_KEYS[track]]["volume"].as_f64().unwrap_or(1.0), mix[MIX_KEYS[track]]["muted"].as_bool().unwrap_or(false))
    }

    pub fn quality_now(&self) -> String {
        let chosen = level(&self.quality.lock().unwrap());
        QUALITIES[self.cap.lock().unwrap().map_or(chosen, |(cap, _)| cap.min(chosen))].0.to_string()
    }

    pub fn live(&self) -> Option<Live> {
        let sink = self.sink.lock().unwrap().clone()?;
        let recent = [0, 1].map(|i| sink.outputs[i].lock().unwrap().recent.iter().cloned().collect());
        Some(Live { room: sink.room.clone(), capture: sink.capture.clone(), start_us: sink.start.get().map(|s| s.1), recent })
    }

    pub fn working(&self) -> bool {
        self.sink.lock().unwrap().clone().is_some_and(|s| !s.dead.load(Ordering::Relaxed) && s.watch.lock().unwrap().working(Instant::now()))
    }

    fn encoder_died(&self, sink: &Sink) {
        let stable = sink.watch.lock().unwrap().stable(Instant::now());
        let (encoder, count) = {
            let mut strikes = self.strikes.lock().unwrap();
            *strikes = strike(if stable { ("", 0) } else { *strikes }, sink.encoder);
            *strikes
        };
        if count >= DROP_AFTER && ffmpeg::drop_encoder(encoder) {
            tracing::warn!("{encoder} failed {count} times, switching to the next encoder");
            self.failures.store(0, Ordering::Relaxed);
        }
        self.failed(stable, "the video encoder stopped");
    }

    fn failed(&self, stable: bool, why: &str) {
        let count = if stable { 1 } else { self.failures.load(Ordering::Relaxed) + 1 };
        self.failures.store(count, Ordering::Relaxed);
        let delay = RETRY_FIRST.saturating_mul(1 << (count - 1).min(5)).min(RETRY_MAX);
        *self.retry_at.lock().unwrap() = Instant::now() + delay;
        *self.status.lock().unwrap() = format!("Capture problem: {why}. Retrying in {} s.", delay.as_secs());
        tracing::warn!("capture problem: {why}");
    }
}

pub fn thin(receivers: &[[Option<Instant>; 2]], now: Instant) -> bool {
    !receivers.is_empty() && receivers.iter().all(|drops| drops.iter().all(|d| d.is_some_and(|d| now.duration_since(d) < THIN_WINDOW)))
}

pub fn lower_if_thin(core: &Core) {
    let now = Instant::now();
    let receivers: Vec<[Option<Instant>; 2]> = core.peers.lock().unwrap().iter().filter(|p| p.picture() && p.served.is_some_and(|s| now.duration_since(s) < THIN_WINDOW)).map(|p| p.drops).collect();
    if !thin(&receivers, now) || core.any_recording() {
        return;
    }
    let e = &core.engine;
    let running = level(&e.quality_now());
    let mut cap = e.cap.lock().unwrap();
    if running == 0 || cap.is_some_and(|(_, at)| now.duration_since(at) < STEP_EVERY) {
        return;
    }
    *cap = Some((running - 1, now));
    drop(cap);
    debug::event(core, now, format!("quality lowered to {}: live sending fell behind for every receiver", QUALITIES[running - 1].0));
}

pub fn starting_mix(saved: &Value) -> Value {
    let mut mix = json!({});
    for key in MIX_KEYS {
        mix[key] = json!({"volume": saved[key]["volume"].as_f64().filter(|v| (0.0..=1.0).contains(v)).unwrap_or(1.0), "muted": key == MIX_KEYS[MIC] || saved[key]["muted"] == true});
    }
    mix
}

fn stalled(window: bool, capturing: bool, quiet: Duration) -> bool {
    window && capturing && quiet >= STALLED_AFTER
}

/// The log line when the captured window stops or starts sending frames again; `since` holds when it stopped.
pub fn stall_line(since: &mut Option<Instant>, stalled: bool, quiet: Duration, now: Instant) -> Option<(tracing::Level, String)> {
    match (*since, stalled) {
        (None, true) => {
            *since = Some(now.checked_sub(quiet).unwrap_or(now));
            Some((tracing::Level::WARN, format!("the captured window sent no new frame for {} s; it may be minimized, hidden or not drawing", quiet.as_secs())))
        }
        (Some(at), false) => {
            *since = None;
            Some((tracing::Level::INFO, format!("the captured window sends frames again after {} s", now.saturating_duration_since(at).as_secs())))
        }
        _ => None,
    }
}

fn log_stall(core: &Core, since: &mut Option<Instant>) {
    let e = &core.engine;
    let quiet = e.fresh.lock().unwrap().elapsed();
    let now_stalled = stalled(e.names.lock().unwrap().kind == "window", e.video.lock().unwrap().is_some(), quiet);
    match stall_line(since, now_stalled, quiet, Instant::now()) {
        Some((level, line)) if level == tracing::Level::WARN => tracing::warn!("{line}"),
        Some((_, line)) => tracing::info!("{line}"),
        None => {}
    }
}

pub fn capture_json(core: &Core, live: Option<Value>) -> Value {
    let e = &core.engine;
    let mix = |track: usize| {
        let (volume, muted) = e.gain(track);
        json!({"volume": volume, "muted": muted})
    };
    json!({
        "source": e.source.lock().unwrap().clone(),
        "source_lost": e.source_lost.load(Ordering::Relaxed),
        "sharing": core.sharing_live(),
        "working": e.working(),
        "stalled": stalled(e.names.lock().unwrap().kind == "window", e.video.lock().unwrap().is_some(), e.fresh.lock().unwrap().elapsed()),
        "status": e.status.lock().unwrap().clone(),
        "mic": e.mic.lock().unwrap().clone(),
        "mix": {"app": mix(APP), "mic": mix(MIC)},
        "quality": quality_json(core),
        "live": live,
    })
}

fn quality_json(core: &Core) -> Value {
    let e = &core.engine;
    let id = e.quality.lock().unwrap().clone();
    let sink = e.sink.lock().unwrap().clone().filter(|s| !s.voice);
    let (size, kbps, codec) = match sink {
        Some(s) => (Some(s.out), s.kbps, s.codec),
        None => {
            let chosen = ffmpeg::pick(&usable_codecs(core));
            let width = quality_width(&e.quality_now());
            (e.canvas.lock().unwrap().map(|c| scaled(c, width)), bitrate_kbps(width, chosen.2), chosen.1)
        }
    };
    json!({"id": id, "width": size.map(|s| s.0), "height": size.map(|s| s.1), "fps": FPS, "kbps": kbps, "codec": codec})
}

pub fn wanted(source: bool, lost: bool, recording: Option<&str>, room: Option<String>, full: bool) -> Option<(String, bool)> {
    if full {
        return None;
    }
    let picture = source && !lost;
    match recording {
        Some(room) => Some((room.to_string(), !picture)),
        None => room.filter(|_| picture).map(|room| (room, false)),
    }
}

fn want(core: &Core) -> Option<(String, bool)> {
    let e = &core.engine;
    let recording = core.active_set.lock().unwrap().as_ref().map(|s| s.room.clone());
    wanted(!e.source.lock().unwrap().is_null(), e.source_lost.load(Ordering::Relaxed), recording.as_deref(), core.room_id(), core.disk_full())
}

pub(crate) fn wants_picture(core: &Core) -> bool {
    want(core).is_none_or(|(_, voice)| !voice)
}

pub fn usable_codecs(core: &Core) -> Vec<String> {
    let mut usable = core.codecs.lock().unwrap().clone();
    for p in core.peers.lock().unwrap().iter().filter(|p| p.conn.is_some()) {
        usable.retain(|c| p.codecs.contains(c));
    }
    usable
}

fn encode_command(size: (u32, u32), out: (u32, u32), kbps: u32, chosen: (&str, &str, f64), inputs: [u16; 2], outputs: [u16; 2]) -> Command {
    let (rate, channels, fps) = (SAMPLE_RATE.to_string(), CHANNELS.to_string(), FPS.to_string());
    let mut command = ffmpeg::command("warning");
    if crate::test_mode() {
        command.args(["-re", "-f", "lavfi", "-i", &format!("testsrc2=size={}x{}:rate={fps}", size.0, size.1)]);
        for hz in TEST_TONES_HZ {
            command.args(["-re", "-f", "lavfi", "-i", &format!("sine=frequency={hz}:sample_rate={rate}")]);
        }
    } else {
        command
            .args(["-use_wallclock_as_timestamps", "1", "-thread_queue_size", QUEUE_SIZE])
            .args(["-f", "rawvideo", "-pix_fmt", "nv12", "-video_size", &format!("{}x{}", out.0, out.1), "-i", "-"]);
        for port in inputs {
            command.args(["-thread_queue_size", QUEUE_SIZE, "-f", "s16le", "-ar", &rate, "-ac", &channels, "-i", &format!("tcp://127.0.0.1:{port}")]);
        }
    }
    command.args(["-map", "0:v", "-map", "1:a"]);
    if crate::test_mode() {
        command.args(["-vf", &format!("scale={}:{}:flags=fast_bilinear,format=yuv420p", out.0, out.1)]);
    }
    command
        .args(["-r", &fps, "-colorspace", "bt709", "-color_primaries", "bt709", "-color_trc", "bt709", "-color_range", "tv"])
        .args(["-c:v", chosen.0, "-bf", "0", "-g", &fps, "-b:v", &format!("{kbps}k")]);
    if chosen.0 == ffmpeg::FALLBACK.0 {
        command.args(["-preset", "ultrafast"]);
    }
    if chosen.1 == "hevc" {
        command.args(["-tag:v", "hvc1"]);
    }
    command
        .args(["-force_key_frames", "expr:gte(t,n_forced*1)"])
        .args(["-c:a", "aac", "-b:a", &format!("{AUDIO_KBPS}k"), "-metadata:s:a:0", "title=Game"])
        .args(["-movflags", &format!("{FRAGMENTED}+skip_trailer"), "-f", "mp4", &format!("tcp://127.0.0.1:{}", outputs[0])])
        .args(["-map", "2:a", "-c:a", "aac", "-b:a", &format!("{AUDIO_KBPS}k"), "-metadata:s:a:0", "title=Microphone"])
        .args(["-movflags", "empty_moov+default_base_moof+skip_trailer", "-frag_duration", VOICE_FRAGMENT_US, "-f", "mp4", &format!("tcp://127.0.0.1:{}", outputs[1])])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn voice_command(input: u16, output: u16) -> Command {
    let mut command = ffmpeg::command("warning");
    if crate::test_mode() {
        command.args(["-re", "-f", "lavfi", "-i", &format!("sine=frequency={}:sample_rate={SAMPLE_RATE}", TEST_TONES_HZ[1])]);
    } else {
        command.args(["-thread_queue_size", QUEUE_SIZE, "-f", "s16le", "-ar", &SAMPLE_RATE.to_string(), "-ac", &CHANNELS.to_string(), "-i", &format!("tcp://127.0.0.1:{input}")]);
    }
    command
        .args(["-map", "0:a", "-c:a", "aac", "-b:a", &format!("{AUDIO_KBPS}k"), "-metadata:s:a:0", "title=Microphone"])
        .args(["-movflags", "empty_moov+default_base_moof+skip_trailer", "-frag_duration", VOICE_FRAGMENT_US, "-f", "mp4", &format!("tcp://127.0.0.1:{output}")])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn ensure_rec(core: &Core, room: &str, set: &str, start_us: i64, meta: &Value) {
    let _guard = core.engine.rec_lock.lock().unwrap();
    if crate::room::joined(&core.own_log(room), set) {
        return;
    }
    let (by, voice, of) = core.active_set.lock().unwrap().as_ref().map(|s| (s.by.clone(), s.voice, s.of.clone())).unwrap_or_default();
    let mut record = json!({"k": "rec", "room": room, "set": set, "start_us": start_us, "by": by, "voice": voice});
    if let Some(of) = of {
        record["of"] = json!(of);
    }
    merge(&mut record, meta);
    core.append_log(room, &record);
}

fn finish_part(path: &Path) -> bool {
    proc::wait_until(RENAME_WAIT, RENAME_GAP, || std::fs::rename(part_of(path), path).is_ok())
}

fn open_part(dir: &Path, n: u32, init: Option<&[u8]>) -> Option<std::fs::File> {
    let path = dir.join(INIT);
    if !path.exists() && !init.is_some_and(|bytes| proc::wait_until(RENAME_WAIT, RENAME_GAP, || write_whole(&path, bytes).is_ok())) {
        return None;
    }
    std::fs::File::create(part_of(&dir.join(seg_name(n)))).ok()
}

fn seg_record(capture: &str, series: &str, n: u32, set: &str, t_us: i64, d_us: i64, size: u64, sha256: &str) -> Value {
    json!({"k": "seg", "capture": capture, "series": series, "n": n, "set": set, "t_us": t_us, "d_us": d_us, "size": size, "sha256": sha256})
}

impl Sink {
    fn start(core: &Arc<Core>, room: String, size: (u32, u32), voice: bool) -> Result<Arc<Self>, Err> {
        if !ffmpeg::present() {
            return Err("ffmpeg is not installed yet".into());
        }
        let capture = (now_us() / 1000).to_string();
        for series in SERIES {
            let dir = core.series_dir(&room, None, &core.me(), &capture, series);
            std::fs::create_dir_all(&dir)?;
            data::keep_capture(&dir);
        }
        let chosen = ffmpeg::pick(&usable_codecs(core));
        let quality = core.engine.quality_now();
        let width = quality_width(&quality);
        let (out, kbps) = (scaled(size, width), bitrate_kbps(width, chosen.2));
        let listeners = [TcpListener::bind("127.0.0.1:0")?, TcpListener::bind("127.0.0.1:0")?];
        let outputs = [TcpListener::bind("127.0.0.1:0")?, TcpListener::bind("127.0.0.1:0")?];
        let port = |l: &TcpListener| l.local_addr().map(|a| a.port());
        let mut command = if voice {
            voice_command(port(&listeners[1])?, port(&outputs[1])?)
        } else {
            encode_command(size, out, kbps, chosen, [port(&listeners[0])?, port(&listeners[1])?], [port(&outputs[0])?, port(&outputs[1])?])
        };
        let mut child = proc::spawn(&mut command, BELOW_NORMAL)?;
        if let Some(stderr) = child.child.stderr.take() {
            thread::spawn(move || {
                use std::io::BufRead;
                let mut repeats = Repeats::new(FFMPEG_REPEAT_EVERY);
                for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
                    if let Some(line) = repeats.line(&line, Instant::now()) {
                        tracing::warn!("ffmpeg: {line}");
                    }
                }
                let held = repeats.take_held();
                if held > 0 {
                    tracing::warn!("ffmpeg: {held} more repeated lines before it ended");
                }
            });
        }
        let names = core.engine.names.lock().unwrap().clone();
        let meta = if voice {
            json!({"voice": true, "mic": names.mic, "version": crate::VERSION})
        } else {
            json!({
                "source_kind": names.kind, "source": names.source, "width": out.0, "height": out.1, "fps": FPS, "kbps": kbps,
                "encoder": chosen.0, "codec": chosen.1, "audio": names.game, "mic": names.mic, "version": crate::VERSION,
            })
        };
        if voice {
            tracing::info!("capture {capture} started: voice only, process {}", child.id());
        } else {
            tracing::info!("capture {capture} started: {} {}x{} at {kbps} kbit/s from a {}, process {}", chosen.0, out.0, out.1, names.kind, child.id());
        }
        let stdin = child.child.stdin.take();
        let sink = Arc::new(Self {
            room,
            capture,
            voice,
            out,
            quality,
            kbps,
            codec: chosen.1,
            encoder: chosen.0,
            meta,
            frame: Mutex::new(None),
            ready: Condvar::new(),
            watch: Mutex::new(Watch::default()),
            child: Mutex::new(child),
            tracks: [Mutex::new(Track::default()), Mutex::new(Track::default())],
            outputs: [Mutex::new(Series::default()), Mutex::new(Series::default())],
            inits: Mutex::new([Value::Null, Value::Null]),
            init: Mutex::new([None, None]),
            readers: Mutex::new(Vec::new()),
            closers: Mutex::new(Vec::new()),
            start: OnceLock::new(),
            cap_logged: Mutex::new(None),
            closed: AtomicBool::new(false),
            dead: AtomicBool::new(false),
        });
        for (track, listener) in listeners.into_iter().enumerate() {
            let sink = sink.clone();
            thread::spawn(move || {
                if let Some(stream) = proc::accept(&listener, || !sink.closed.load(Ordering::Relaxed)) {
                    let _ = stream.set_write_timeout(Some(STALL_AFTER * 2));
                    if let Ok(closer) = stream.try_clone() {
                        sink.closers.lock().unwrap().push(closer);
                    }
                    sink.tracks[track].lock().unwrap().stream = Some(stream);
                }
            });
        }
        for (which, listener) in outputs.into_iter().enumerate() {
            let (sink2, core) = (sink.clone(), core.clone());
            sink.readers.lock().unwrap().push(thread::spawn(move || sink2.read_output(&core, which, listener)));
        }
        if let Some(pipe) = stdin {
            let sink = sink.clone();
            thread::spawn(move || sink.feed(pipe));
        }
        if crate::test_mode() || voice {
            let _ = sink.start.set((Instant::now(), now_us()));
        }
        Ok(sink)
    }

    fn read_output(&self, core: &Core, which: usize, listener: TcpListener) {
        if let Some(mut stream) = proc::accept(&listener, || !self.closed.load(Ordering::Relaxed)) {
            let mut splitter = media::Splitter::default();
            let mut buf = vec![0u8; READ_CHUNK];
            loop {
                let n = match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if which == 0 || self.voice {
                    self.watch.lock().unwrap().answer(Instant::now());
                }
                let Some(outs) = splitter.push(&buf[..n]) else { break };
                for out in outs {
                    match out {
                        media::Out::Init(bytes) => self.take_init(which, &bytes),
                        media::Out::Fragment(bytes) => self.take_fragment(core, which, &bytes),
                    }
                }
            }
        }
        let mut series = self.outputs[which].lock().unwrap();
        self.close(core, which, &mut series, None);
        if !self.closed.load(Ordering::Relaxed) {
            self.dead.store(true, Ordering::Relaxed);
        }
    }

    fn take_init(&self, which: usize, bytes: &[u8]) {
        self.outputs[which].lock().unwrap().scales = media::timescales(bytes);
        self.inits.lock().unwrap()[which] = json!({"size": bytes.len(), "sha256": hex(&Sha256::digest(bytes))});
        self.init.lock().unwrap()[which] = Some(bytes.to_vec());
    }

    fn log_cap(&self, core: &Core) {
        let mut logged = self.cap_logged.lock().unwrap();
        let inits = self.inits.lock().unwrap().clone();
        if logged.as_ref() != Some(&inits) {
            let mut record = json!({"k": "cap", "room": self.room, "capture": self.capture, "start_us": self.start.get().map(|s| s.1), "init": {SERIES[0]: inits[0], SERIES[1]: inits[1]}});
            merge(&mut record, &self.meta);
            core.append_log(&self.room, &record);
            *logged = Some(inits);
        }
    }

    fn take_fragment(&self, core: &Core, which: usize, bytes: &[u8]) {
        let Some(&(_, start_us)) = self.start.get() else { return };
        let times = media::fragment_times(bytes);
        let mut series = self.outputs[which].lock().unwrap();
        let Some(first) = times.first() else { return };
        let t_us = start_us + media::micros(first, &series.scales);
        let wanted = core.active_set.lock().unwrap().as_ref().filter(|a| a.room == self.room && !(a.voice && which == 0)).map(|a| a.id.clone());
        let open = series.open.as_ref();
        if media::cut(open.is_some(), open.map_or(0, |o| o.count), FRAGMENTS_PER_SEGMENT, open.and_then(|o| o.set.as_deref()), wanted.as_deref()) {
            self.close(core, which, &mut series, Some(t_us));
            self.open(core, which, &mut series, wanted, t_us);
        }
        if series.last_us > 0 {
            series.step_us = t_us - series.last_us;
        }
        series.last_us = t_us;
        let Some(open) = series.open.as_mut() else { return };
        if open.stopped {
            return;
        }
        if open.file.write_all(bytes).is_err() {
            open.stopped = true;
            if open.file.set_len(open.size).is_err() {
                open.count = 0;
            }
            self.dead.store(true, Ordering::Relaxed);
            return;
        }
        open.hasher.update(bytes);
        open.size += bytes.len() as u64;
        open.count += 1;
    }

    fn open(&self, core: &Core, which: usize, series: &mut Series, set: Option<String>, t_us: i64) {
        let n = series.n + 1;
        let dir = core.series_dir(&self.room, set.as_deref(), &core.me(), &self.capture, SERIES[which]);
        if std::fs::create_dir_all(&dir).is_err() {
            self.dead.store(true, Ordering::Relaxed);
            return;
        }
        if let Some(set) = &set {
            ensure_rec(core, &self.room, set, t_us, &self.meta);
            self.log_cap(core);
        }
        let init = self.init.lock().unwrap()[which].clone();
        let Some(file) = open_part(&dir, n, init.as_deref()) else {
            self.dead.store(true, Ordering::Relaxed);
            return;
        };
        data::forget(&core.files, &dir);
        series.n = n;
        series.recent.push_back((n, set.clone()));
        if series.recent.len() > data::LIVE_KEEP as usize + 1 {
            series.recent.pop_front();
        }
        series.open = Some(Open { n, set, dir, file, hasher: Sha256::new(), size: 0, t_us, count: 0, stopped: false });
    }

    fn close(&self, core: &Core, which: usize, series: &mut Series, next_us: Option<i64>) {
        let Some(open) = series.open.take() else { return };
        drop(open.file);
        let path = open.dir.join(seg_name(open.n));
        let d_us = next_us.map_or(series.last_us - open.t_us + series.step_us.max(0), |next| next - open.t_us);
        if open.stopped && open.count == 0 {
            let _ = std::fs::remove_file(part_of(&path));
            return;
        }
        if !finish_part(&path) {
            let tail: Vec<String> = path.iter().rev().take(3).collect::<Vec<_>>().into_iter().rev().map(|p| p.to_string_lossy().into_owned()).collect();
            tracing::error!("could not finish segment {} (renaming its .part failed), so it is left out of the recording", tail.join("/"));
            return;
        }
        match &open.set {
            Some(set) => {
                data::forget(&core.files, &open.dir);
                let sha = hex(&open.hasher.finalize());
                core.append_log(&self.room, &seg_record(&self.capture, SERIES[which], open.n, set, open.t_us, d_us, open.size, &sha));
            }
            None => data::trim_ring(&open.dir, open.n),
        }
    }

    pub(crate) fn out(&self) -> (u32, u32) {
        self.out
    }

    pub(crate) fn write_video(&self, frame: Arc<Vec<u8>>) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        if self.frame.lock().unwrap().replace(frame).is_some() {
            debug::frame(debug::DROPPED);
        }
        self.ready.notify_one();
    }

    fn next_frame(&self) -> Option<Arc<Vec<u8>>> {
        let mut next = self.frame.lock().unwrap();
        loop {
            if self.closed.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(frame) = next.take() {
                return Some(frame);
            }
            next = self.ready.wait(next).unwrap();
        }
    }

    fn feed(&self, mut pipe: ChildStdin) {
        while let Some(frame) = self.next_frame() {
            let _ = self.start.set((Instant::now(), now_us()));
            self.watch.lock().unwrap().fed(Instant::now());
            if pipe.write_all(&frame).is_err() {
                self.dead.store(true, Ordering::Relaxed);
                return;
            }
            self.watch.lock().unwrap().wrote();
            debug::frame(debug::ENCODED);
        }
        if crate::test_mode() {
            let _ = pipe.write_all(b"q");
        }
    }

    pub fn write_audio(&self, track: usize, pcm: &[u8]) {
        let Some((start, _)) = self.start.get() else { return };
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

    fn wait_cut(&self, set: &str) {
        let deadline = Instant::now() + CUT_WAIT;
        while Instant::now() < deadline && !self.dead.load(Ordering::Relaxed) {
            if self.outputs.iter().all(|o| o.lock().unwrap().open.as_ref().is_none_or(|o| o.set.as_deref() != Some(set))) {
                return;
            }
            thread::sleep(POLL);
        }
    }

    fn finish(&self) {
        self.closed.store(true, Ordering::Relaxed);
        for stream in self.closers.lock().unwrap().drain(..) {
            let _ = stream.shutdown(Shutdown::Both);
        }
        drop(self.frame.lock().unwrap());
        self.ready.notify_all();
        let deadline = Instant::now() + FINISH_WAIT;
        let mut exited = false;
        {
            let mut child = self.child.lock().unwrap();
            while Instant::now() < deadline && !exited {
                exited = child.exited();
                thread::sleep(POLL);
            }
            if !exited {
                tracing::warn!("ffmpeg did not finish in time, killing it");
            }
            child.kill();
        }
        for reader in self.readers.lock().unwrap().drain(..) {
            let _ = reader.join();
        }
    }
}

pub(crate) fn current_sink(core: &Arc<Core>, canvas: Option<(u32, u32)>) -> Option<Arc<Sink>> {
    let e = &core.engine;
    let want = want(core);
    let usable = usable_codecs(core);
    let recording = core.any_recording();
    let mut slot = e.sink.lock().unwrap();
    let stale = slot.as_ref().map(|sink| {
        let resized = !recording && canvas.is_some_and(|c| scaled(c, quality_width(&sink.quality)) != sink.out);
        let codec = ffmpeg::pick(&usable).0 != sink.encoder && (!recording || !usable.iter().any(|c| c == sink.codec));
        let requality = !recording && e.quality_now() != sink.quality;
        let kind = want.as_ref().map(|(room, voice)| (room.as_str(), *voice)) != Some((sink.room.as_str(), sink.voice));
        let rebased = sink.start.get().is_some_and(|(at, at_us)| clock::stepped(*at_us, now_us(), at.elapsed()));
        let dead = sink.dead.load(Ordering::Relaxed);
        let why = [(dead, "the encoder ended"), (kind, "what is captured changed"), (rebased, "the clock was set"), (!sink.voice && resized, "the picture size changed"), (!sink.voice && codec, "the encoder changed"), (!sink.voice && requality, "the quality changed")];
        (dead, !sink.voice && (resized || codec || requality) || kind || rebased, why.into_iter().find(|(on, _)| *on).map_or("", |(_, why)| why))
    });
    if let Some((dead, changed, why)) = stale.filter(|(dead, changed, _)| *dead || *changed) {
        let old = slot.take().unwrap();
        tracing::info!("capture {} stopped: {why}", old.capture);
        if dead {
            old.child.lock().unwrap().kill();
            if !changed {
                e.encoder_died(&old);
            }
        }
        e.finishing.lock().unwrap().push(thread::spawn(move || old.finish()));
    }
    if slot.is_none() {
        let (room, voice) = want?;
        let size = if voice { (0, 0) } else { canvas? };
        let mut finishing = e.finishing.lock().unwrap();
        finishing.retain(|h| !h.is_finished());
        if !finishing.is_empty() || Instant::now() < *e.retry_at.lock().unwrap() {
            return None;
        }
        drop(finishing);
        match Sink::start(core, room, size, voice) {
            Ok(sink) => *slot = Some(sink),
            Err(err) => e.failed(false, &err.to_string()),
        }
    }
    slot.clone()
}

pub fn retire(core: &Core) {
    let e = &core.engine;
    let want = want(core);
    let old = {
        let mut slot = e.sink.lock().unwrap();
        let stale = slot.as_ref().is_some_and(|s| want.as_ref().map(|(room, voice)| (room.as_str(), *voice)) != Some((s.room.as_str(), s.voice)));
        slot.take().filter(|_| stale)
    };
    if let Some(old) = old {
        tracing::info!("capture {} stopped: it is no longer wanted", old.capture);
        old.finish();
    }
}

pub fn end_set(core: &Core, only: Option<&str>) -> Option<crate::room::ActiveSet> {
    let active = {
        let mut slot = core.active_set.lock().unwrap();
        if only.is_some_and(|id| slot.as_ref().is_none_or(|a| a.id != id)) {
            return None;
        }
        slot.take()?
    };
    let sink = core.engine.sink.lock().unwrap().clone().filter(|s| s.room == active.room);
    if let Some(sink) = sink {
        sink.wait_cut(&active.id);
    }
    Some(active)
}

pub struct Recovered {
    pub room: String,
    pub set: String,
    pub capture: String,
    pub series: &'static str,
    pub n: u32,
    pub path: PathBuf,
}

fn cut_back(path: &Path) -> bool {
    let part = part_of(path);
    let Ok((_, whole)) = media::scan(&part) else { return false };
    if whole == 0 {
        let _ = std::fs::remove_file(&part);
        return false;
    }
    std::fs::OpenOptions::new().write(true).open(&part).and_then(|f| f.set_len(whole)).is_ok() && std::fs::rename(&part, path).is_ok()
}

pub fn recover_files(dir: &Path, me: &str, skip: impl Fn(&Recovered) -> bool) -> Vec<Recovered> {
    let mut out = Vec::new();
    for (room, room_dir) in crate::entries(&dir.join("rooms")) {
        for (set, set_dir) in crate::entries(&room_dir.join(RECORDINGS)) {
            for (capture, _) in crate::entries(&set_dir.join(me)).into_iter().filter(|(c, _)| capture_id(c)) {
                for series in SERIES {
                    let series_dir = room::series_dir(&room_dir, Some(&set), me, &capture, series);
                    for (name, _) in crate::entries(&series_dir) {
                        let Some(n) = room::seg_number(&name) else { continue };
                        let path = series_dir.join(seg_name(n));
                        let (done, open) = (name == seg_name(n), name == format!("{}.part", seg_name(n)) && !path.exists());
                        let found = Recovered { room: room.clone(), set: set.clone(), capture: capture.clone(), series, n, path };
                        if (done || open) && !skip(&found) && (done || cut_back(&found.path)) {
                            out.push(found);
                        }
                    }
                }
            }
        }
    }
    out
}

fn recovered_seg(own: &[Value], r: &Recovered) -> Option<Value> {
    let start_us = own.iter().filter(|x| x["k"] == "cap" && x["capture"] == r.capture.as_str()).find_map(|x| x["start_us"].as_i64())?;
    let (size, sha) = data::sha256(&r.path).ok()?;
    let scales = media::timescales(&std::fs::read(r.path.with_file_name(INIT)).unwrap_or_default());
    let times = media::scan(&r.path).ok()?.0;
    let track = times.first().map(|t| t.track);
    let us: Vec<i64> = times.iter().filter(|t| Some(t.track) == track).map(|t| media::micros(t, &scales)).collect();
    let (Some(first), Some(last)) = (us.first(), us.last()) else { return None };
    let step = if us.len() > 1 { last - us[us.len() - 2] } else { 0 };
    Some(seg_record(&r.capture, r.series, r.n, &r.set, start_us + first, last - first + step, size, &sha))
}

fn logged(own: &[Value]) -> HashSet<String> {
    let key = |r: &Value| match r["k"].as_str()? {
        "rec" => Some(format!("rec {}", r["set"].as_str()?)),
        "cap" => Some(format!("cap {}", r["capture"].as_str()?)),
        "seg" => Some(format!("seg {}/{}/{}", r["capture"].as_str()?, r["series"].as_str()?, r["n"].as_u64()?)),
        _ => None,
    };
    own.iter().filter_map(key).collect()
}

fn skip(logged: &HashSet<String>, r: &Recovered) -> bool {
    !logged.contains(&format!("rec {}", r.set)) || !logged.contains(&format!("cap {}", r.capture)) || logged.contains(&format!("seg {}/{}/{}", r.capture, r.series, r.n))
}

fn rec_ends(own: &[Value], room: &str) -> Vec<Value> {
    let sets: BTreeSet<&str> = own.iter().filter(|r| r["k"] == "rec").filter_map(|r| r["set"].as_str()).collect();
    sets.into_iter()
        .filter(|set| room::joined(own, set))
        .map(|set| {
            let start = own.iter().rev().find(|r| r["k"] == "rec" && r["set"] == set).and_then(|r| r["start_us"].as_i64());
            let end = own.iter().filter(|r| r["k"] == "seg" && r["set"] == set).filter_map(|r| Some(r["t_us"].as_i64()? + r["d_us"].as_i64()?)).max();
            json!({"k": "rec_end", "set": set, "room": room, "t_us": end.or(start).unwrap_or_else(now_us)})
        })
        .collect()
}

pub fn recover(core: &Core) {
    let rooms: Vec<String> = crate::entries(&core.dir.join("rooms")).into_iter().map(|(room, _)| room).collect();
    let known: HashMap<&String, HashSet<String>> = rooms.iter().map(|room| (room, logged(&core.own_log(room)))).collect();
    for r in recover_files(&core.dir, &core.me(), |r| known.get(&r.room).is_none_or(|k| skip(k, r))) {
        if let Some(record) = recovered_seg(&core.own_log(&r.room), &r) {
            core.append_log(&r.room, &record);
        }
        if let Some(dir) = r.path.parent() {
            data::forget(&core.files, dir);
        }
    }
    for room in &rooms {
        for record in rec_ends(&core.own_log(room), room) {
            core.append_log(room, &record);
        }
    }
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
        if let Some(sink) = sink.filter(|s| !s.voice && s.out == last.1) {
            sink.write_video(last.0.clone());
        }
        last.2 = Instant::now() - (REPEAT_AFTER - REPEAT_POLL);
    }
}

#[derive(Default)]
struct Speech {
    on: bool,
    loud_at: Option<Instant>,
}

impl Speech {
    fn step(&mut self, now: Instant, level: f32, active: bool) -> Option<bool> {
        if active && level > SPEAK_LEVEL {
            self.loud_at = Some(now);
        }
        let on = active && self.loud_at.is_some_and(|at| now.duration_since(at) <= SPEAK_HANG);
        (on != self.on).then(|| {
            self.on = on;
            on
        })
    }
}

fn meters(core: Arc<Core>) {
    let mut quiet = false;
    let mut speech = Speech::default();
    loop {
        thread::sleep(Duration::from_millis(1000 / LEVELS_PER_SECOND));
        let [app, mic] = [APP, MIC].map(|t| f32::from_bits(core.engine.peaks[t].swap(0, Ordering::Relaxed)));
        let active = core.any_recording() && !core.engine.gain(MIC).1;
        if let Some(on) = speech.step(Instant::now(), mic, active) {
            net::broadcast(&core, json!({"t": "speaking", "on": on}));
            let _ = core.app.emit("speaking", json!({"from": core.me(), "on": on}));
        }
        let silent = app == 0.0 && mic == 0.0;
        if !(silent && quiet) {
            let _ = core.app.emit("levels", json!({"app": app, "mic": mic}));
        }
        quiet = silent;
    }
}

fn rescan(core: &Core, source: &Value, last: Option<usize>) -> Option<Found> {
    let e = &core.engine;
    let found = sources::find(source, last);
    e.source_lost.store(!source.is_null() && found.is_none(), Ordering::Relaxed);
    let mut names = e.names.lock().unwrap();
    names.kind = found.as_ref().map(|f| f.kind.to_string()).unwrap_or_default();
    names.source = found.as_ref().map(|f| f.name.clone()).unwrap_or_default();
    names.game = found.as_ref().map(|f| f.game.clone()).unwrap_or_default();
    found
}

fn ensure_video(core: &Arc<Core>, wanted: Option<&Found>) {
    let e = &core.engine;
    let mut video = e.video.lock().unwrap();
    if video.as_ref().is_some_and(|v| v.finished() || wanted.is_none_or(|f| f.key != v.key)) {
        if let Some(control) = video.take().unwrap().control {
            let _ = control.stop();
        }
        e.last.lock().unwrap().0 = Arc::new(Vec::new());
        *e.canvas.lock().unwrap() = None;
    }
    let Some(f) = wanted.filter(|_| video.is_none()) else { return };
    if Instant::now() < *e.retry_at.lock().unwrap() {
        return;
    }
    let (refused, started) = match f.item {
        Item::Window(w) => start_capture::<Capture, _>(w, core.clone()),
        Item::Display(m) => start_capture::<Capture, _>(m, core.clone()),
        Item::Test => {
            *e.canvas.lock().unwrap() = Some(TEST_SIZE);
            *video = Some(Video { control: None, key: f.key });
            return;
        }
    };
    if let Some(why) = refused {
        tracing::warn!("capture without border refused ({why}), using the default border");
    }
    match started {
        Ok(control) => {
            *e.canvas.lock().unwrap() = None;
            *e.fresh.lock().unwrap() = Instant::now();
            *video = Some(Video { control: Some(control), key: f.key });
        }
        Err(err) => e.failed(false, &format!("screen capture could not start: {err}")),
    }
}

fn check_sink(core: &Core) {
    let e = &core.engine;
    let Some(sink) = e.sink.lock().unwrap().clone() else { return };
    let (watch, now) = (*sink.watch.lock().unwrap(), Instant::now());
    let mut child = sink.child.lock().unwrap();
    if child.exited() {
        sink.dead.store(true, Ordering::Relaxed);
    } else if watch.stalled(now) {
        tracing::warn!("{} produced nothing for {} s, stopping it", sink.encoder, STALL_AFTER.as_secs());
        sink.dead.store(true, Ordering::Relaxed);
        child.kill();
    }
    drop(child);
    if watch.working(now) {
        e.status.lock().unwrap().clear();
    }
    if watch.stable(now) {
        e.failures.store(0, Ordering::Relaxed);
        *e.strikes.lock().unwrap() = ("", 0);
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
    let mut stalled_since = None;
    loop {
        let source = e.source.lock().unwrap().clone();
        let ended = e.video.lock().unwrap().as_ref().is_some_and(Video::finished);
        if ended || scanned.as_ref().is_none_or(|(s, at)| *s != source || at.elapsed() > RESCAN_EVERY) {
            found = rescan(&core, &source, found.as_ref().map(|f| f.key));
            scanned = Some((source, Instant::now()));
        }
        if !crate::test_mode() {
            audio::ensure(&core, APP, found.as_ref().map_or_else(String::new, |f| f.audio.key()));
            let mic = e.mic.lock().unwrap().clone();
            audio::ensure(&core, MIC, mic);
        }
        ensure_video(&core, found.as_ref().filter(|_| want(&core).is_some_and(|(_, voice)| !voice) && ffmpeg::encoders_ready()));
        check_sink(&core);
        log_stall(&core, &mut stalled_since);
        hw::tick(&core);
        thread::sleep(SUPERVISE_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::tests::{hold, put, temp};
    use crate::media::tests::{fragment, init};

    #[test]
    fn speaking_changes_only_when_the_mic_turns_loud_or_stays_quiet_past_the_hang() {
        let (mut speech, start) = (Speech::default(), Instant::now());
        let at = |ms: u64| start + Duration::from_millis(ms);
        assert_eq!(speech.step(at(0), 0.0, true), None);
        assert_eq!(speech.step(at(10), 0.5, true), Some(true));
        assert_eq!(speech.step(at(40), 0.5, true), None);
        assert_eq!(speech.step(at(200), 0.0, true), None);
        assert_eq!(speech.step(at(330), 0.0, true), None);
        assert_eq!(speech.step(at(350), 0.0, true), Some(false));
        assert_eq!(speech.step(at(500), 0.5, true), Some(true));
        assert_eq!(speech.step(at(510), 0.5, false), Some(false));
        assert_eq!(speech.step(at(520), 0.5, false), None);
    }

    #[test]
    fn a_window_that_stops_sending_frames_is_logged_once_with_its_return() {
        let secs = Duration::from_secs;
        let t = Instant::now();
        let mut since = None;
        let (level, line) = stall_line(&mut since, true, secs(5), t).unwrap();
        assert_eq!(level, tracing::Level::WARN);
        assert!(line.starts_with("the captured window sent no new frame for 5 s"));
        assert!(stall_line(&mut since, true, secs(30), t + secs(25)).is_none());
        let (level, line) = stall_line(&mut since, false, secs(0), t + secs(40)).unwrap();
        assert_eq!(level, tracing::Level::INFO);
        assert_eq!(line, "the captured window sends frames again after 45 s");
        assert!(stall_line(&mut since, false, secs(0), t + secs(41)).is_none());
    }

    #[test]
    fn a_window_that_sends_no_frames_for_a_while_is_stalled() {
        let secs = Duration::from_secs;
        assert!(stalled(true, true, secs(5)));
        assert!(!stalled(true, true, secs(2)));
        assert!(!stalled(false, true, secs(60)));
        assert!(!stalled(true, false, secs(60)));
    }

    #[test]
    fn every_start_has_the_microphone_muted_and_keeps_the_volumes() {
        let mix = starting_mix(&json!({"app": {"volume": 0.4, "muted": true}, "mic": {"volume": 0.7, "muted": false}, "game": {"volume": 0.45}, "mic:{6844}": {"volume": 0.55}}));
        assert_eq!(mix, json!({"app": {"volume": 0.4, "muted": true}, "mic": {"volume": 0.7, "muted": true}}));
        assert_eq!(starting_mix(&Value::Null), json!({"app": {"volume": 1.0, "muted": false}, "mic": {"volume": 1.0, "muted": true}}));
    }

    #[test]
    fn recording_without_a_screen_records_the_voice_only() {
        let room = || Some("r".to_string());
        assert_eq!(wanted(false, false, None, room(), false), None);
        assert_eq!(wanted(true, false, None, room(), false), Some(("r".into(), false)));
        assert_eq!(wanted(true, true, None, room(), false), None);
        assert_eq!(wanted(true, false, None, None, false), None);
        assert_eq!(wanted(false, false, Some("s"), room(), false), Some(("s".into(), true)));
        assert_eq!(wanted(true, true, Some("s"), room(), false), Some(("s".into(), true)));
        assert_eq!(wanted(true, false, Some("s"), room(), true), None);
    }

    #[test]
    fn a_capture_born_before_the_clock_moved_is_finished() {
        let secs = Duration::from_secs;
        assert!(clock::stepped(0, 18_000_000, secs(10)));
        assert!(!clock::stepped(0, 10_100_000, secs(10)));
    }

    #[test]
    fn presets_scale_down_only_and_keep_even_sizes() {
        assert_eq!(scaled((2560, 1440), quality_width("standard")), (1280, 720));
        assert_eq!(scaled((1280, 720), quality_width("high")), (1280, 720));
        assert_eq!(scaled((1921, 1081), 1920), (1920, 1080));
        assert_eq!(scaled((3440, 1440), quality_width("light")), (960, 402));
        assert_eq!(quality_width("nonsense"), quality_width("standard"));
    }

    #[test]
    fn bitrate_follows_pixels_and_codec() {
        assert_eq!(bitrate_kbps(1280, 1.0), 5530);
        assert!(bitrate_kbps(1920, 1.0) > 2 * bitrate_kbps(1280, 1.0));
        assert_eq!(bitrate_kbps(1280, 0.5), 2765);
    }

    fn fed_every(w: &mut Watch, from: Instant, to: Instant) {
        let mut t = from;
        while t <= to {
            w.fed(t);
            w.wrote();
            t += FRAME_GAP;
        }
    }

    #[test]
    fn an_encoder_that_takes_frames_but_gives_nothing_is_stalled() {
        let t = Instant::now();
        let mut w = Watch::default();
        assert!(!w.stalled(t + STALL_AFTER * 9));
        fed_every(&mut w, t, t + STALL_AFTER);
        assert!(!w.working(t + STALL_AFTER));
        assert!(!w.stalled(t + STALL_AFTER));
        fed_every(&mut w, t + STALL_AFTER, t + STALL_AFTER + POLL * 2);
        assert!(w.stalled(t + STALL_AFTER + POLL * 2));
        w.answer(t + STALL_AFTER * 2);
        fed_every(&mut w, t + STALL_AFTER * 2, t + STALL_AFTER * 3);
        assert!(w.working(t + STALL_AFTER * 3));
    }

    #[test]
    fn a_write_that_never_returns_is_a_stall() {
        let t = Instant::now();
        let mut w = Watch::default();
        w.fed(t);
        assert!(!w.stalled(t + STALL_AFTER));
        assert!(w.stalled(t + STALL_AFTER + POLL));
    }

    #[test]
    fn no_frames_to_encode_is_not_a_stall() {
        let t = Instant::now();
        let mut w = Watch::default();
        w.fed(t);
        w.wrote();
        assert!(!w.stalled(t + STALL_AFTER * 3));
        w.fed(t + STALL_AFTER * 3);
        w.wrote();
        assert!(!w.stalled(t + STALL_AFTER * 3 + FEED_GAP));
    }

    #[test]
    fn output_is_stable_only_after_a_long_working_run() {
        let t = Instant::now();
        let mut w = Watch::default();
        w.fed(t);
        w.wrote();
        w.answer(t);
        assert!(!w.stable(t + STABLE_AFTER));
        assert!(w.stable(t + STABLE_AFTER + POLL));
        w.fed(t + STABLE_AFTER);
        assert!(!w.stable(t + STABLE_AFTER + STALL_AFTER + POLL));
    }

    #[test]
    fn a_restart_logs_each_unlogged_own_segment_once() {
        let (dir, me) = (temp("recover"), iroh::SecretKey::generate().public().to_string());
        let (one, two, three) = (fragment(&[(1, 0)], b"aaaa"), fragment(&[(1, 1000)], b"bbbb"), fragment(&[(1, 2000)], b"cccc"));
        let own = dir.join(format!("rooms/r/recordings/s1/{me}/5000"));
        for (name, body) in [("v/init.mp4", init(&[(1, 1000)])), ("v/000001.m4s", one.clone()), ("v/000002.m4s.part", [&one[..], &two, b"torn"].concat()), ("v/000003.m4s", three.clone()), ("m/000002.m4s.part", b"torn".to_vec())] {
            put(&own, name, &body);
        }
        let mut own_log = vec![json!({"k": "rec", "set": "s1", "start_us": 5_000_000}), json!({"k": "cap", "capture": "5000", "start_us": 5_000_000}), seg_record("5000", "v", 1, "s1", 0, 0, 1, "x")];
        let held = hold(&own.join("v/000002.m4s.part"));
        assert_eq!(recover_files(&dir, &me, |r| skip(&logged(&own_log), r)).iter().map(|r| r.n).collect::<Vec<_>>(), [3]);
        drop(held);
        let mut found = recover_files(&dir, &me, |r| skip(&logged(&own_log), r));
        found.sort_by_key(|r| r.n);
        let records: Vec<Value> = found.iter().map(|r| recovered_seg(&own_log, r).unwrap()).collect();
        let (cut, sha) = ([one, two].concat(), |bytes: &[u8]| hex(&Sha256::digest(bytes)));
        assert_eq!(records, [seg_record("5000", "v", 2, "s1", 5_000_000, 2_000_000, cut.len() as u64, &sha(&cut)), seg_record("5000", "v", 3, "s1", 7_000_000, 0, three.len() as u64, &sha(&three))]);
        assert_eq!((std::fs::read(own.join("v/000002.m4s")).unwrap(), own.join("m/000002.m4s.part").exists()), (cut, false));
        assert!(recovered_seg(&own_log[..1], &found[0]).is_none());
        own_log.extend(records);
        assert!(recover_files(&dir, &me, |r| skip(&logged(&own_log), r)).is_empty());
        let ends = rec_ends(&own_log, "r");
        assert_eq!(ends, [json!({"k": "rec_end", "set": "s1", "room": "r", "t_us": 7_000_000})]);
        own_log.extend(ends);
        assert!(rec_ends(&own_log, "r").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_segment_opens_only_beside_its_init() {
        let series = temp("open");
        assert!(open_part(&series, 1, None).is_none());
        put(&series, "init.mp4.part", b"");
        let held = hold(&part_of(&series.join(INIT)));
        assert!(open_part(&series, 1, Some(b"init")).is_none() && !series.join(INIT).exists());
        drop(held);
        assert!(open_part(&series, 1, Some(b"init")).is_some() && open_part(&series, 2, Some(b"other")).is_some());
        assert_eq!((std::fs::read(series.join(INIT)).unwrap(), part_of(&series.join(seg_name(2))).exists()), (b"init".to_vec(), true));
        let _ = std::fs::remove_dir_all(series);
    }

    #[test]
    fn a_live_stream_serves_only_segments_of_the_recording_it_asked_for() {
        use LiveStep::*;
        let set = |s: &str| Some(s.to_string());
        let recent = vec![(5, None), (6, None), (7, set("s1")), (8, set("s1"))];
        assert_eq!(live_step(&[], None, 1), Wait);
        for (wanted, n, step) in [(None, 9, Wait), (None, 6, Behind), (None, 7, Stop), (Some("s1"), 7, Serve), (Some("s1"), 6, Behind), (Some("s2"), 8, Stop), (None, 4, Behind)] {
            assert_eq!(live_step(&recent, wanted, n), step);
        }
        let ended = vec![(7, set("s1")), (8, None), (9, None)];
        assert_eq!((live_step(&ended, Some("s1"), 8), live_step(&ended, Some("s1"), 10)), (Stop, Wait));
    }

    #[test]
    fn quality_steps_down_only_while_every_direct_receiver_fell_behind_twice_recently() {
        let now = Instant::now() + THIN_WINDOW * 2;
        let (fresh, old) = (Some(now - Duration::from_secs(5)), Some(now - THIN_WINDOW));
        assert!(!thin(&[], now));
        assert!(thin(&[[fresh, fresh]], now));
        assert!(!thin(&[[None, fresh]], now));
        assert!(!thin(&[[old, fresh]], now));
        assert!(thin(&[[fresh, fresh], [fresh, fresh]], now));
        assert!(!thin(&[[fresh, fresh], [None, None]], now));
    }

    #[test]
    fn strikes_count_one_encoder_in_a_row() {
        let a = strike(("", 0), "av1_nvenc");
        let b = strike(a, "av1_nvenc");
        assert_eq!(strike(b, "av1_nvenc"), ("av1_nvenc", DROP_AFTER));
        assert_eq!(strike(b, "hevc_nvenc"), ("hevc_nvenc", 1));
    }
}

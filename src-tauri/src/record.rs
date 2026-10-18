use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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
use crate::room::{part_of, seg_name, INIT, SERIES};
use crate::sources::{self, Found, Item};
use crate::{data, debug, hex, media, merge, now_us, read_json, write_whole, Core, BELOW_NORMAL, NO_WINDOW};

pub type Err = Box<dyn std::error::Error + Send + Sync>;

pub const FPS: u32 = 30;
pub const FRAGMENTS_PER_SEGMENT: u32 = 4;
const H264_BITS_PER_PIXEL: f64 = 0.2;
const AUDIO_BITRATE: &str = "128k";
const QUEUE_SIZE: &str = "512";
const VOICE_FRAGMENT_US: &str = "1000000";
const FRAME_GAP: Duration = Duration::from_millis(30);
const REPEAT_AFTER: Duration = Duration::from_millis(250);
const REPEAT_POLL: Duration = Duration::from_millis(100);
const PCM_FRAME_BYTES: u64 = 4;
const PAD_AFTER_FRAMES: u64 = SAMPLE_RATE as u64 / 50;
pub const QUALITIES: [(&str, u32); 3] = [("light", 960), ("standard", 1280), ("high", 1920)];
const WIDE: (u32, u32) = (16, 9);
const LEVELS_PER_SECOND: u64 = 30;
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
const PROBE_WAIT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(20);
pub const APP: usize = 0;
pub const MIC: usize = 1;
const TEST_SIZE: (u32, u32) = (1280, 720);
const TEST_TONES_HZ: [u32; 2] = [440, 880];

pub fn quality_width(id: &str) -> u32 {
    QUALITIES.iter().find(|q| q.0 == id).unwrap_or(&QUALITIES[1]).1
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
pub const OPEN: &str = "open.json";
pub const CAP: &str = "cap.json";

#[derive(Default)]
struct Track {
    stream: Option<TcpStream>,
    written: u64,
}

struct Open {
    n: u32,
    set: Option<String>,
    file: std::fs::File,
    hasher: Sha256,
    size: u64,
    t_us: i64,
    count: u32,
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
        }
        self.owed.get_or_insert(now);
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
    last_us: i64,
    step_us: i64,
}

pub struct Sink {
    pub room: String,
    pub capture: String,
    dir: PathBuf,
    size: (u32, u32),
    out: (u32, u32),
    quality: String,
    kbps: u32,
    encoder: &'static str,
    codec: &'static str,
    meta: Value,
    child: Mutex<Child>,
    frame: Mutex<Option<Arc<Vec<u8>>>>,
    ready: Condvar,
    watch: Mutex<Watch>,
    tracks: [Mutex<Track>; 2],
    outputs: [Mutex<Series>; 2],
    inits: Mutex<[Value; 2]>,
    readers: Mutex<Vec<JoinHandle<()>>>,
    closers: Mutex<Vec<TcpStream>>,
    start: OnceLock<(Instant, i64)>,
    cap_logged: AtomicBool,
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
    pub quality: Mutex<String>,
    pub source_lost: AtomicBool,
    pub status: Mutex<String>,
    pub names: Mutex<Names>,
    pub sink: Mutex<Option<Arc<Sink>>>,
    pub audio: [Mutex<Option<Audio>>; 2],
    pub peaks: [AtomicU32; 2],
    video: Mutex<Option<Video>>,
    canvas: Mutex<Option<(u32, u32)>>,
    last: Mutex<(Arc<Vec<u8>>, (u32, u32), Instant)>,
    failures: AtomicU32,
    strikes: Mutex<(&'static str, u32)>,
    retry_at: Mutex<Instant>,
    rec_lock: Mutex<()>,
}

impl Engine {
    pub fn new(settings: &Value) -> Self {
        Self {
            source: Mutex::new(settings["source"].clone()),
            mic: Mutex::new(settings["mic"].as_str().unwrap_or_default().to_string()),
            mix: Mutex::new(if settings["mix"].is_object() { settings["mix"].clone() } else { json!({}) }),
            quality: Mutex::new(settings["quality"].as_str().unwrap_or(QUALITIES[1].0).to_string()),
            source_lost: AtomicBool::new(false),
            status: Mutex::new(String::new()),
            names: Mutex::new(Names::default()),
            sink: Mutex::new(None),
            audio: [Mutex::new(None), Mutex::new(None)],
            peaks: [AtomicU32::new(0), AtomicU32::new(0)],
            video: Mutex::new(None),
            canvas: Mutex::new(None),
            last: Mutex::new((Arc::new(Vec::new()), (0, 0), Instant::now())),
            failures: AtomicU32::new(0),
            strikes: Mutex::new(("", 0)),
            retry_at: Mutex::new(Instant::now()),
            rec_lock: Mutex::new(()),
        }
    }

    pub fn mix_key(&self, track: usize) -> String {
        if track == APP { "app".into() } else { format!("mic:{}", self.names.lock().unwrap().mic_id) }
    }

    pub fn gain(&self, track: usize) -> (f32, bool) {
        let key = self.mix_key(track);
        let mix = self.mix.lock().unwrap();
        (mix[&key]["volume"].as_f64().unwrap_or(1.0) as f32, mix[&key]["muted"].as_bool().unwrap_or(false))
    }

    pub fn live(&self) -> Option<(String, String, [u32; 2])> {
        let sink = self.sink.lock().unwrap().clone()?;
        let n = [0, 1].map(|i| sink.outputs[i].lock().unwrap().n);
        Some((sink.room.clone(), sink.capture.clone(), n))
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
        "working": e.working(),
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
    let sink = e.sink.lock().unwrap().clone().filter(|s| s.quality == id);
    let (size, kbps, codec) = match sink {
        Some(s) => (Some(s.out), s.kbps, s.codec),
        None => {
            let chosen = ffmpeg::pick(&usable_codecs(core));
            let canvas = *e.canvas.lock().unwrap();
            (canvas.map(|c| scaled(c, quality_width(&id))), bitrate_kbps(quality_width(&id), chosen.2), chosen.1)
        }
    };
    json!({"id": id, "width": size.map(|s| s.0), "height": size.map(|s| s.1), "fps": FPS, "kbps": kbps, "codec": codec})
}

fn want(core: &Core) -> Option<String> {
    let e = &core.engine;
    if (e.source.lock().unwrap().is_null() && !crate::test_mode()) || core.disk_full() {
        return None;
    }
    if let Some(s) = core.active_set.lock().unwrap().as_ref() {
        return Some(s.room.clone());
    }
    let room = core.room_id()?;
    (core.share.load(Ordering::Relaxed) && !e.source_lost.load(Ordering::Relaxed)).then_some(room)
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
    let mut command = Command::new(ffmpeg::path());
    command.args(["-hide_banner", "-loglevel", "warning", "-y"]);
    if crate::test_mode() {
        command.args(["-re", "-f", "lavfi", "-i", &format!("testsrc2=size={}x{}:rate={fps}", size.0, size.1)]);
        for hz in TEST_TONES_HZ {
            command.args(["-re", "-f", "lavfi", "-i", &format!("sine=frequency={hz}:sample_rate={rate}")]);
        }
    } else {
        command
            .args(["-use_wallclock_as_timestamps", "1", "-thread_queue_size", QUEUE_SIZE])
            .args(["-f", "rawvideo", "-pix_fmt", "bgra", "-video_size", &format!("{}x{}", size.0, size.1), "-i", "-"]);
        for port in inputs {
            command.args(["-thread_queue_size", QUEUE_SIZE, "-f", "s16le", "-ar", &rate, "-ac", &channels, "-i", &format!("tcp://127.0.0.1:{port}")]);
        }
    }
    command
        .args(["-map", "0:v", "-map", "1:a"])
        .args(["-vf", &format!("scale={}:{}:flags=fast_bilinear,format=yuv420p", out.0, out.1), "-r", &fps])
        .args(["-c:v", chosen.0, "-bf", "0", "-g", &fps, "-b:v", &format!("{kbps}k")]);
    if chosen.0 == ffmpeg::FALLBACK.0 {
        command.args(["-preset", "ultrafast"]);
    }
    if chosen.1 == "hevc" {
        command.args(["-tag:v", "hvc1"]);
    }
    command
        .args(["-force_key_frames", "expr:gte(t,n_forced*1)"])
        .args(["-c:a", "aac", "-b:a", AUDIO_BITRATE, "-metadata:s:a:0", "title=Game"])
        .args(["-movflags", &format!("{FRAGMENTED}+skip_trailer"), "-f", "mp4", &format!("tcp://127.0.0.1:{}", outputs[0])])
        .args(["-map", "2:a", "-c:a", "aac", "-b:a", AUDIO_BITRATE, "-metadata:s:a:0", "title=Microphone"])
        .args(["-movflags", "empty_moov+default_base_moof+skip_trailer", "-frag_duration", VOICE_FRAGMENT_US, "-f", "mp4", &format!("tcp://127.0.0.1:{}", outputs[1])])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(NO_WINDOW | BELOW_NORMAL);
    command
}

fn accept(listener: &TcpListener, closed: &AtomicBool) -> Option<TcpStream> {
    listener.set_nonblocking(true).ok()?;
    while !closed.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).ok()?;
                return Some(stream);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => thread::sleep(POLL),
            Err(_) => return None,
        }
    }
    None
}

fn ensure_rec(core: &Core, room: &str, set: &str, start_us: i64, meta: &Value) {
    let _guard = core.engine.rec_lock.lock().unwrap();
    if crate::room::joined(&core.own_log(room), set) {
        return;
    }
    let by = core.active_set.lock().unwrap().as_ref().map(|s| s.by.clone()).unwrap_or_default();
    let mut record = json!({"k": "rec", "room": room, "set": set, "start_us": start_us, "by": by});
    merge(&mut record, meta);
    core.append_log(room, &record);
}

fn log_cap(core: &Core, room: &str, capture: &str, cap: &Value) {
    if core.own_log(room).iter().any(|r| r["k"] == "cap" && r["capture"] == capture) {
        return;
    }
    let mut record = json!({"k": "cap", "room": room, "capture": capture});
    merge(&mut record, cap);
    core.append_log(room, &record);
}

fn seg_record(capture: &str, series: &str, n: u32, set: &str, t_us: i64, d_us: i64, size: u64, sha256: &str) -> Value {
    json!({"k": "seg", "capture": capture, "series": series, "n": n, "set": set, "t_us": t_us, "d_us": d_us, "size": size, "sha256": sha256})
}

impl Sink {
    fn start(core: &Arc<Core>, room: String, size: (u32, u32)) -> Result<Arc<Self>, Err> {
        if !ffmpeg::present() {
            return Err("ffmpeg is not installed yet".into());
        }
        let capture = (now_us() / 1000).to_string();
        let dir = core.capture_dir(&room, &core.me(), &capture);
        for series in SERIES {
            std::fs::create_dir_all(dir.join(series))?;
        }
        let chosen = ffmpeg::pick(&usable_codecs(core));
        let quality = core.engine.quality.lock().unwrap().clone();
        let width = quality_width(&quality);
        let (out, kbps) = (scaled(size, width), bitrate_kbps(width, chosen.2));
        let listeners = [TcpListener::bind("127.0.0.1:0")?, TcpListener::bind("127.0.0.1:0")?];
        let outputs = [TcpListener::bind("127.0.0.1:0")?, TcpListener::bind("127.0.0.1:0")?];
        let port = |l: &TcpListener| l.local_addr().map(|a| a.port());
        let mut child = encode_command(size, out, kbps, chosen, [port(&listeners[0])?, port(&listeners[1])?], [port(&outputs[0])?, port(&outputs[1])?]).spawn()?;
        if let Some(stderr) = child.stderr.take() {
            thread::spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(stderr).lines().map_while(Result::ok) {
                    tracing::warn!("ffmpeg: {line}");
                    debug::ffmpeg_line(&line);
                }
            });
        }
        let names = core.engine.names.lock().unwrap().clone();
        let meta = json!({
            "source_kind": names.kind, "source": names.source, "width": out.0, "height": out.1, "fps": FPS, "kbps": kbps,
            "encoder": chosen.0, "codec": chosen.1, "audio": names.game, "mic": names.mic, "version": crate::VERSION,
        });
        let stdin = child.stdin.take();
        let sink = Arc::new(Self {
            room,
            capture,
            dir,
            size,
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
            readers: Mutex::new(Vec::new()),
            closers: Mutex::new(Vec::new()),
            start: OnceLock::new(),
            cap_logged: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            dead: AtomicBool::new(false),
            born: Instant::now(),
        });
        for (track, listener) in listeners.into_iter().enumerate() {
            let sink = sink.clone();
            thread::spawn(move || {
                if let Some(stream) = accept(&listener, &sink.closed) {
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
        if crate::test_mode() {
            let _ = sink.start.set((Instant::now(), now_us()));
        }
        Ok(sink)
    }

    fn read_output(&self, core: &Core, which: usize, listener: TcpListener) {
        if let Some(mut stream) = accept(&listener, &self.closed) {
            let mut splitter = media::Splitter::default();
            let mut buf = vec![0u8; READ_CHUNK];
            loop {
                let n = match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                if which == 0 {
                    self.watch.lock().unwrap().answer(Instant::now());
                }
                let Some(outs) = splitter.push(&buf[..n]) else { break };
                for out in outs {
                    match out {
                        media::Out::Init(bytes) => self.take_init(core, which, &bytes),
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

    fn take_init(&self, core: &Core, which: usize, bytes: &[u8]) {
        let path = self.dir.join(SERIES[which]).join(INIT);
        if write_whole(&path, bytes).is_err() {
            self.dead.store(true, Ordering::Relaxed);
            return;
        }
        self.outputs[which].lock().unwrap().scales = media::timescales(bytes);
        data::mark(core, &self.dir, &format!("{}/{INIT}", SERIES[which]), bytes.len() as u64, true);
        let mut inits = self.inits.lock().unwrap();
        inits[which] = json!({"size": bytes.len(), "sha256": hex(&Sha256::digest(bytes))});
        let mut cap = json!({"start_us": self.start.get().map(|s| s.1), "init": {SERIES[0]: inits[0], SERIES[1]: inits[1]}});
        merge(&mut cap, &self.meta);
        let _ = write_whole(&self.dir.join(CAP), cap.to_string().as_bytes());
    }

    fn take_fragment(&self, core: &Core, which: usize, bytes: &[u8]) {
        let Some(&(_, start_us)) = self.start.get() else { return };
        let times = media::fragment_times(bytes, 0);
        let mut series = self.outputs[which].lock().unwrap();
        let Some(first) = times.first() else { return };
        let t_us = start_us + media::micros(first, &series.scales);
        let wanted = core.active_set.lock().unwrap().as_ref().filter(|a| a.room == self.room).map(|a| a.id.clone());
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
        if open.file.write_all(bytes).is_err() {
            self.dead.store(true, Ordering::Relaxed);
            return;
        }
        open.hasher.update(bytes);
        open.size += bytes.len() as u64;
        open.count += 1;
    }

    fn open(&self, core: &Core, which: usize, series: &mut Series, set: Option<String>, t_us: i64) {
        let n = series.n + 1;
        let dir = self.dir.join(SERIES[which]);
        let Ok(file) = std::fs::File::create(part_of(&dir.join(seg_name(n)))) else {
            self.dead.store(true, Ordering::Relaxed);
            return;
        };
        series.n = n;
        let _ = write_whole(&dir.join(OPEN), json!({"n": n, "set": set}).to_string().as_bytes());
        if let Some(set) = &set {
            ensure_rec(core, &self.room, set, t_us, &self.meta);
        }
        series.open = Some(Open { n, set, file, hasher: Sha256::new(), size: 0, t_us, count: 0 });
    }

    fn close(&self, core: &Core, which: usize, series: &mut Series, next_us: Option<i64>) {
        let Some(open) = series.open.take() else { return };
        drop(open.file);
        let dir = self.dir.join(SERIES[which]);
        let path = dir.join(seg_name(open.n));
        let d_us = next_us.map_or(series.last_us - open.t_us + series.step_us.max(0), |next| next - open.t_us);
        debug::cut(&self.capture, SERIES[which], open.n, open.set.as_deref(), open.size, open.count, d_us);
        if let Some(set) = &open.set {
            let sha = hex(&open.hasher.finalize());
            if !self.cap_logged.swap(true, Ordering::Relaxed) {
                log_cap(core, &self.room, &self.capture, &read_json(&self.dir.join(CAP)));
            }
            data::mark(core, &self.dir, &format!("{}/{}", SERIES[which], seg_name(open.n)), open.size, true);
            core.append_log(&self.room, &seg_record(&self.capture, SERIES[which], open.n, set, open.t_us, d_us, open.size, &sha));
        }
        let _ = std::fs::rename(part_of(&path), &path);
        let _ = std::fs::remove_file(dir.join(OPEN));
    }

    fn write_video(&self, frame: Arc<Vec<u8>>) {
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        *self.frame.lock().unwrap() = Some(frame);
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
                exited = matches!(child.try_wait(), Ok(Some(_)));
                thread::sleep(POLL);
            }
            if !exited {
                tracing::warn!("ffmpeg did not finish in time, killing it");
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        for reader in self.readers.lock().unwrap().drain(..) {
            let _ = reader.join();
        }
    }
}

impl Sink {
    fn debug_json(&self) -> Value {
        let series: Vec<Value> = (0..SERIES.len())
            .map(|i| {
                let s = self.outputs[i].lock().unwrap();
                let open = s.open.as_ref().map(|o| json!({"n": o.n, "set": o.set, "size": o.size, "fragments": o.count, "t_us": o.t_us}));
                json!({"series": SERIES[i], "n": s.n, "open": open, "last_us": s.last_us, "step_us": s.step_us, "pcm_frames": self.tracks[i].try_lock().ok().map(|t| t.written)})
            })
            .collect();
        json!({
            "room": self.room, "capture": self.capture, "size": self.size, "codec": self.codec, "meta": self.meta, "age_ms": self.born.elapsed().as_millis() as u64,
            "encoder": self.encoder, "working": self.watch.lock().unwrap().working(Instant::now()),
            "dead": self.dead.load(Ordering::Relaxed), "closed": self.closed.load(Ordering::Relaxed), "pid": self.child.lock().unwrap().id(), "series": series, "inits": *self.inits.lock().unwrap(),
        })
    }
}

pub fn debug_json(core: &Core) -> Value {
    let e = &core.engine;
    let names = e.names.lock().unwrap().clone();
    let sink = e.sink.lock().unwrap().clone();
    let (frame_size, frame_ms) = {
        let last = e.last.lock().unwrap();
        (last.1, (!last.0.is_empty()).then(|| last.2.elapsed().as_millis() as u64))
    };
    json!({
        "source": e.source.lock().unwrap().clone(),
        "names": {"kind": names.kind, "source": names.source, "game": names.game, "mic": names.mic},
        "source_lost": e.source_lost.load(Ordering::Relaxed),
        "status": e.status.lock().unwrap().clone(),
        "failures": e.failures.load(Ordering::Relaxed),
        "strikes": *e.strikes.lock().unwrap(),
        "dropped": ffmpeg::dropped(),
        "retry_in_ms": e.retry_at.lock().unwrap().saturating_duration_since(Instant::now()).as_millis() as u64,
        "capturing": e.video.lock().unwrap().is_some(),
        "canvas": *e.canvas.lock().unwrap(),
        "frame_size": frame_size,
        "frame_age_ms": frame_ms,
        "audio": ([APP, MIC].map(|t| e.audio[t].lock().unwrap().is_some())),
        "mix": e.mix.lock().unwrap().clone(),
        "want": want(core),
        "encoders_ready": ffmpeg::encoders_ready(),
        "sink": sink.map(|s| s.debug_json()),
    })
}

fn current_sink(core: &Arc<Core>, canvas: Option<(u32, u32)>) -> Option<Arc<Sink>> {
    let e = &core.engine;
    let want = want(core);
    let usable = usable_codecs(core);
    let recording = core.active_set.lock().unwrap().is_some();
    let mut slot = e.sink.lock().unwrap();
    let stale = slot.as_ref().map(|sink| {
        let resized = !recording && canvas.is_some_and(|c| c != sink.size);
        let codec = ffmpeg::pick(&usable).0 != sink.encoder && (!recording || !usable.iter().any(|c| c == sink.codec));
        let requality = !recording && *e.quality.lock().unwrap() != sink.quality;
        (sink.dead.load(Ordering::Relaxed), resized || codec || requality || want.as_deref() != Some(sink.room.as_str()))
    });
    if let Some((dead, changed)) = stale.filter(|(dead, changed)| *dead || *changed) {
        let old = slot.take().unwrap();
        if dead && !changed {
            e.encoder_died(&old);
        }
        thread::spawn(move || old.finish());
    }
    if slot.is_none() {
        let (room, size) = (want?, canvas?);
        if Instant::now() < *e.retry_at.lock().unwrap() {
            return None;
        }
        match Sink::start(core, room, size) {
            Ok(sink) => *slot = Some(sink),
            Err(err) => e.failed(false, &err.to_string()),
        }
    }
    slot.clone()
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

fn recover_series(core: &Core, room: &str, capture: &str, dir: &Path, which: usize) {
    let series_dir = dir.join(SERIES[which]);
    let open: Value = read_json(&series_dir.join(OPEN));
    let Some(n) = open["n"].as_u64().map(|n| n as u32) else { return };
    let path = series_dir.join(seg_name(n));
    let part = part_of(&path);
    if part.exists() {
        let whole = media::scan(&part).1;
        let kept = whole > 0 && std::fs::OpenOptions::new().write(true).open(&part).and_then(|f| f.set_len(whole)).is_ok() && std::fs::rename(&part, &path).is_ok();
        if !kept {
            let _ = std::fs::remove_file(&part);
        }
    }
    if let Some(set) = open["set"].as_str().filter(|_| path.exists()) {
        log_recovered(core, room, capture, dir, which, n, set);
    }
    let _ = std::fs::remove_file(series_dir.join(OPEN));
}

fn log_recovered(core: &Core, room: &str, capture: &str, dir: &Path, which: usize, n: u32, set: &str) {
    let logged = core.own_log(room).iter().any(|r| r["k"] == "seg" && r["capture"] == capture && r["series"] == SERIES[which] && r["n"] == n);
    if logged {
        return;
    }
    let series_dir = dir.join(SERIES[which]);
    let path = series_dir.join(seg_name(n));
    let cap: Value = read_json(&dir.join(CAP));
    let (Some(start_us), Some((size, sha))) = (cap["start_us"].as_i64(), data::sha256(&path)) else { return };
    let scales = media::timescales(&std::fs::read(series_dir.join(INIT)).unwrap_or_default());
    let times = media::scan(&path).0;
    let track = times.first().map(|t| t.track);
    let us: Vec<i64> = times.iter().filter(|t| Some(t.track) == track).map(|t| media::micros(t, &scales)).collect();
    let (Some(first), Some(last)) = (us.first(), us.last()) else { return };
    let step = if us.len() > 1 { last - us[us.len() - 2] } else { 0 };
    log_cap(core, room, capture, &cap);
    data::mark(core, dir, &format!("{}/{}", SERIES[which], seg_name(n)), size, true);
    core.append_log(room, &seg_record(capture, SERIES[which], n, set, start_us + first, last - first + step, size, &sha));
}

pub fn recover(core: &Core) {
    let me = core.me();
    for room in std::fs::read_dir(core.dir.join("rooms")).into_iter().flatten().flatten() {
        let room = room.file_name().to_string_lossy().to_string();
        for capture in std::fs::read_dir(core.dir.join("rooms").join(&room).join(&me)).into_iter().flatten().flatten() {
            let id = capture.file_name().to_string_lossy().to_string();
            for which in 0..SERIES.len() {
                recover_series(core, &room, &id, &capture.path(), which);
            }
        }
        let own = core.own_log(&room);
        let mut last: HashMap<&str, &Value> = HashMap::new();
        for r in own.iter().filter(|r| r["k"] == "rec" || r["k"] == "rec_end") {
            if let Some(set) = r["set"].as_str() {
                last.insert(set, r);
            }
        }
        for (set, rec) in last.into_iter().filter(|(_, r)| r["k"] == "rec") {
            let end = own.iter().filter(|r| r["k"] == "seg" && r["set"] == set).filter_map(|r| Some(r["t_us"].as_i64()? + r["d_us"].as_i64()?)).max();
            core.append_log(&room, &json!({"k": "rec_end", "set": set, "room": room, "t_us": end.or(rec["start_us"].as_i64()).unwrap_or_else(now_us)}));
        }
    }
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
        debug::frame(debug::ARRIVED);
        if self.last.elapsed() < FRAME_GAP || w < 2 || h < 2 {
            return Ok(());
        }
        debug::frame(debug::TAKEN);
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
        let frame = Arc::new(if same { data.to_vec() } else { std::mem::take(&mut self.fitted) });
        if let Some(sink) = &sink {
            sink.write_video(frame.clone());
        }
        *e.last.lock().unwrap() = (frame, size, Instant::now());
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
            debug::frame(debug::REPEATED);
            sink.write_video(last.0.clone());
        }
        last.2 = Instant::now() - (REPEAT_AFTER - REPEAT_POLL);
    }
}

fn meters(core: Arc<Core>) {
    let mut quiet = false;
    loop {
        thread::sleep(Duration::from_millis(1000 / LEVELS_PER_SECOND));
        let [app, mic] = [APP, MIC].map(|t| f32::from_bits(core.engine.peaks[t].swap(0, Ordering::Relaxed)));
        let silent = app == 0.0 && mic == 0.0;
        if !(silent && quiet) {
            let _ = core.app.emit("levels", json!({"app": app, "mic": mic}));
        }
        quiet = silent;
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
            e.last.lock().unwrap().0 = Arc::new(Vec::new());
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
        Err(err) => e.failed(false, &format!("screen capture could not start: {err}")),
    }
}

fn check_sink(core: &Core) {
    let e = &core.engine;
    let Some(sink) = e.sink.lock().unwrap().clone() else { return };
    let (watch, now) = (*sink.watch.lock().unwrap(), Instant::now());
    let mut child = sink.child.lock().unwrap();
    if matches!(child.try_wait(), Ok(Some(_))) {
        sink.dead.store(true, Ordering::Relaxed);
    } else if watch.stalled(now) {
        tracing::warn!("{} produced nothing for {} s, stopping it", sink.encoder, STALL_AFTER.as_secs());
        sink.dead.store(true, Ordering::Relaxed);
        let _ = child.kill();
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

fn test_source(core: &Core) {
    let e = &core.engine;
    if ffmpeg::encoders_ready() {
        *e.canvas.lock().unwrap() = Some(TEST_SIZE);
    }
    let mut names = e.names.lock().unwrap();
    names.kind = "test".into();
    names.source = "Test picture".into();
    names.game = "Test tone".into();
    names.mic = "Test tone".into();
}

pub fn supervise(core: Arc<Core>) {
    let c = core.clone();
    thread::spawn(move || repeat(c));
    let c = core.clone();
    thread::spawn(move || meters(c));
    let e = &core.engine;
    let mut scanned: Option<(Value, Instant)> = None;
    let mut found: Option<Found> = None;
    loop {
        if crate::test_mode() {
            test_source(&core);
        } else {
            let source = e.source.lock().unwrap().clone();
            let ended = e.video.lock().unwrap().as_ref().is_some_and(|v| v.control.is_finished());
            if ended || scanned.as_ref().is_none_or(|(s, at)| *s != source || at.elapsed() > RESCAN_EVERY) {
                found = rescan(&core, &source);
                scanned = Some((source, Instant::now()));
            }
            audio::ensure(&core, APP, found.as_ref().map_or(0, |f| f.game.1).to_string());
            let mic = e.mic.lock().unwrap().clone();
            audio::ensure(&core, MIC, mic);
            ensure_video(&core, found.as_ref().filter(|_| want(&core).is_some() && ffmpeg::encoders_ready()));
        }
        check_sink(&core);
        thread::sleep(SUPERVISE_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn strikes_count_one_encoder_in_a_row() {
        let a = strike(("", 0), "av1_nvenc");
        let b = strike(a, "av1_nvenc");
        assert_eq!(strike(b, "av1_nvenc"), ("av1_nvenc", DROP_AFTER));
        assert_eq!(strike(b, "hevc_nvenc"), ("hevc_nvenc", 1));
    }
}

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::Emitter;
use wasapi::{initialize_mta, AudioCaptureClient, AudioClient, DeviceEnumerator, Direction, Handle, SampleType, StreamMode, WaveFormat};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, GraphicsCaptureItemType,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use crate::{now_us, Core};

type Err = Box<dyn std::error::Error + Send + Sync>;


const NO_WINDOW: u32 = 0x08000000;
const BELOW_NORMAL: u32 = 0x00004000;
const FPS: u32 = 30;

static DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub fn set_data_dir(dir: PathBuf) {
    let _ = DATA_DIR.set(dir);
}

pub fn ffmpeg() -> PathBuf {
    let beside = std::env::current_exe().ok().and_then(|exe| exe.parent().map(|dir| dir.join("ffmpeg.exe")));
    let downloaded = DATA_DIR.get().map(|dir| dir.join("ffmpeg.exe"));
    [beside, downloaded].into_iter().flatten().find(|p| p.exists()).unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

pub fn ffmpeg_present() -> bool {
    ffmpeg().is_absolute()
}

const FFMPEG_URL: &str = "https://github.com/GyanD/codexffmpeg/releases/download/8.1/ffmpeg-8.1-essentials_build.zip";
const FFMPEG_SHA256: &str = "8748283d821613d930b0e7be685aaa9df4ca6f0ad4d0c42fd02622b3623463c6";

pub fn download_ffmpeg() -> Result<(), String> {
    let dir = DATA_DIR.get().ok_or("no data folder")?;
    let script = format!(
        "$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'; \
         $zip = Join-Path '{dir}' 'ffmpeg.zip'; \
         Invoke-WebRequest -UseBasicParsing -Uri '{FFMPEG_URL}' -OutFile $zip; \
         if ((Get-FileHash $zip -Algorithm SHA256).Hash.ToLower() -ne '{FFMPEG_SHA256}') {{ Remove-Item $zip; throw 'checksum mismatch' }}; \
         Add-Type -AssemblyName System.IO.Compression.FileSystem; \
         $z = [IO.Compression.ZipFile]::OpenRead($zip); \
         $e = $z.Entries | Where-Object {{ $_.Name -eq 'ffmpeg.exe' }} | Select-Object -First 1; \
         [IO.Compression.ZipFileExtensions]::ExtractToFile($e, (Join-Path '{dir}' 'ffmpeg.exe.part'), $true); \
         $z.Dispose(); Remove-Item $zip; \
         Move-Item -Force (Join-Path '{dir}' 'ffmpeg.exe.part') (Join-Path '{dir}' 'ffmpeg.exe')",
        dir = dir.display()
    );
    let out = Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .creation_flags(NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub fn encoder_results() -> Vec<(&'static str, bool)> {
    static CACHE: Mutex<Option<Vec<(&'static str, bool)>>> = Mutex::new(None);
    if !ffmpeg_present() {
        return Vec::new();
    }
    let mut cache = CACHE.lock().unwrap();
    cache
        .get_or_insert_with(|| {
            CANDIDATES.iter().map(|(name, _, _)| *name)
                .map(|name| {
                    let ok = Command::new(ffmpeg())
                        .args(["-v", "quiet", "-f", "lavfi", "-i", "testsrc2=size=1280x720:rate=30", "-t", "1", "-c:v", name, "-f", "null", "-"])
                        .creation_flags(NO_WINDOW)
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                    (name, ok)
                })
                .collect()
        })
        .clone()
}

const CANDIDATES: [(&str, &str, f64); 10] = [
    ("av1_nvenc", "av1", 0.55),
    ("av1_amf", "av1", 0.55),
    ("av1_qsv", "av1", 0.55),
    ("hevc_nvenc", "hevc", 0.7),
    ("hevc_amf", "hevc", 0.7),
    ("hevc_qsv", "hevc", 0.7),
    ("h264_nvenc", "h264", 1.0),
    ("h264_amf", "h264", 1.0),
    ("h264_qsv", "h264", 1.0),
    ("libx264", "h264", 1.0),
];
const BASE_BITRATE_KBPS: f64 = 6000.0;

pub fn pick(usable: &[String]) -> (&'static str, &'static str, f64) {
    let results = encoder_results();
    CANDIDATES
        .iter()
        .find(|(name, codec, _)| usable.iter().any(|u| u == codec) && results.iter().any(|(n, ok)| n == name && *ok))
        .copied()
        .unwrap_or(("libx264", "h264", 1.0))
}

pub fn encoder() -> &'static str {
    pick(&["h264".to_string()]).0
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

pub fn check_capture() -> Vec<(&'static str, String)> {
    let mut lines = Vec::new();
    let got = Arc::new(AtomicBool::new(false));
    let Ok(primary) = Monitor::primary() else {
        return vec![("fail", "no display found".to_string())];
    };
    let mut control = Probe::start_free_threaded(settings(primary, DrawBorderSettings::WithoutBorder, got.clone()));
    if let Err(e) = &control {
        lines.push(("warn", format!("capture without border was refused ({e}); falling back to the default border")));
        control = Probe::start_free_threaded(settings(primary, DrawBorderSettings::Default, got.clone()));
    }
    match control {
        Err(e) => lines.push(("fail", format!("screen capture could not start: {e}"))),
        Ok(control) => {
            let until = Instant::now() + Duration::from_millis(2000);
            while !got.load(Ordering::Relaxed) && Instant::now() < until {
                thread::sleep(Duration::from_millis(50));
            }
            let _ = control.stop();
            if got.load(Ordering::Relaxed) {
                lines.push(("ok", "screen capture started and a frame arrived".into()));
            } else {
                lines.push(("warn", "screen capture started but no frame arrived in 2 s (idle or locked screen)".into()));
            }
        }
    }
    lines
}


const FRAME_GAP: Duration = Duration::from_millis(30);
const REPEAT_AFTER: Duration = Duration::from_millis(250);
const REPEAT_POLL: Duration = Duration::from_millis(100);
const SAMPLE_RATE: usize = 48000;
const CHANNELS: usize = 2;
const FLOAT_FRAME_BYTES: usize = 8;
const PCM_FRAME_BYTES: u64 = 4;
const PAD_AFTER_FRAMES: u64 = SAMPLE_RATE as u64 / 50;
const AUDIO_WAIT_MS: u32 = 20;
const SHARED_MAX_WIDTH: u32 = 1280;
const LEVELS_PER_SECOND: u64 = 30;
const LIVE_ROTATE_BYTES: u64 = 100 << 20;
const SUPERVISE_EVERY: Duration = Duration::from_millis(500);
const RESCAN_EVERY: Duration = Duration::from_secs(5);
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(30);
const STABLE_AFTER: Duration = Duration::from_secs(60);
const FINISH_WAIT: Duration = Duration::from_secs(10);
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
    pub path: PathBuf,
    size: (u32, u32),
    meta: Value,
    child: Mutex<Child>,
    video: Mutex<Option<ChildStdin>>,
    tracks: [Mutex<Track>; 2],
    closers: Mutex<Vec<TcpStream>>,
    start: OnceLock<Instant>,
    closed: AtomicBool,
    dead: AtomicBool,
    rotate: AtomicBool,
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

struct Audio {
    key: String,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

pub struct Engine {
    pub source: Mutex<Value>,
    pub mic: Mutex<String>,
    pub mix: Mutex<Value>,
    pub source_lost: AtomicBool,
    pub status: Mutex<String>,
    pub names: Mutex<Names>,
    pub sink: Mutex<Option<Arc<Sink>>>,
    video: Mutex<Option<Video>>,
    audio: [Mutex<Option<Audio>>; 2],
    canvas: Mutex<Option<(u32, u32)>>,
    last: Mutex<(Vec<u8>, (u32, u32), Instant)>,
    peaks: [AtomicU32; 2],
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
            video: Mutex::new(None),
            audio: [Mutex::new(None), Mutex::new(None)],
            canvas: Mutex::new(None),
            last: Mutex::new((Vec::new(), (0, 0), Instant::now())),
            peaks: [AtomicU32::new(0), AtomicU32::new(0)],
            failures: AtomicU32::new(0),
            retry_at: Mutex::new(Instant::now()),
        }
    }

    pub fn mix_key(&self, track: usize) -> String {
        if track == GAME { "game".into() } else { format!("mic:{}", self.names.lock().unwrap().mic_id) }
    }

    fn gain(&self, track: usize) -> (f32, bool) {
        let key = self.mix_key(track);
        let mix = self.mix.lock().unwrap();
        (mix[&key]["volume"].as_f64().unwrap_or(1.0) as f32, mix[&key]["muted"].as_bool().unwrap_or(false))
    }

    pub fn live_file(&self) -> Option<String> {
        let sink = self.sink.lock().unwrap();
        let sink = sink.as_ref().filter(|s| s.target.set.is_none())?;
        Some(sink.path.file_name()?.to_string_lossy().to_string())
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

pub fn live_name(name: &str) -> Option<(String, i64)> {
    let (who, ms) = name.strip_suffix(".mp4")?.rsplit_once('-')?;
    Some((who.to_string(), ms.parse().ok()?))
}

pub fn want(core: &Core) -> Option<Target> {
    let e = &core.engine;
    if e.source.lock().unwrap().is_null() || core.disk_full() {
        return None;
    }
    if let Some(s) = core.active_set.lock().unwrap().as_ref() {
        return Some(Target { room: s.room.clone(), set: Some(s.id.clone()) });
    }
    let room = core.active_room()?;
    (core.share.load(Ordering::Relaxed) && !e.source_lost.load(Ordering::Relaxed)).then_some(Target { room: room.id, set: None })
}

impl Sink {
    fn start(core: &Arc<Core>, target: Target, size: (u32, u32)) -> Result<Arc<Self>, Err> {
        if !ffmpeg_present() {
            return Err("ffmpeg is not installed yet".into());
        }
        let id = core.ep.id().to_string();
        let (dir, file) = match &target.set {
            Some(set) => (core.set_dir(&target.room, set), format!("{id}.mp4")),
            None => (core.live_dir(&target.room), format!("{id}-{}.mp4", now_us() / 1000)),
        };
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(&file);
        let mut usable = core.codecs.lock().unwrap().clone();
        for p in core.peers.lock().unwrap().iter().filter(|p| p.conn.is_some()) {
            usable.retain(|c| p.codecs.contains(c));
        }
        let chosen = pick(&usable);
        let listeners = [TcpListener::bind("127.0.0.1:0")?, TcpListener::bind("127.0.0.1:0")?];
        let rate = SAMPLE_RATE.to_string();
        let channels = CHANNELS.to_string();
        let mut command = Command::new(ffmpeg());
        command
            .args(["-hide_banner", "-loglevel", "warning", "-y"])
            .args(["-use_wallclock_as_timestamps", "1", "-thread_queue_size", "512"])
            .args(["-f", "rawvideo", "-pix_fmt", "bgra", "-video_size", &format!("{}x{}", size.0, size.1), "-i", "-"]);
        for listener in &listeners {
            let input = format!("tcp://127.0.0.1:{}", listener.local_addr()?.port());
            command.args(["-thread_queue_size", "512", "-f", "s16le", "-ar", &rate, "-ac", &channels, "-i", &input]);
        }
        let mut child = command
            .args(["-map", "0:v", "-map", "1:a", "-map", "2:a"])
            .args(["-vf", &format!("scale={}:-2:flags=fast_bilinear,format=yuv420p", SHARED_MAX_WIDTH.min(size.0) & !1), "-r", &FPS.to_string()])
            .args(["-c:v", chosen.0, "-bf", "0", "-g", &FPS.to_string(), "-b:v", &format!("{}k", (BASE_BITRATE_KBPS * chosen.2) as u32)])
            .args(if chosen.0 == "libx264" { ["-preset", "ultrafast"] } else { ["-v", "warning"] })
            .args(if chosen.1 == "hevc" { ["-tag:v", "hvc1"] } else { ["-v", "warning"] })
            .args(["-force_key_frames", "expr:gte(t,n_forced*1)"])
            .args(["-c:a", "aac", "-b:a", "128k", "-metadata:s:a:0", "title=Game", "-metadata:s:a:1", "title=Microphone"])
            .args(["-flush_packets", "1", "-f", "mp4", "-movflags", "frag_keyframe+empty_moov+default_base_moof"])
            .arg(&path)
            .stdin(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(NO_WINDOW | BELOW_NORMAL)
            .spawn()?;
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
            meta.as_object_mut().unwrap().extend(json!({"set": set, "room": target.room, "file": file, "by": by}).as_object().cloned().unwrap_or_default());
        }
        let sink = Arc::new(Self {
            target,
            path,
            size,
            meta,
            video: Mutex::new(child.stdin.take()),
            child: Mutex::new(child),
            tracks: [Mutex::new(Track::default()), Mutex::new(Track::default())],
            closers: Mutex::new(Vec::new()),
            start: OnceLock::new(),
            closed: AtomicBool::new(false),
            dead: AtomicBool::new(false),
            rotate: AtomicBool::new(false),
            born: Instant::now(),
        });
        for (track, listener) in listeners.into_iter().enumerate() {
            let sink = sink.clone();
            thread::spawn(move || sink.accept(track, listener));
        }
        Ok(sink)
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
            record.as_object_mut().unwrap().extend(self.meta.as_object().cloned().unwrap_or_default());
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

    fn write_audio(&self, track: usize, pcm: &[u8]) {
        let Some(start) = self.start.get() else { return };
        if self.closed.load(Ordering::Relaxed) {
            return;
        }
        let mut t = self.tracks[track].lock().unwrap();
        let due = (start.elapsed().as_secs_f64() * SAMPLE_RATE as f64) as u64;
        let lag = due.saturating_sub(t.written);
        let mut out = Vec::new();
        if (t.written == 0 || pcm.is_empty()) && lag > PAD_AFTER_FRAMES {
            out.resize((lag * PCM_FRAME_BYTES) as usize, 0);
        }
        out.extend_from_slice(pcm);
        let Some(stream) = t.stream.as_mut() else { return };
        if out.is_empty() {
            return;
        }
        if stream.write_all(&out).is_err() {
            t.stream = None;
            if !self.closed.load(Ordering::Relaxed) {
                self.dead.store(true, Ordering::Relaxed);
            }
            return;
        }
        t.written += out.len() as u64 / PCM_FRAME_BYTES;
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
        (sink.dead.load(Ordering::Relaxed), resized || sink.rotate.load(Ordering::Relaxed) || Some(&sink.target) != want.as_ref())
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
        let done = target.set.as_ref().is_some_and(|set| core.set_dir(&target.room, set).join(format!("{}.mp4", core.ep.id())).exists());
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
    if (w, h) == (cw, ch) {
        out.extend_from_slice(src);
        return;
    }
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
        fit(data, (w, h), size, &mut self.fitted);
        if let Some(sink) = &sink {
            sink.write_video(&self.core, &self.fitted);
        }
        let mut last = e.last.lock().unwrap();
        last.0.clear();
        last.0.extend_from_slice(&self.fitted);
        last.1 = size;
        last.2 = Instant::now();
        Ok(())
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
        if let Some(sink) = sink.filter(|s| s.size == last.1) {
            sink.write_video(&core, &last.0);
        }
        last.2 = Instant::now() - (REPEAT_AFTER - REPEAT_POLL);
    }
}

fn meters(core: Arc<Core>) {
    loop {
        thread::sleep(Duration::from_millis(1000 / LEVELS_PER_SECOND));
        let [game, mic] = [GAME, MIC].map(|t| f32::from_bits(core.engine.peaks[t].swap(0, Ordering::Relaxed)));
        let _ = core.app.emit("levels", json!({"game": game, "mic": mic}));
    }
}

struct Opened {
    client: AudioClient,
    event: Handle,
    capture: AudioCaptureClient,
}

fn open_audio(core: &Core, track: usize, key: &str) -> Result<Option<Opened>, Err> {
    let format = WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE, CHANNELS, None);
    let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
    let mut found = None;
    let mut client = if track == GAME {
        let pid: u32 = key.parse().unwrap_or(0);
        if pid == 0 {
            return Ok(None);
        }
        AudioClient::new_application_loopback_client(pid, true)?
    } else {
        let devices = DeviceEnumerator::new()?;
        let device = match devices.get_device(key) {
            Ok(device) if !key.is_empty() => device,
            _ => devices.get_default_device(&Direction::Capture)?,
        };
        found = Some((device.get_id()?, device.get_friendlyname()?));
        device.get_iaudioclient()?
    };
    client.initialize_client(&format, &Direction::Capture, &mode)?;
    let event = client.set_get_eventhandle()?;
    let capture = client.get_audiocaptureclient()?;
    client.start_stream()?;
    if let Some((id, name)) = found {
        let mut names = core.engine.names.lock().unwrap();
        names.mic_id = id;
        names.mic = name;
    }
    Ok(Some(Opened { client, event, capture }))
}

fn read_audio(opened: &Opened, bytes: &mut VecDeque<u8>) -> Result<(), Err> {
    let _ = opened.event.wait_for_event(AUDIO_WAIT_MS);
    while opened.capture.get_next_packet_size()?.unwrap_or(0) > 0 {
        opened.capture.read_from_device_to_deque(bytes)?;
    }
    Ok(())
}

fn audio_loop(core: &Core, track: usize, key: &str, stop: &AtomicBool) -> Result<(), Err> {
    initialize_mta().ok()?;
    let e = &core.engine;
    let mut source: Option<Opened> = None;
    let mut tried: Option<Instant> = None;
    let mut bytes: VecDeque<u8> = VecDeque::new();
    while !stop.load(Ordering::Relaxed) {
        if source.is_none() && tried.is_none_or(|t| t.elapsed() > RESCAN_EVERY) {
            tried = Some(Instant::now());
            source = open_audio(core, track, key).unwrap_or_else(|err| {
                tracing::debug!("audio source {track} unavailable: {err}");
                None
            });
        }
        match &source {
            Some(opened) => {
                if read_audio(opened, &mut bytes).is_err() {
                    source = None;
                    bytes.clear();
                }
            }
            None => thread::sleep(Duration::from_millis(AUDIO_WAIT_MS as u64)),
        }
        if source.is_none() && track == MIC {
            let mut names = e.names.lock().unwrap();
            names.mic_id.clear();
            names.mic.clear();
        }
        let whole = bytes.len() / FLOAT_FRAME_BYTES * FLOAT_FRAME_BYTES;
        let raw: Vec<u8> = bytes.drain(..whole).collect();
        let (volume, muted) = e.gain(track);
        let mut peak = 0f32;
        let pcm: Vec<u8> = raw
            .chunks_exact(4)
            .flat_map(|c| {
                let v = (f32::from_le_bytes([c[0], c[1], c[2], c[3]]) * volume).clamp(-1.0, 1.0);
                peak = peak.max(v.abs());
                (if muted { 0i16 } else { (v * i16::MAX as f32) as i16 }).to_le_bytes()
            })
            .collect();
        e.peaks[track].fetch_max(peak.to_bits(), Ordering::Relaxed);
        let sink = e.sink.lock().unwrap().clone();
        if let Some(sink) = sink {
            sink.write_audio(track, &pcm);
        }
    }
    if let Some(opened) = &source {
        let _ = opened.client.stop_stream();
    }
    Ok(())
}

fn ensure_audio(core: &Arc<Core>, track: usize, key: String) {
    let mut slot = core.engine.audio[track].lock().unwrap();
    if slot.as_ref().is_some_and(|a| a.key == key && !a.thread.is_finished()) {
        return;
    }
    if let Some(old) = slot.take() {
        old.stop.store(true, Ordering::Relaxed);
        let _ = old.thread.join();
    }
    let stop = Arc::new(AtomicBool::new(false));
    let (c, s, k) = (core.clone(), stop.clone(), key.clone());
    let thread = thread::spawn(move || {
        if let Err(e) = audio_loop(&c, track, &k, &s) {
            tracing::warn!("audio {track} failed: {e}");
        }
    });
    *slot = Some(Audio { key, stop, thread });
}

#[derive(Clone, Copy)]
enum Item {
    Window(Window),
    Display(Monitor),
}

struct Found {
    item: Item,
    key: usize,
    kind: &'static str,
    name: String,
    game: (String, u32),
}

fn windows() -> Vec<(Window, String, String, u32)> {
    let names: HashMap<u32, String> = crate::processes().into_iter().map(|(name, pid)| (pid, name)).collect();
    let own = std::process::id();
    Window::enumerate()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|w| {
            let pid = w.process_id().ok().filter(|p| *p != own)?;
            let title = w.title().ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())?;
            if w.width().ok()? <= 0 || w.height().ok()? <= 0 {
                return None;
            }
            Some((w, names.get(&pid)?.clone(), title, pid))
        })
        .collect()
}

fn is_game(exe: &str) -> bool {
    exe.to_lowercase().starts_with("r5apex")
}

pub fn list_sources() -> Vec<Value> {
    let mut list: Vec<(Window, String, String, u32)> = windows();
    list.sort_by_key(|w| !is_game(&w.1));
    let windows = list.into_iter().map(|(_, exe, title, _)| json!({"id": {"kind": "window", "exe": exe, "title": title}, "name": format!("{title} ({exe})")}));
    let displays = Monitor::enumerate().unwrap_or_default().into_iter().enumerate().map(|(i, m)| {
        let size = m.width().and_then(|w| Ok((w, m.height()?))).map(|(w, h)| format!(" ({w}x{h})")).unwrap_or_default();
        json!({"id": {"kind": "display", "index": i + 1}, "name": format!("Display {}{size}", i + 1)})
    });
    windows.chain(displays).collect()
}

pub fn primary_display() -> Value {
    let all = Monitor::enumerate().unwrap_or_default();
    let index = Monitor::primary().ok().and_then(|p| all.iter().position(|m| *m == p)).unwrap_or(0);
    json!({"kind": "display", "index": index + 1})
}

pub fn valid_source(source: &Value) -> bool {
    match source["kind"].as_str() {
        Some("window") => source["exe"].is_string() && source["title"].is_string(),
        Some("display") => source["index"].as_u64().is_some_and(|i| i > 0),
        _ => source.is_null(),
    }
}

fn find(core: &Core, source: &Value) -> Option<Found> {
    match source["kind"].as_str()? {
        "window" => {
            let (exe, title) = (source["exe"].as_str()?, source["title"].as_str()?);
            let list = windows();
            let (w, exe, title, pid) = list
                .iter()
                .find(|w| w.1.eq_ignore_ascii_case(exe) && w.2 == title)
                .or_else(|| list.iter().find(|w| w.1.eq_ignore_ascii_case(exe)))?
                .clone();
            Some(Found { item: Item::Window(w), key: w.as_raw_hwnd() as usize, kind: "window", name: format!("{title} ({exe})"), game: (exe, pid) })
        }
        "display" => {
            let index = source["index"].as_u64()? as usize;
            let monitor = *Monitor::enumerate().ok()?.get(index.checked_sub(1)?)?;
            let game = crate::audio_target(&core.audio_process.lock().unwrap());
            Some(Found { item: Item::Display(monitor), key: monitor.as_raw_hmonitor() as usize, kind: "display", name: format!("Display {index}"), game })
        }
        _ => None,
    }
}

fn start_item<T: TryInto<GraphicsCaptureItemType> + Send + Copy + 'static>(core: &Arc<Core>, item: T) -> Result<CaptureControl<Capture, Err>, Err> {
    match Capture::start_free_threaded(settings(item, DrawBorderSettings::WithoutBorder, core.clone())) {
        Ok(control) => Ok(control),
        Err(e) => {
            tracing::warn!("capture without border refused ({e}), using the default border");
            Ok(Capture::start_free_threaded(settings(item, DrawBorderSettings::Default, core.clone()))?)
        }
    }
}

fn clean_live(core: &Core) {
    let own = core.ep.id().to_string();
    let current = core.engine.sink.lock().unwrap().as_ref().map(|s| s.path.clone());
    let Ok(rooms) = std::fs::read_dir(core.dir.join("rooms")) else { return };
    for room in rooms.flatten() {
        let Ok(files) = std::fs::read_dir(room.path().join("live")) else { continue };
        let all: Vec<(String, i64, PathBuf)> = files
            .flatten()
            .filter_map(|f| live_name(&f.file_name().to_string_lossy()).map(|(who, ms)| (who, ms, f.path())))
            .collect();
        let mut newest: HashMap<&str, i64> = HashMap::new();
        for (who, ms, _) in &all {
            let best = newest.entry(who).or_insert(*ms);
            *best = (*best).max(*ms);
        }
        for (who, ms, path) in &all {
            let keep = if *who == own { current.as_ref() == Some(path) } else { newest[who.as_str()] == *ms };
            if !keep {
                let _ = std::fs::remove_file(path);
            }
        }
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
    loop {
        let source = e.source.lock().unwrap().clone();
        let ended = e.video.lock().unwrap().as_ref().is_some_and(|v| v.control.is_finished());
        if ended || scanned.as_ref().is_none_or(|(s, at)| *s != source || at.elapsed() > RESCAN_EVERY) {
            found = find(&core, &source);
            e.source_lost.store(!source.is_null() && found.is_none(), Ordering::Relaxed);
            let mut names = e.names.lock().unwrap();
            names.kind = found.as_ref().map(|f| f.kind.to_string()).unwrap_or_default();
            names.source = found.as_ref().map(|f| f.name.clone()).unwrap_or_default();
            names.game = found.as_ref().map(|f| f.game.0.clone()).unwrap_or_default();
            drop(names);
            scanned = Some((source, Instant::now()));
        }
        ensure_audio(&core, GAME, found.as_ref().map_or(0, |f| f.game.1).to_string());
        let mic = e.mic.lock().unwrap().clone();
        ensure_audio(&core, MIC, mic);
        let wanted = found.as_ref().filter(|_| want(&core).is_some() && ffmpeg_present());
        {
            let mut video = e.video.lock().unwrap();
            let stale = video.as_ref().is_some_and(|v| v.control.is_finished() || wanted.is_none_or(|f| f.key != v.key));
            if stale {
                let v = video.take().unwrap();
                let closed = v.control.is_finished();
                let _ = v.control.stop();
                if !closed {
                    e.last.lock().unwrap().0.clear();
                    *e.canvas.lock().unwrap() = None;
                }
            }
            if let Some(f) = wanted.filter(|_| video.is_none()) {
                if Instant::now() >= *e.retry_at.lock().unwrap() {
                    let started = match f.item {
                        Item::Window(w) => start_item(&core, w),
                        Item::Display(m) => start_item(&core, m),
                    };
                    match started {
                        Ok(control) => {
                            *e.canvas.lock().unwrap() = None;
                            *video = Some(Video { control, key: f.key });
                        }
                        Err(err) => e.failed(Instant::now(), &format!("screen capture could not start: {err}")),
                    }
                }
            }
        }
        let sink = e.sink.lock().unwrap().clone();
        if let Some(sink) = sink {
            if matches!(sink.child.lock().unwrap().try_wait(), Ok(Some(_))) {
                sink.dead.store(true, Ordering::Relaxed);
            }
            if sink.target.set.is_none() && std::fs::metadata(&sink.path).map(|m| m.len()).unwrap_or(0) > LIVE_ROTATE_BYTES {
                sink.rotate.store(true, Ordering::Relaxed);
            }
            if sink.born.elapsed() > STABLE_AFTER {
                e.failures.store(0, Ordering::Relaxed);
                e.status.lock().unwrap().clear();
            }
        }
        clean_live(&core);
        thread::sleep(SUPERVISE_EVERY);
    }
}

pub fn list_mics() -> Vec<Value> {
    let _ = initialize_mta();
    let Ok(devices) = DeviceEnumerator::new() else { return Vec::new() };
    let default = devices.get_default_device(&Direction::Capture).and_then(|d| d.get_id()).unwrap_or_default();
    let Ok(collection) = devices.get_device_collection(&Direction::Capture) else { return Vec::new() };
    (&collection)
        .into_iter()
        .flatten()
        .filter_map(|d| Some(json!({"id": d.get_id().ok()?, "name": d.get_friendlyname().ok()?, "default": d.get_id().ok()? == default})))
        .collect()
}

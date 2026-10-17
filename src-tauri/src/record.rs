use std::collections::VecDeque;
use std::io::Write;
use std::net::TcpListener;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::json;
use wasapi::{initialize_mta, AudioClient, Direction, SampleType, StreamMode, WaveFormat};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

use crate::{log, now_us};

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

struct Shared {
    stdin: Mutex<Option<ChildStdin>>,
    last: Mutex<(Vec<u8>, Instant)>,
    started: AtomicBool,
    stop: AtomicBool,
}

struct Flags {
    shared: Arc<Shared>,
    dir: PathBuf,
    name: String,
    meta: serde_json::Value,
}

struct Capture {
    flags: Flags,
    buffer: Vec<u8>,
    last: Instant,
}

impl GraphicsCaptureApiHandler for Capture {
    type Flags = Flags;
    type Error = Err;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self { flags: ctx.flags, buffer: Vec::new(), last: Instant::now() - Duration::from_secs(1) })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, control: InternalCaptureControl) -> Result<(), Self::Error> {
        let shared = &self.flags.shared;
        if shared.stop.load(Ordering::Relaxed) {
            control.stop();
            return Ok(());
        }
        if self.last.elapsed() < Duration::from_millis(30) {
            return Ok(());
        }
        self.last = Instant::now();
        let buffer = frame.buffer()?;
        let data = buffer.as_nopadding_buffer(&mut self.buffer);
        if !shared.started.swap(true, Ordering::Relaxed) {
            let mut record = json!({"k": "rec", "name": self.flags.name, "start_us": now_us()});
            record.as_object_mut().unwrap().extend(self.flags.meta.as_object().cloned().unwrap_or_default());
            log::append(&self.flags.dir.join("log.bin"), &record)?;
        }
        {
            let mut last = shared.last.lock().unwrap();
            last.0.clear();
            last.0.extend_from_slice(data);
            last.1 = Instant::now();
        }
        let mut stdin = shared.stdin.lock().unwrap();
        let Some(pipe) = stdin.as_mut() else {
            shared.stop.store(true, Ordering::Relaxed);
            control.stop();
            return Ok(());
        };
        if pipe.write_all(data).is_err() {
            *stdin = None;
            shared.stop.store(true, Ordering::Relaxed);
            control.stop();
        }
        Ok(())
    }
}

pub struct Recorder {
    pub name: String,
    control: CaptureControl<Capture, Err>,
    shared: Arc<Shared>,
    child: Child,
    audio: JoinHandle<()>,
}

pub fn encoder_results() -> Vec<(&'static str, bool)> {
    static CACHE: Mutex<Option<Vec<(&'static str, bool)>>> = Mutex::new(None);
    if !ffmpeg_present() {
        return Vec::new();
    }
    let mut cache = CACHE.lock().unwrap();
    cache
        .get_or_insert_with(|| {
            ["h264_nvenc", "h264_amf", "h264_qsv", "libx264"]
                .into_iter()
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

pub fn encoder() -> &'static str {
    encoder_results().into_iter().find(|(_, ok)| *ok).map_or("libx264", |(name, _)| name)
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

fn settings<F>(border: DrawBorderSettings, flags: F) -> Result<Settings<F, Monitor>, Err> {
    Ok(Settings::new(
        Monitor::primary()?,
        CursorCaptureSettings::Default,
        border,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    ))
}

pub fn check_capture() -> Vec<(&'static str, String)> {
    let mut lines = Vec::new();
    let got = Arc::new(AtomicBool::new(false));
    let mut control = Probe::start_free_threaded(settings(DrawBorderSettings::WithoutBorder, got.clone()).unwrap());
    if let Err(e) = &control {
        lines.push(("warn", format!("capture without border was refused ({e}); falling back to the default border")));
        control = Probe::start_free_threaded(settings(DrawBorderSettings::Default, got.clone()).unwrap());
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

fn audio_loop(pid: u32, listener: TcpListener, shared: Arc<Shared>) -> Result<(), Err> {
    listener.set_nonblocking(true)?;
    let mut out = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if shared.stop.load(Ordering::Relaxed) {
                    return Ok(());
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e.into()),
        }
    };
    out.set_nonblocking(false)?;
    initialize_mta().ok()?;
    let mut source = None;
    if pid != 0 {
        let format = WaveFormat::new(32, 32, &SampleType::Float, 48000, 2, None);
        let mut client = AudioClient::new_application_loopback_client(pid, true)?;
        let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
        client.initialize_client(&format, &Direction::Capture, &mode)?;
        let event = client.set_get_eventhandle()?;
        let capture = client.get_audiocaptureclient()?;
        client.start_stream()?;
        source = Some((client, event, capture));
    }
    while !shared.started.load(Ordering::Relaxed) {
        if shared.stop.load(Ordering::Relaxed) {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(5));
    }
    let start = Instant::now();
    let mut written = 0u64;
    let mut bytes: VecDeque<u8> = VecDeque::new();
    while !shared.stop.load(Ordering::Relaxed) {
        match &source {
            Some((_, event, capture)) => {
                let _ = event.wait_for_event(20);
                if capture.get_next_packet_size()?.unwrap_or(0) > 0 {
                    capture.read_from_device_to_deque(&mut bytes)?;
                }
            }
            None => thread::sleep(Duration::from_millis(20)),
        }
        let whole = bytes.len() / 8 * 8;
        let mut pcm: Vec<u8> = Vec::new();
        if whole > 0 {
            let raw: Vec<u8> = bytes.drain(..whole).collect();
            pcm.extend(raw.chunks_exact(4).flat_map(|c| {
                let v = f32::from_le_bytes([c[0], c[1], c[2], c[3]]).clamp(-1.0, 1.0);
                ((v * 32767.0) as i16).to_le_bytes()
            }));
        } else {
            let due = (start.elapsed().as_secs_f64() * 48000.0) as u64;
            if due > written + 960 {
                pcm.resize(((due - written) * 4) as usize, 0);
            }
        }
        if !pcm.is_empty() {
            written += pcm.len() as u64 / 4;
            if out.write_all(&pcm).is_err() {
                break;
            }
        }
    }
    if let Some((client, _, _)) = &source {
        client.stop_stream()?;
    }
    Ok(())
}

impl Recorder {
    pub fn start(dir: &Path, audio_pid: u32, audio_name: &str) -> Result<Self, Err> {
        if !ffmpeg_present() {
            return Err("ffmpeg is not installed yet".into());
        }
        let monitor = Monitor::primary()?;
        let (width, height) = (monitor.width()?, monitor.height()?);
        let name = format!("rec-{}.mp4", now_us() / 1000);
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let mut child = Command::new(ffmpeg())
            .args(["-hide_banner", "-loglevel", "warning", "-y"])
            .args(["-use_wallclock_as_timestamps", "1", "-thread_queue_size", "512"])
            .args(["-f", "rawvideo", "-pix_fmt", "bgra", "-video_size", &format!("{width}x{height}"), "-i", "-"])
            .args(["-thread_queue_size", "512"])
            .args(["-f", "s16le", "-ar", "48000", "-ac", "2", "-i", &format!("tcp://127.0.0.1:{port}")])
            .args(["-vf", "scale=1280:-2:flags=fast_bilinear,format=yuv420p", "-r", &FPS.to_string()])
            .args(["-c:v", encoder(), "-bf", "0", "-g", &FPS.to_string(), "-b:v", "6M"])
            .args(if encoder() == "libx264" { ["-preset", "ultrafast"] } else { ["-v", "warning"] })
            .args(["-force_key_frames", "expr:gte(t,n_forced*1)"])
            .args(["-c:a", "aac", "-b:a", "128k"])
            .args(["-f", "mp4", "-movflags", "frag_keyframe+empty_moov+default_base_moof"])
            .arg(dir.join(&name))
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
        let shared = Arc::new(Shared {
            stdin: Mutex::new(child.stdin.take()),
            last: Mutex::new((Vec::new(), Instant::now())),
            started: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let audio_shared = shared.clone();
        let audio = thread::spawn(move || {
            if let Err(e) = audio_loop(audio_pid, listener, audio_shared) {
                tracing::warn!("audio failed: {e}");
            }
        });
        let repeat = shared.clone();
        thread::spawn(move || {
            while !repeat.stop.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(100));
                let mut last = repeat.last.lock().unwrap();
                if last.0.is_empty() || last.1.elapsed() < Duration::from_millis(250) {
                    continue;
                }
                let mut stdin = repeat.stdin.lock().unwrap();
                let Some(pipe) = stdin.as_mut() else { break };
                if pipe.write_all(&last.0).is_err() {
                    *stdin = None;
                    repeat.stop.store(true, Ordering::Relaxed);
                    break;
                }
                last.1 = Instant::now() - Duration::from_millis(200);
            }
        });
        let meta = json!({"width": width, "height": height, "fps": FPS, "encoder": encoder(), "audio": audio_name, "version": crate::VERSION});
        let flags = || Flags { shared: shared.clone(), dir: dir.to_path_buf(), name: name.clone(), meta: meta.clone() };
        let mut started = Capture::start_free_threaded(settings(DrawBorderSettings::WithoutBorder, flags())?);
        if let Err(e) = &started {
            tracing::warn!("capture without border refused ({e}), using the default border");
            started = Capture::start_free_threaded(settings(DrawBorderSettings::Default, flags())?);
        }
        let control = match started {
            Ok(control) => control,
            Err(e) => {
                shared.stop.store(true, Ordering::Relaxed);
                *shared.stdin.lock().unwrap() = None;
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.into());
            }
        };
        Ok(Self { name, control, shared, child, audio })
    }

    pub fn is_dead(&mut self) -> bool {
        self.shared.stop.load(Ordering::Relaxed) || matches!(self.child.try_wait(), Ok(Some(_)))
    }

    pub fn stop(self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        let _ = self.control.stop();
        *self.shared.stdin.lock().unwrap() = None;
        let _ = self.audio.join();
        let mut child = self.child;
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            thread::sleep(Duration::from_millis(100));
        }
        tracing::warn!("ffmpeg did not finish in time, killing it");
        let _ = child.kill();
        let _ = child.wait();
    }
}

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

struct Shared {
    stdin: Mutex<Option<ChildStdin>>,
    started: AtomicBool,
    stop: AtomicBool,
}

struct Flags {
    shared: Arc<Shared>,
    dir: PathBuf,
    name: String,
}

struct Capture {
    flags: Flags,
    buffer: Vec<u8>,
}

impl GraphicsCaptureApiHandler for Capture {
    type Flags = Flags;
    type Error = Err;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self { flags: ctx.flags, buffer: Vec::new() })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, control: InternalCaptureControl) -> Result<(), Self::Error> {
        let shared = &self.flags.shared;
        if shared.stop.load(Ordering::Relaxed) {
            control.stop();
            return Ok(());
        }
        let buffer = frame.buffer()?;
        let data = buffer.as_nopadding_buffer(&mut self.buffer);
        if !shared.started.swap(true, Ordering::Relaxed) {
            let record = json!({"k": "rec", "name": self.flags.name, "start_us": now_us()});
            log::append(&self.flags.dir.join("log.bin"), &record)?;
        }
        let mut stdin = shared.stdin.lock().unwrap();
        let Some(pipe) = stdin.as_mut() else {
            control.stop();
            return Ok(());
        };
        if pipe.write_all(data).is_err() {
            *stdin = None;
            control.stop();
        }
        Ok(())
    }
}

pub struct Recorder {
    control: CaptureControl<Capture, Err>,
    shared: Arc<Shared>,
    child: Child,
    audio: JoinHandle<()>,
}

fn encoder() -> &'static str {
    for name in ["h264_nvenc", "h264_amf"] {
        let ok = Command::new("ffmpeg")
            .args(["-v", "quiet", "-f", "lavfi", "-i", "nullsrc=s=256x256", "-frames:v", "1", "-c:v", name, "-f", "null", "-"])
            .creation_flags(NO_WINDOW)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return name;
        }
    }
    "libx264"
}

fn audio_loop(pid: u32, listener: TcpListener, shared: Arc<Shared>) -> Result<(), Err> {
    let (mut out, _) = listener.accept()?;
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
    pub fn start(dir: &Path, audio_pid: u32) -> Result<Self, Err> {
        let monitor = Monitor::primary()?;
        let (width, height) = (monitor.width()?, monitor.height()?);
        let name = format!("rec-{}.mp4", now_us() / 1000);
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "warning", "-y"])
            .args(["-use_wallclock_as_timestamps", "1", "-thread_queue_size", "512"])
            .args(["-f", "rawvideo", "-pix_fmt", "bgra", "-video_size", &format!("{width}x{height}"), "-i", "-"])
            .args(["-thread_queue_size", "512"])
            .args(["-f", "s16le", "-ar", "48000", "-ac", "2", "-i", &format!("tcp://127.0.0.1:{port}")])
            .args(["-vf", "scale=1280:-2:flags=fast_bilinear,format=yuv420p", "-r", "30"])
            .args(["-c:v", encoder(), "-bf", "0", "-g", "30", "-b:v", "6M"])
            .args(["-force_key_frames", "expr:gte(t,n_forced*1)"])
            .args(["-c:a", "aac", "-b:a", "128k"])
            .args(["-f", "mp4", "-movflags", "frag_keyframe+empty_moov+default_base_moof"])
            .arg(dir.join(&name))
            .stdin(Stdio::piped())
            .creation_flags(NO_WINDOW)
            .spawn()?;
        let shared = Arc::new(Shared {
            stdin: Mutex::new(child.stdin.take()),
            started: AtomicBool::new(false),
            stop: AtomicBool::new(false),
        });
        let audio_shared = shared.clone();
        let audio = thread::spawn(move || {
            if let Err(e) = audio_loop(audio_pid, listener, audio_shared) {
                eprintln!("audio failed: {e}");
            }
        });
        let settings = Settings::new(
            monitor,
            CursorCaptureSettings::Default,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Default,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            Flags { shared: shared.clone(), dir: dir.to_path_buf(), name },
        );
        let control = Capture::start_free_threaded(settings)?;
        Ok(Self { control, shared, child, audio })
    }

    pub fn stop(self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        let _ = self.control.stop();
        *self.shared.stdin.lock().unwrap() = None;
        let _ = self.audio.join();
        let mut child = self.child;
        let _ = child.wait();
    }
}

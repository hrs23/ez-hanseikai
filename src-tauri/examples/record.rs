use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use wasapi::{initialize_mta, AudioClient, Direction, SampleType, StreamMode, WaveFormat};
use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
    VideoSettingsSubType,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

struct Flags {
    width: u32,
    height: u32,
    seconds: u64,
    audio: Receiver<Vec<u8>>,
    stop: Arc<AtomicBool>,
}

struct Record {
    encoder: Option<VideoEncoder>,
    flags: Flags,
    start: Instant,
    frames: u32,
    audio_bytes: u64,
}

impl GraphicsCaptureApiHandler for Record {
    type Flags = Flags;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let flags = ctx.flags;
        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(flags.width, flags.height)
                .sub_type(VideoSettingsSubType::H264)
                .bitrate(15_000_000)
                .frame_rate(60),
            AudioSettingsBuilder::default(),
            ContainerSettingsBuilder::default(),
            "record.mp4",
        )?;
        Ok(Self { encoder: Some(encoder), flags, start: Instant::now(), frames: 0, audio_bytes: 0 })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        let encoder = self.encoder.as_mut().unwrap();
        while let Ok(chunk) = self.flags.audio.try_recv() {
            self.audio_bytes += chunk.len() as u64;
            encoder.send_audio_buffer(&chunk, 0)?;
        }
        encoder.send_frame(frame)?;
        self.frames += 1;
        if self.start.elapsed() >= Duration::from_secs(self.flags.seconds) {
            self.flags.stop.store(true, Ordering::Relaxed);
            self.encoder.take().unwrap().finish()?;
            let secs = self.start.elapsed().as_secs_f64();
            println!(
                "{} frames in {:.2} s, {:.1} fps, audio {:.2} s",
                self.frames,
                secs,
                self.frames as f64 / secs,
                self.audio_bytes as f64 / 4.0 / 48000.0
            );
            control.stop();
        }
        Ok(())
    }
}

fn audio_loop(pid: u32, tx: Sender<Vec<u8>>, stop: Arc<AtomicBool>) -> Result<(), Box<dyn std::error::Error>> {
    initialize_mta().ok()?;
    let format = WaveFormat::new(32, 32, &SampleType::Float, 48000, 2, None);
    let mut client = AudioClient::new_application_loopback_client(pid, true)?;
    let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
    client.initialize_client(&format, &Direction::Capture, &mode)?;
    let event = client.set_get_eventhandle()?;
    let capture = client.get_audiocaptureclient()?;
    client.start_stream()?;
    let mut bytes: VecDeque<u8> = VecDeque::new();
    while !stop.load(Ordering::Relaxed) {
        let _ = event.wait_for_event(200);
        if capture.get_next_packet_size()?.unwrap_or(0) > 0 {
            capture.read_from_device_to_deque(&mut bytes)?;
        }
        let whole = bytes.len() / 4 * 4;
        if whole == 0 {
            continue;
        }
        let pcm: Vec<u8> = bytes
            .drain(..whole)
            .collect::<Vec<u8>>()
            .chunks_exact(4)
            .flat_map(|c| {
                let v = f32::from_le_bytes([c[0], c[1], c[2], c[3]]).clamp(-1.0, 1.0);
                ((v * 32767.0) as i16).to_le_bytes()
            })
            .collect();
        if tx.send(pcm).is_err() {
            break;
        }
    }
    client.stop_stream()?;
    Ok(())
}

fn main() {
    let mut args = std::env::args().skip(1);
    let pid: u32 = args.next().expect("usage: record <pid> [seconds]").parse().unwrap();
    let seconds: u64 = args.next().map(|s| s.parse().unwrap()).unwrap_or(10);
    let monitor = Monitor::primary().expect("no primary monitor");
    let size = (monitor.width().unwrap(), monitor.height().unwrap());
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(seconds + 8));
        eprintln!("timed out waiting for frames");
        std::process::exit(2);
    });
    let (tx, rx) = channel();
    let stop = Arc::new(AtomicBool::new(false));
    let audio_stop = stop.clone();
    thread::spawn(move || {
        if let Err(e) = audio_loop(pid, tx, audio_stop) {
            eprintln!("audio failed: {e}");
        }
    });
    let flags = Flags { width: size.0, height: size.1, seconds, audio: rx, stop };
    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::Default,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        flags,
    );
    Record::start(settings).expect("record failed");
}

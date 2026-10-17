use std::time::{Duration, Instant};

use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::encoder::{
    AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};

const SECONDS: u64 = 5;

struct Capture {
    encoder: Option<VideoEncoder>,
    start: Instant,
    frames: u32,
}

impl GraphicsCaptureApiHandler for Capture {
    type Flags = (u32, u32);
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let (width, height) = ctx.flags;
        let encoder = VideoEncoder::new(
            VideoSettingsBuilder::new(width, height),
            AudioSettingsBuilder::default().disabled(true),
            ContainerSettingsBuilder::default(),
            "capture.mp4",
        )?;
        Ok(Self { encoder: Some(encoder), start: Instant::now(), frames: 0 })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        self.encoder.as_mut().unwrap().send_frame(frame)?;
        self.frames += 1;
        if self.start.elapsed() >= Duration::from_secs(SECONDS) {
            self.encoder.take().unwrap().finish()?;
            let secs = self.start.elapsed().as_secs_f64();
            println!("{} frames in {:.2} s, {:.1} fps", self.frames, secs, self.frames as f64 / secs);
            control.stop();
        }
        Ok(())
    }
}

fn main() {
    let monitor = Monitor::primary().expect("no primary monitor");
    let size = (monitor.width().unwrap(), monitor.height().unwrap());
    println!("monitor {}x{}", size.0, size.1);
    let settings = Settings::new(
        monitor,
        CursorCaptureSettings::Default,
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Default,
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        size,
    );
    Capture::start(settings).expect("capture failed");
}

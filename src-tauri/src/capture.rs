use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use windows::core::Interface;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, ID3D11VideoContext, ID3D11VideoDevice, ID3D11VideoProcessor,
    ID3D11VideoProcessorInputView, ID3D11VideoProcessorOutputView, D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE,
    D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_TEX2D_VPIV, D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC,
    D3D11_USAGE, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING, D3D11_VIDEO_COLOR, D3D11_VIDEO_COLOR_0, D3D11_VIDEO_COLOR_RGBA,
    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_COLOR_SPACE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM, D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, GraphicsCaptureItemType, MinimumUpdateIntervalSettings,
    SecondaryWindowSettings, Settings,
};

use crate::check::line;
use crate::record::{current_sink, Err, FPS, FRAME_GAP};
use crate::{debug, proc, Core};

const PROBE_WAIT: Duration = Duration::from_secs(2);
const POLL: Duration = Duration::from_millis(20);
const OUTPUT_COLORS: u32 = (1 << 2) | (1 << 4);

pub fn fit_rect((w, h): (u32, u32), (cw, ch): (u32, u32)) -> (u32, u32, u32, u32) {
    let scale = (cw as f64 / w as f64).min(ch as f64 / h as f64);
    let (dw, dh) = (((w as f64 * scale) as u32).clamp(2, cw) & !1, ((h as f64 * scale) as u32).clamp(2, ch) & !1);
    (((cw - dw) / 2) & !1, ((ch - dh) / 2) & !1, dw, dh)
}

fn luma(r: i32, g: i32, b: i32) -> u8 {
    (16 + ((47 * r + 157 * g + 16 * b + 128) >> 8)) as u8
}

fn chroma(r: i32, g: i32, b: i32) -> (u8, u8) {
    ((128 + ((-26 * r - 86 * g + 112 * b + 128) >> 8)) as u8, (128 + ((112 * r - 102 * g - 10 * b + 128) >> 8)) as u8)
}

pub fn nv12(src: &[u8], (w, h): (u32, u32), (ow, oh): (u32, u32), out: &mut Vec<u8>) {
    let (x0, y0, dw, dh) = fit_rect((w, h), (ow, oh));
    let (w, h, ow, oh) = (w as usize, h as usize, ow as usize, oh as usize);
    let (x0, y0, dw, dh) = (x0 as usize, y0 as usize, dw as usize, dh as usize);
    out.clear();
    out.resize(ow * oh * 3 / 2, 0);
    let rgb = |x: usize, y: usize| -> [i32; 3] {
        if x < x0 || y < y0 || x >= x0 + dw || y >= y0 + dh {
            return [0; 3];
        }
        let i = (((y - y0) * h / dh) * w + (x - x0) * w / dw) * 4;
        [src[i + 2] as i32, src[i + 1] as i32, src[i] as i32]
    };
    for y in (0..oh).step_by(2) {
        for x in (0..ow).step_by(2) {
            let mut sum = [0; 3];
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let [r, g, b] = rgb(x + dx, y + dy);
                out[(y + dy) * ow + x + dx] = luma(r, g, b);
                sum = [sum[0] + r, sum[1] + g, sum[2] + b];
            }
            let (u, v) = chroma(sum[0] / 4, sum[1] / 4, sum[2] / 4);
            let at = ow * oh + y / 2 * ow + x;
            out[at] = u;
            out[at + 1] = v;
        }
    }
}

struct Scaler {
    from: (u32, u32),
    to: (u32, u32),
    video: ID3D11VideoContext,
    processor: ID3D11VideoProcessor,
    input: ID3D11Texture2D,
    input_view: ID3D11VideoProcessorInputView,
    output: ID3D11Texture2D,
    output_view: ID3D11VideoProcessorOutputView,
    staging: ID3D11Texture2D,
}

fn texture(device: &ID3D11Device, (w, h): (u32, u32), format: DXGI_FORMAT, usage: D3D11_USAGE, bind: u32, cpu: u32) -> windows::core::Result<ID3D11Texture2D> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: usage,
        BindFlags: bind,
        CPUAccessFlags: cpu,
        MiscFlags: 0,
    };
    let mut out = None;
    unsafe { device.CreateTexture2D(&desc, None, Some(&mut out)) }?;
    out.ok_or_else(windows::core::Error::empty)
}

impl Scaler {
    fn new(device: &ID3D11Device, context: &ID3D11DeviceContext, from: (u32, u32), to: (u32, u32)) -> windows::core::Result<Self> {
        let rate = DXGI_RATIONAL { Numerator: FPS, Denominator: 1 };
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: rate,
            InputWidth: from.0,
            InputHeight: from.1,
            OutputFrameRate: rate,
            OutputWidth: to.0,
            OutputHeight: to.1,
            Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
        };
        let device_video: ID3D11VideoDevice = device.cast()?;
        let video: ID3D11VideoContext = context.cast()?;
        unsafe {
            let enumerator = device_video.CreateVideoProcessorEnumerator(&desc)?;
            let processor = device_video.CreateVideoProcessor(&enumerator, 0)?;
            let input = texture(device, from, DXGI_FORMAT_B8G8R8A8_UNORM, D3D11_USAGE_DEFAULT, (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32, 0)?;
            let output = texture(device, to, DXGI_FORMAT_NV12, D3D11_USAGE_DEFAULT, D3D11_BIND_RENDER_TARGET.0 as u32, 0)?;
            let staging = texture(device, to, DXGI_FORMAT_NV12, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ.0 as u32)?;
            let input_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: 0 } },
            };
            let mut input_view = None;
            device_video.CreateVideoProcessorInputView(&input, &enumerator, &input_desc, Some(&mut input_view))?;
            let output_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
            };
            let mut output_view = None;
            device_video.CreateVideoProcessorOutputView(&output, &enumerator, &output_desc, Some(&mut output_view))?;
            let (x, y, w, h) = fit_rect(from, to);
            let whole = |(w, h): (u32, u32)| RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 };
            let dest = RECT { left: x as i32, top: y as i32, right: (x + w) as i32, bottom: (y + h) as i32 };
            video.VideoProcessorSetStreamFrameFormat(&processor, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
            video.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            video.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&whole(from)));
            video.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&dest));
            video.VideoProcessorSetOutputTargetRect(&processor, true, Some(&whole(to)));
            video.VideoProcessorSetStreamColorSpace(&processor, 0, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 });
            video.VideoProcessorSetOutputColorSpace(&processor, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: OUTPUT_COLORS });
            let black = D3D11_VIDEO_COLOR { Anonymous: D3D11_VIDEO_COLOR_0 { RGBA: D3D11_VIDEO_COLOR_RGBA { R: 0.0, G: 0.0, B: 0.0, A: 1.0 } } };
            video.VideoProcessorSetOutputBackgroundColor(&processor, false, &black);
            Ok(Self {
                from,
                to,
                video,
                processor,
                input,
                input_view: input_view.ok_or_else(windows::core::Error::empty)?,
                output,
                output_view: output_view.ok_or_else(windows::core::Error::empty)?,
                staging,
            })
        }
    }

    fn scale(&self, context: &ID3D11DeviceContext, frame: &ID3D11Texture2D, out: &mut Vec<u8>) -> windows::core::Result<()> {
        let (w, h) = (self.to.0 as usize, self.to.1 as usize);
        unsafe {
            context.CopyResource(&self.input, frame);
            let stream = D3D11_VIDEO_PROCESSOR_STREAM { Enable: true.into(), pInputSurface: ManuallyDrop::new(Some(self.input_view.clone())), ..Default::default() };
            let streams = [stream];
            let blt = self.video.VideoProcessorBlt(&self.processor, &self.output_view, 0, &streams);
            let [mut stream] = streams;
            ManuallyDrop::drop(&mut stream.pInputSurface);
            blt?;
            context.CopyResource(&self.staging, &self.output);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(&self.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;
            let pitch = mapped.RowPitch as usize;
            if pitch < w || (mapped.DepthPitch as usize) < pitch * h * 3 / 2 {
                context.Unmap(&self.staging, 0);
                return Err(windows::core::Error::empty());
            }
            let data = std::slice::from_raw_parts(mapped.pData as *const u8, pitch * h * 3 / 2);
            out.clear();
            for row in 0..h + h / 2 {
                out.extend_from_slice(&data[row * pitch..row * pitch + w]);
            }
            context.Unmap(&self.staging, 0);
        }
        Ok(())
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

pub struct Capture {
    core: Arc<Core>,
    raw: Vec<u8>,
    scaler: Option<Scaler>,
    gpu: bool,
    last: Instant,
}

impl Capture {
    fn convert(&mut self, frame: &mut Frame, out: (u32, u32), into: &mut Vec<u8>) -> Result<(), Err> {
        let from = (frame.width(), frame.height());
        if self.gpu {
            if self.scaler.as_ref().is_none_or(|s| s.from != from || s.to != out) {
                self.scaler = None;
                match Scaler::new(frame.device(), frame.device_context(), from, out) {
                    Ok(s) => self.scaler = Some(s),
                    Err(e) => {
                        tracing::warn!("scaling on the GPU is not available ({e}), scaling on the CPU");
                        self.gpu = false;
                    }
                }
            }
            if let Some(scaler) = &self.scaler {
                match scaler.scale(frame.device_context(), frame.as_raw_texture(), into) {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        tracing::warn!("scaling on the GPU failed ({e}), scaling on the CPU");
                        (self.gpu, self.scaler) = (false, None);
                    }
                }
            }
        }
        let buffer = frame.buffer()?;
        let data = buffer.as_nopadding_buffer(&mut self.raw);
        nv12(data, from, out, into);
        Ok(())
    }
}

impl GraphicsCaptureApiHandler for Capture {
    type Flags = Arc<Core>;
    type Error = Err;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        Ok(Self { core: ctx.flags, raw: Vec::new(), scaler: None, gpu: true, last: Instant::now() - FRAME_GAP })
    }

    fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), Self::Error> {
        let (w, h) = (frame.width(), frame.height());
        debug::frame(debug::ARRIVED);
        if self.last.elapsed() < FRAME_GAP || w < 2 || h < 2 {
            return Ok(());
        }
        debug::frame(debug::TAKEN);
        self.last = Instant::now();
        let core = self.core.clone();
        let e = &core.engine;
        *e.fresh.lock().unwrap() = Instant::now();
        let recording = core.active_set.lock().unwrap().is_some();
        let canvas = {
            let mut canvas = e.canvas.lock().unwrap();
            if !recording || canvas.is_none() {
                *canvas = Some((w & !1, h & !1));
            }
            canvas.unwrap()
        };
        let Some(sink) = current_sink(&core, Some(canvas)) else { return Ok(()) };
        let mut picture = Vec::new();
        self.convert(frame, sink.out(), &mut picture)?;
        let picture = Arc::new(picture);
        sink.write_video(picture.clone());
        *e.last.lock().unwrap() = (picture, sink.out(), Instant::now());
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

pub fn start_capture<H, T>(item: T, flags: H::Flags) -> Started<H>
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

pub fn check_capture() -> Vec<Value> {
    let mut lines = Vec::new();
    let got = Arc::new(AtomicBool::new(false));
    let Ok(primary) = Monitor::primary() else {
        return vec![line("fail", "No display found", json!({}))];
    };
    let (refused, control) = start_capture::<Probe, _>(primary, got.clone());
    if let Some(e) = refused {
        lines.push(line("warn", "Capture without a border was refused ({error}); falling back to the default border", json!({"error": e})));
    }
    lines.push(match control {
        Err(e) => line("fail", "Screen capture could not start: {error}", json!({"error": e})),
        Ok(control) => {
            proc::wait_until(PROBE_WAIT, POLL, || got.load(Ordering::Relaxed));
            let _ = control.stop();
            if got.load(Ordering::Relaxed) {
                line("ok", "Screen capture started and a frame arrived", json!({}))
            } else {
                line("warn", "Screen capture started but no frame arrived in {seconds} s (idle or locked screen)", json!({"seconds": PROBE_WAIT.as_secs()}))
            }
        }
    });
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_fits_inside_with_even_bars() {
        assert_eq!(fit_rect((2560, 1440), (1280, 720)), (0, 0, 1280, 720));
        assert_eq!(fit_rect((1920, 1200), (1280, 720)), (64, 0, 1152, 720));
        assert_eq!(fit_rect((3440, 1440), (1280, 720)), (0, 92, 1280, 534));
    }

    #[test]
    fn colors_become_studio_range_bt709() {
        let (w, h) = (4u32, 2u32);
        let mut src = Vec::new();
        for i in 0..w * h {
            src.extend_from_slice(if i % 4 < 2 { &[255, 255, 255, 255] } else { &[0, 0, 255, 255] });
        }
        let mut out = Vec::new();
        nv12(&src, (w, h), (w, h), &mut out);
        assert_eq!(out.len(), 12);
        assert_eq!(&out[..2], &[235, 235]);
        assert_eq!(out[2], luma(255, 0, 0));
        assert_eq!(&out[8..10], &[128, 128]);
        assert_eq!(out[10], chroma(255, 0, 0).0);
        assert!(out[11] > 230);
        let mut black = Vec::new();
        nv12(&[0, 0, 0, 255].repeat(4), (2, 2), (2, 2), &mut black);
        assert_eq!(black, [16, 16, 16, 16, 128, 128]);
    }

    #[test]
    fn a_narrow_picture_gets_black_bars_on_the_sides() {
        let mut out = Vec::new();
        nv12(&[255u8; 2 * 4 * 4], (2, 4), (8, 4), &mut out);
        assert_eq!(&out[..8], &[16, 16, 235, 235, 16, 16, 16, 16]);
    }
}

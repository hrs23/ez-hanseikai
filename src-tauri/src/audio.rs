use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use serde_json::{json, Value};
use wasapi::{initialize_mta, AudioCaptureClient, AudioClient, DeviceEnumerator, Direction, Handle, SampleType, StreamMode, WaveFormat};

use crate::record::{Err, APP, MIC, RESCAN_EVERY};
use crate::Core;

pub const SAMPLE_RATE: usize = 48000;
pub const CHANNELS: usize = 2;
const FLOAT_FRAME_BYTES: usize = 8;
const WAIT_MS: u32 = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loopback {
    Include(u32),
    ExcludeSelf,
}

impl Loopback {
    const SELF: &'static str = "self";

    pub fn key(self) -> String {
        match self {
            Loopback::Include(pid) => pid.to_string(),
            Loopback::ExcludeSelf => Self::SELF.into(),
        }
    }

    fn from_key(key: &str) -> Option<Self> {
        match key {
            Self::SELF => Some(Loopback::ExcludeSelf),
            _ => key.parse().ok().filter(|pid| *pid != 0).map(Loopback::Include),
        }
    }

    pub fn client(self) -> Result<AudioClient, Err> {
        let (pid, include) = match self {
            Loopback::Include(pid) => (pid, true),
            Loopback::ExcludeSelf => (std::process::id(), false),
        };
        Ok(AudioClient::new_application_loopback_client(pid, include)?)
    }
}

pub struct Audio {
    key: String,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

struct Opened {
    client: AudioClient,
    event: Handle,
    capture: AudioCaptureClient,
}

pub fn capture_format() -> (WaveFormat, StreamMode) {
    (WaveFormat::new(32, 32, &SampleType::Float, SAMPLE_RATE, CHANNELS, None), StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 })
}

fn open(core: &Core, track: usize, key: &str) -> Result<Option<Opened>, Err> {
    let (format, mode) = capture_format();
    let mut found = None;
    let mut client = if track == APP {
        let Some(loopback) = Loopback::from_key(key) else { return Ok(None) };
        loopback.client()?
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

fn read(opened: &Opened, bytes: &mut VecDeque<u8>) -> Result<(), Err> {
    let _ = opened.event.wait_for_event(WAIT_MS);
    while opened.capture.get_next_packet_size()?.unwrap_or(0) > 0 {
        opened.capture.read_from_device_to_deque(bytes)?;
    }
    Ok(())
}

fn run(core: &Core, track: usize, key: &str, stop: &AtomicBool) {
    if let Err(err) = initialize_mta().ok() {
        tracing::warn!("audio {track} failed: {err}");
        return;
    }
    let e = &core.engine;
    let mut source: Option<Opened> = None;
    let mut tried: Option<Instant> = None;
    let mut bytes: VecDeque<u8> = VecDeque::new();
    let mut raw: Vec<u8> = Vec::new();
    let mut pcm: Vec<u8> = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        if source.is_none() && tried.is_none_or(|t| t.elapsed() > RESCAN_EVERY) {
            tried = Some(Instant::now());
            source = open(core, track, key).unwrap_or_else(|err| {
                tracing::debug!("audio source {track} unavailable: {err}");
                None
            });
        }
        match &source {
            Some(opened) => {
                if read(opened, &mut bytes).is_err() {
                    source = None;
                    bytes.clear();
                }
            }
            None => thread::sleep(std::time::Duration::from_millis(WAIT_MS as u64)),
        }
        if source.is_none() && track == MIC {
            let mut names = e.names.lock().unwrap();
            names.mic_id.clear();
            names.mic.clear();
        }
        let whole = bytes.len() / FLOAT_FRAME_BYTES * FLOAT_FRAME_BYTES;
        let (volume, muted) = e.gain(track);
        let volume = volume as f32;
        let mut peak = 0f32;
        raw.clear();
        raw.extend(bytes.drain(..whole));
        pcm.clear();
        for c in raw.chunks_exact(4) {
            let v = (f32::from_le_bytes([c[0], c[1], c[2], c[3]]) * volume).clamp(-1.0, 1.0);
            peak = peak.max(v.abs());
            pcm.extend_from_slice(&(if muted { 0i16 } else { (v * i16::MAX as f32) as i16 }).to_le_bytes());
        }
        e.peaks[track].fetch_max(peak.to_bits(), Ordering::Relaxed);
        let sink = e.sink.lock().unwrap().clone();
        if let Some(sink) = sink {
            sink.write_audio(track, &pcm);
        }
    }
    if let Some(opened) = &source {
        let _ = opened.client.stop_stream();
    }
}

pub fn ensure(core: &Arc<Core>, track: usize, key: String) {
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
    let thread = thread::spawn(move || run(&c, track, &k, &s));
    *slot = Some(Audio { key, stop, thread });
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

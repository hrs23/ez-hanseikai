use std::collections::VecDeque;
use std::time::{Duration, Instant};

use wasapi::{initialize_mta, AudioClient, Direction, SampleType, StreamMode, WaveFormat};

const SECONDS: u64 = 4;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pid: u32 = std::env::args().nth(1).expect("usage: audio <pid>").parse()?;
    initialize_mta().ok()?;
    let format = WaveFormat::new(32, 32, &SampleType::Float, 48000, 2, None);
    let mut client = AudioClient::new_application_loopback_client(pid, true)?;
    let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 0 };
    client.initialize_client(&format, &Direction::Capture, &mode)?;
    let event = client.set_get_eventhandle()?;
    let capture = client.get_audiocaptureclient()?;
    client.start_stream()?;

    let start = Instant::now();
    let mut bytes: VecDeque<u8> = VecDeque::new();
    let mut samples = 0u64;
    let mut peak = 0f32;
    let mut sum = 0f64;
    while start.elapsed() < Duration::from_secs(SECONDS) {
        let _ = event.wait_for_event(200);
        if capture.get_next_packet_size()?.unwrap_or(0) > 0 {
            capture.read_from_device_to_deque(&mut bytes)?;
        }
        let whole = bytes.len() / 4 * 4;
        for chunk in bytes.drain(..whole).collect::<Vec<u8>>().chunks_exact(4) {
            let v = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            peak = peak.max(v.abs());
            sum += (v as f64) * (v as f64);
            samples += 1;
        }
    }
    client.stop_stream()?;
    let rms = if samples > 0 { (sum / samples as f64).sqrt() } else { 0.0 };
    println!("pid {pid}: {samples} samples, peak {peak:.4}, rms {rms:.4}");
    Ok(())
}

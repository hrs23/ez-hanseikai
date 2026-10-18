use std::sync::Arc;

use iroh::Watcher;
use serde_json::{json, Value};
use wasapi::{initialize_mta, AudioClient, DeviceEnumerator, Direction};

use crate::{audio, debug, ffmpeg, record, Core};

fn line(level: &str, text: impl Into<String>) -> Value {
    json!({"level": level, "text": text.into()})
}

fn windows_build() -> Option<u32> {
    let text = debug::command_line("cmd", &["/c", "ver"]);
    let version = text.split("Version").nth(1)?;
    version.trim().trim_end_matches(']').split('.').nth(2)?.trim().parse().ok()
}

fn audio_lines(pid: Option<u32>) -> Vec<Value> {
    let mut lines = Vec::new();
    let _ = initialize_mta();
    match DeviceEnumerator::new().and_then(|e| e.get_device_collection(&Direction::Render)) {
        Err(e) => lines.push(line("fail", format!("could not list playback devices: {e}"))),
        Ok(devices) => {
            let count = devices.get_nbr_devices().unwrap_or(0);
            if count == 0 {
                lines.push(line("warn", "no playback device: there is no game sound to capture on this PC"));
            } else {
                let names: Vec<String> = (0..count).filter_map(|i| devices.get_device_at_index(i).ok()?.get_friendlyname().ok()).collect();
                lines.push(line("ok", format!("{count} playback device(s): {}", names.join(", "))));
            }
        }
    }
    match pid {
        None => lines.push(line("info", "no game process selected, so game sound is not checked")),
        Some(pid) => {
            let (format, mode) = audio::capture_format();
            let result = AudioClient::new_application_loopback_client(pid, true)
                .and_then(|mut client| client.initialize_client(&format, &Direction::Capture, &mode));
            match result {
                Ok(()) => lines.push(line("ok", "the selected process's sound can be captured")),
                Err(e) => lines.push(line("fail", format!("the selected process's sound cannot be captured: {e}"))),
            }
        }
    }
    lines
}

pub fn run(core: &Arc<Core>, audio_pid: Option<u32>) -> Vec<Value> {
    let mut lines = Vec::new();
    match windows_build() {
        None => lines.push(line("warn", "could not read the Windows build")),
        Some(build) => {
            lines.push(line("info", format!("Windows build {build}")));
            if build < 19041 {
                lines.push(line("fail", "per-process game sound capture needs Windows build 19041 or newer; recording will have no game sound"));
            } else {
                lines.push(line("ok", "per-process sound capture is supported"));
            }
            if build < 22000 {
                lines.push(line("warn", "screen capture may show a yellow border before Windows 11"));
            }
        }
    }
    match tauri::webview_version() {
        Ok(version) => lines.push(line("ok", format!("WebView2 runtime {version}"))),
        Err(e) => lines.push(line("fail", format!("WebView2 runtime missing: {e}"))),
    }
    for (level, text) in record::check_capture() {
        lines.push(line(level, text));
    }
    lines.extend(audio_lines(audio_pid));
    if !ffmpeg::present() {
        lines.push(line("fail", "ffmpeg is not installed yet; it is downloaded on first run"));
    }
    for (name, ok) in ffmpeg::encoder_results() {
        lines.push(line(if ok { "ok" } else { "info" }, format!("encoder {name}: {}", if ok { "works" } else { "not available" })));
    }
    let usable = core.codecs.lock().unwrap().clone();
    let (encoder, codec, _) = ffmpeg::pick(&usable);
    lines.push(line("info", format!("recording with {encoder} ({codec})")));
    let relay = format!("{:?}", core.ep.home_relay_status().get());
    if relay.contains("Connected") {
        lines.push(line("ok", "a relay server is reachable"));
    } else {
        lines.push(line("warn", format!("no relay connection yet: {relay}")));
    }
    let report = format!("{:?}", core.ep.net_report().get());
    if report.contains("udp_v4: true") {
        lines.push(line("ok", "UDP works"));
    } else {
        lines.push(line("warn", "UDP may be blocked; connections will use the relay and may be slower"));
    }
    if report.contains("mapping_varies_by_dest_ipv4: Some(true)") {
        lines.push(line("warn", "this network changes the address per destination, so a direct connection may fail and the relay will be used"));
    }
    lines.push(line("info", "Windows may ask once to allow the app through the firewall; click Allow. If it is declined, connections can still work through the relay"));
    lines
}

pub fn text(lines: &[Value]) -> String {
    lines
        .iter()
        .map(|l| format!("[{}] {}", l["level"].as_str().unwrap_or(""), l["text"].as_str().unwrap_or("")))
        .collect::<Vec<_>>()
        .join("\n")
}

use std::sync::Arc;

use iroh::Watcher;
use serde_json::{json, Value};
use wasapi::{initialize_mta, AudioClient, DeviceEnumerator, Direction};

use crate::{audio, capture, debug, ffmpeg, Core};

pub(crate) fn line(level: &str, text: &str, args: Value) -> Value {
    json!({"level": level, "text": text, "args": args})
}

pub fn fill(text: &str, args: &Value) -> String {
    let mut out = text.to_string();
    for (key, value) in args.as_object().into_iter().flatten() {
        let value = value.as_str().map_or_else(|| value.to_string(), String::from);
        out = out.replace(&format!("{{{key}}}"), &value);
    }
    out
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
        Err(e) => lines.push(line("fail", "Could not list playback devices: {error}", json!({"error": e.to_string()}))),
        Ok(devices) => {
            let count = devices.get_nbr_devices().unwrap_or(0);
            lines.push(if count == 0 { line("warn", "No playback device: there is no app sound to capture on this PC", json!({})) } else { line("ok", "{count} playback device(s)", json!({"count": count})) });
        }
    }
    lines.push(match pid {
        None => line("info", "No app selected for app sound, so it is not checked", json!({})),
        Some(pid) => {
            let (format, mode) = audio::capture_format();
            match AudioClient::new_application_loopback_client(pid, true).and_then(|mut client| client.initialize_client(&format, &Direction::Capture, &mode)) {
                Ok(()) => line("ok", "The selected app's sound can be captured", json!({})),
                Err(e) => line("fail", "The selected app's sound cannot be captured: {error}", json!({"error": e.to_string()})),
            }
        }
    });
    lines
}

pub fn run(core: &Arc<Core>, audio_pid: Option<u32>) -> Vec<Value> {
    let mut lines = Vec::new();
    match windows_build() {
        None => lines.push(line("warn", "Could not read the Windows build", json!({}))),
        Some(build) => {
            lines.push(line("info", "Windows build {build}", json!({"build": build})));
            lines.push(if build < 19041 { line("fail", "Per-process app sound capture needs Windows build 19041 or newer; recording will have no app sound", json!({})) } else { line("ok", "Per-process sound capture is supported", json!({})) });
            if build < 22000 {
                lines.push(line("warn", "Screen capture may show a yellow border before Windows 11", json!({})));
            }
        }
    }
    lines.push(match tauri::webview_version() {
        Ok(version) => line("ok", "WebView2 runtime {version}", json!({"version": version})),
        Err(e) => line("fail", "WebView2 runtime missing: {error}", json!({"error": e.to_string()})),
    });
    lines.extend(capture::check_capture());
    lines.extend(audio_lines(audio_pid));
    if !ffmpeg::present() {
        lines.push(line("fail", "ffmpeg is not installed yet; it is downloaded on first run", json!({})));
    }
    for (name, ok) in ffmpeg::encoder_results() {
        lines.push(line(if ok { "ok" } else { "info" }, if ok { "Encoder {name} works" } else { "Encoder {name} is not available" }, json!({"name": name})));
    }
    let usable = core.codecs.lock().unwrap().clone();
    let (encoder, codec, _) = ffmpeg::pick(&usable);
    lines.push(line("info", "Recording with {encoder} ({codec})", json!({"encoder": encoder, "codec": codec})));
    lines.push(if format!("{:?}", core.ep.home_relay_status().get()).contains("Connected") { line("ok", "A relay server is reachable", json!({})) } else { line("warn", "No relay connection yet", json!({})) });
    let report = format!("{:?}", core.ep.net_report().get());
    lines.push(if report.contains("udp_v4: true") { line("ok", "UDP works", json!({})) } else { line("warn", "UDP may be blocked; connections will use the relay and may be slower", json!({})) });
    if report.contains("mapping_varies_by_dest_ipv4: Some(true)") {
        lines.push(line("warn", "This network changes the address per destination, so a direct connection may fail and the relay will be used", json!({})));
    }
    lines.push(line("info", "Windows may ask once to allow the app through the firewall; click Allow. If it is declined, connections can still work through the relay", json!({})));
    lines
}

pub fn text(lines: &[Value]) -> String {
    lines
        .iter()
        .map(|l| format!("[{}] {}", l["level"].as_str().unwrap_or(""), fill(l["text"].as_str().unwrap_or(""), &l["args"])))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_filled_from_their_arguments_and_carry_no_device_names() {
        let l = line("ok", "{count} playback device(s)", json!({"count": 2}));
        assert_eq!(text(&[l]), "[ok] 2 playback device(s)");
        assert_eq!(fill("Windows build {build}", &json!({"build": 22631})), "Windows build 22631");
        assert_eq!(fill("no arguments", &Value::Null), "no arguments");
    }
}

use std::sync::Arc;

use iroh::Watcher;
use wasapi::{initialize_mta, DeviceEnumerator, Direction};

use crate::{audio, capture, debug, ffmpeg, sources, Core};

pub(crate) fn line(level: &str, text: impl std::fmt::Display) -> String {
    format!("[{level}] {text}")
}

fn windows_build() -> Option<u32> {
    let text = debug::command_line("cmd", &["/c", "ver"]);
    let version = text.split("Version").nth(1)?;
    version.trim().trim_end_matches(']').split('.').nth(2)?.trim().parse().ok()
}

fn audio_lines(core: &Core) -> Vec<String> {
    let mut lines = Vec::new();
    let _ = initialize_mta();
    match DeviceEnumerator::new().and_then(|e| e.get_device_collection(&Direction::Render)) {
        Err(e) => lines.push(line("fail", format!("Could not list playback devices: {e}"))),
        Ok(devices) => {
            let count = devices.get_nbr_devices().unwrap_or(0);
            lines.push(if count == 0 { line("warn", "No playback device: there is no app sound to capture on this PC") } else { line("ok", format!("{count} playback device(s)")) });
        }
    }
    let source = core.engine.source.lock().unwrap().clone();
    lines.push(match sources::find(&source, None) {
        None => line("info", "No screen is shared, so its sound is not checked"),
        Some(found) => {
            let (format, mode) = audio::capture_format();
            match found.audio.client().and_then(|mut client| Ok(client.initialize_client(&format, &Direction::Capture, &mode)?)) {
                Ok(()) => line("ok", "The shared screen's sound can be captured"),
                Err(e) => line("fail", format!("The shared screen's sound cannot be captured: {e}")),
            }
        }
    });
    lines
}

pub fn run(core: &Arc<Core>) -> Vec<String> {
    let mut lines = Vec::new();
    match windows_build() {
        None => lines.push(line("warn", "Could not read the Windows build")),
        Some(build) => {
            lines.push(line("info", format!("Windows build {build}")));
            lines.push(if build < 19041 { line("fail", "Per-process app sound capture needs Windows build 19041 or newer; recording will have no app sound") } else { line("ok", "Per-process sound capture is supported") });
            if build < 22000 {
                lines.push(line("warn", "Screen capture may show a yellow border before Windows 11"));
            }
        }
    }
    lines.push(match tauri::webview_version() {
        Ok(version) => line("ok", format!("WebView2 runtime {version}")),
        Err(e) => line("fail", format!("WebView2 runtime missing: {e}")),
    });
    lines.extend(capture::check_capture());
    lines.extend(audio_lines(core));
    if !ffmpeg::present() {
        lines.push(line("fail", "ffmpeg is not installed yet; it is downloaded on first run"));
    }
    for (name, ok) in ffmpeg::encoder_results() {
        lines.push(line(if ok { "ok" } else { "info" }, if ok { format!("Encoder {name} works") } else { format!("Encoder {name} is not available") }));
    }
    let usable = core.codecs.lock().unwrap().clone();
    let (encoder, codec, _) = ffmpeg::pick(&usable);
    lines.push(line("info", format!("Recording with {encoder} ({codec})")));
    lines.push(if crate::relay::home(core).is_some() { line("ok", "A relay server is reachable") } else { line("warn", "No relay connection yet") });
    let report = core.ep.net_report().get();
    lines.push(if report.as_ref().is_some_and(|r| r.udp_v4) { line("ok", "UDP works") } else { line("warn", "UDP may be blocked") });
    if report.is_some_and(|r| r.mapping_varies_by_dest_ipv4 == Some(true)) {
        lines.push(line("warn", "This network changes the address per destination"));
    }
    lines.push(line("info", "Windows may ask once to allow the app through the firewall; click Allow"));
    lines
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_starts_with_its_level() {
        assert_eq!(line("ok", format!("{} playback device(s)", 2)), "[ok] 2 playback device(s)");
    }
}

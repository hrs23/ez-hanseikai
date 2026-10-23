use std::fmt;
use std::sync::Arc;

use iroh::Watcher;
use serde_json::{json, Map, Value};
use wasapi::{initialize_mta, DeviceEnumerator, Direction};

use crate::{audio, capture, debug, ffmpeg, sources, Core};

/// One check result: a level (ok, warn, fail, info) and an English text whose
/// `{name}` places are filled from `args`, so the window can translate it.
pub(crate) struct Line {
    level: &'static str,
    text: &'static str,
    args: Map<String, Value>,
}

pub(crate) fn line(level: &'static str, text: &'static str) -> Line {
    Line { level, text, args: Map::new() }
}

impl Line {
    pub(crate) fn with(mut self, key: &str, value: impl ToString) -> Self {
        self.args.insert(key.into(), Value::String(value.to_string()));
        self
    }

    fn json(&self) -> Value {
        json!({"level": self.level, "text": self.text, "args": self.args})
    }
}

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut text = self.text.to_string();
        for (key, value) in &self.args {
            text = text.replace(&format!("{{{key}}}"), value.as_str().unwrap_or_default());
        }
        write!(f, "[{}] {text}", self.level)
    }
}

/// One item of the checklist: a fixed name and its lines. The first line says
/// what was found; the item is as bad as its worst line.
pub(crate) struct Item {
    name: &'static str,
    lines: Vec<Line>,
}

const SEVERITY: [&str; 4] = ["info", "ok", "warn", "fail"];

impl Item {
    fn level(&self) -> &'static str {
        let worst = self.lines.iter().map(|l| SEVERITY.iter().position(|s| *s == l.level).unwrap_or(0)).max().unwrap_or(0);
        if worst <= 1 { "ok" } else { SEVERITY[worst] }
    }

    fn json(&self) -> Value {
        json!({"item": self.name, "level": self.level(), "lines": self.lines.iter().map(Line::json).collect::<Vec<_>>()})
    }
}

fn item(name: &'static str, lines: Vec<Line>) -> Item {
    Item { name, lines }
}

fn windows_build() -> Option<u32> {
    let text = debug::command_line("cmd", &["/c", "ver"]);
    let version = text.split("Version").nth(1)?;
    version.trim().trim_end_matches(']').split('.').nth(2)?.trim().parse().ok()
}

fn windows() -> Item {
    let mut lines = Vec::new();
    match windows_build() {
        None => lines.push(line("warn", "Could not read the Windows build")),
        Some(build) => {
            lines.push(line("info", "Windows build {build}").with("build", build));
            if build < 19041 {
                lines.push(line("fail", "Per-process app sound capture needs Windows build 19041 or newer; recording will have no app sound"));
            }
            if build < 22000 {
                lines.push(line("warn", "Screen capture may show a yellow border before Windows 11"));
            }
        }
    }
    item("Windows version", lines)
}

fn app_sound(core: &Core) -> Item {
    let mut lines = Vec::new();
    let _ = initialize_mta();
    match DeviceEnumerator::new().and_then(|e| e.get_device_collection(&Direction::Render)) {
        Err(e) => lines.push(line("fail", "Could not list playback devices: {error}").with("error", e)),
        Ok(devices) => {
            let count = devices.get_nbr_devices().unwrap_or(0);
            lines.push(if count == 0 { line("warn", "No playback device: there is no app sound to capture on this PC") } else { line("ok", "{count} playback device(s)").with("count", count) });
        }
    }
    let source = core.engine.source.lock().unwrap().clone();
    lines.push(match sources::find(&source, None) {
        None => line("info", "No screen is shared, so its sound is not checked"),
        Some(found) => {
            let (format, mode) = audio::capture_format();
            match found.audio.client().and_then(|mut client| Ok(client.initialize_client(&format, &Direction::Capture, &mode)?)) {
                Ok(()) => line("ok", "The shared screen's sound can be captured"),
                Err(e) => line("fail", "The shared screen's sound cannot be captured: {error}").with("error", e),
            }
        }
    });
    item("App sound", lines)
}

fn recording(core: &Core) -> Item {
    let usable = core.codecs.lock().unwrap().clone();
    let (encoder, codec, _) = ffmpeg::pick(&usable);
    let mut lines = vec![if ffmpeg::present() { line("ok", "Recording with {encoder} ({codec})").with("encoder", encoder).with("codec", codec) } else { line("fail", "ffmpeg is not installed yet; it is downloaded on first run") }];
    for (name, ok) in ffmpeg::encoder_results() {
        lines.push(if ok { line("info", "Encoder {name} works") } else { line("info", "Encoder {name} is not available") }.with("name", name));
    }
    item("Encoder", lines)
}

/// This PC: Windows, WebView2, screen and sound capture, and recording.
pub(crate) fn machine(core: &Arc<Core>) -> Vec<Item> {
    vec![
        windows(),
        item("WebView2", vec![match tauri::webview_version() {
            Ok(version) => line("ok", "WebView2 runtime {version}").with("version", version),
            Err(e) => line("fail", "WebView2 runtime missing: {error}").with("error", e),
        }]),
        item("Screen capture", capture::check_capture()),
        app_sound(core),
        recording(core),
    ]
}

/// The connection: the relay server and what the network allows.
pub(crate) fn network(core: &Arc<Core>) -> Vec<Item> {
    let report = core.ep.net_report().get();
    let varies = report.as_ref().is_some_and(|r| r.mapping_varies_by_dest_ipv4 == Some(true));
    vec![
        item("Relay server", vec![if crate::relay::home(core).is_some() { line("ok", "A relay server is reachable") } else { line("warn", "No relay connection yet") }]),
        item("UDP", vec![if report.as_ref().is_some_and(|r| r.udp_v4) { line("ok", "UDP works") } else { line("warn", "UDP may be blocked") }]),
        item("Direct connections", vec![
            if varies { line("warn", "This network changes the address per destination") } else { line("ok", "This network allows direct connections") },
            line("info", "Windows may ask once to allow the app through the firewall; click Allow"),
        ]),
    ]
}

/// Both groups as the window and the `check` message carry them.
pub fn json(core: &Arc<Core>) -> Value {
    let list = |items: Vec<Item>| items.iter().map(Item::json).collect::<Vec<_>>();
    json!({"pc": list(machine(core)), "net": list(network(core))})
}

/// Both groups as plain lines for the report.
pub fn run(core: &Arc<Core>) -> Vec<String> {
    machine(core).into_iter().chain(network(core)).map(|i| format!("{} [{}]\n{}", i.name, i.level(), i.lines.iter().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n"))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_starts_with_its_level_and_fills_its_arguments() {
        assert_eq!(line("ok", "{count} playback device(s)").with("count", 2).to_string(), "[ok] 2 playback device(s)");
        assert_eq!(line("info", "Recording with {encoder} ({codec})").with("encoder", "x").with("codec", "h264").json(), json!({"level": "info", "text": "Recording with {encoder} ({codec})", "args": {"encoder": "x", "codec": "h264"}}));
    }

    #[test]
    fn an_item_is_as_bad_as_its_worst_line_and_ok_when_it_only_informs() {
        assert_eq!(item("Windows", vec![line("info", "Windows build {build}")]).level(), "ok");
        assert_eq!(item("Windows", vec![line("info", "a"), line("ok", "b")]).level(), "ok");
        assert_eq!(item("Windows", vec![line("ok", "a"), line("warn", "b")]).level(), "warn");
        assert_eq!(item("Windows", vec![line("warn", "a"), line("fail", "b"), line("ok", "c")]).level(), "fail");
        assert_eq!(item("UDP", vec![line("ok", "UDP works")]).json()["item"], "UDP");
    }
}

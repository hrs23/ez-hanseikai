use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::ops::arg;
use crate::{debug, proc, Core, VERSION};

pub const URL: &str = "https://ezh.hiros.page:8444/v1/report";

pub fn url() -> Option<&'static str> {
    Some(URL)
}

pub fn host() -> Option<&'static str> {
    url().and_then(|u| u.split('/').nth(2))
}

pub const KINDS: [&str; 3] = ["bug", "idea", "comment"];
pub const COMMENT_MAX: usize = 4000;
const DIAG_MAX: usize = 600_000;
const IMAGE_MAX: usize = 3_000_000;
const IMAGE_WIDTH_MAX: u32 = 1600;
const SEND_WAIT: Duration = Duration::from_secs(60);
const SEND: &str = r#"
[Console]::OutputEncoding = [Text.Encoding]::UTF8
$ErrorActionPreference = 'Stop'
trap { [Console]::Error.WriteLine($_.Exception.Message); exit 1 }
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor 3072
$bytes = [IO.File]::ReadAllBytes($env:EZH_BODY)
$request = [Net.HttpWebRequest]::Create($env:EZH_URL)
$request.Method = 'POST'
$request.ContentType = 'application/json; charset=utf-8'
$request.Timeout = 30000
$request.ReadWriteTimeout = 30000
$request.ContentLength = $bytes.Length
$stream = $request.GetRequestStream()
try { $stream.Write($bytes, 0, $bytes.Length) } finally { $stream.Dispose() }
$response = $request.GetResponse()
try { [Console]::Error.Write((New-Object IO.StreamReader($response.GetResponseStream())).ReadToEnd()) } finally { $response.Dispose() }
"#;

/// What the dialog showed and Send sends: the diagnostics, the window picture and the gzipped log.
static SHOWN: Mutex<(String, Option<Vec<u8>>, Vec<u8>)> = Mutex::new((String::new(), None, Vec::new()));

pub fn feedback_snap(core: &Arc<Core>) -> Value {
    let image = snap(core).ok();
    let picture = image.as_ref().map(|png| format!("data:image/png;base64,{}", base64(png)));
    *SHOWN.lock().unwrap() = (String::new(), image, Vec::new());
    json!(picture)
}

pub fn feedback_preview(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    let diagnostics = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| debug::diagnostics(core, false))) {
        Ok(Ok(text)) => fit(&text, DIAG_MAX),
        Ok(Err(err)) => format!("diagnostics failed: {err}"),
        Err(_) => "diagnostics failed".to_string(),
    };
    let log = debug::report_log(core);
    let log_bytes = log.len();
    let mut shown = SHOWN.lock().unwrap();
    (shown.0, shown.2) = (diagnostics.clone(), log);
    Ok(json!({"diagnostics": diagnostics, "log_bytes": log_bytes, "to": host()}))
}

/// The text cut to `max` characters by dropping the middle, so the summary at the top and the newest log at the end both stay.
pub fn fit(text: &str, max: usize) -> String {
    const CUT: &str = "\n\n... (cut to fit) ...\n\n";
    let n = text.chars().count();
    if n <= max {
        return text.to_string();
    }
    let head = max / 4;
    let tail = max.saturating_sub(head + CUT.chars().count());
    let mut out: String = text.chars().take(head).collect();
    out.push_str(CUT);
    out.extend(text.chars().skip(n - tail));
    out
}

/// The report as sent. The gzipped log goes with the diagnostics.
pub fn body(kind: &str, comment: &str, diagnostics: Option<(&str, &[u8])>, image: Option<&[u8]>, lang: &str) -> Result<Value, String> {
    if !KINDS.contains(&kind) {
        return Err("kind is missing or not valid.".into());
    }
    let comment = crate::clean_note(comment, COMMENT_MAX);
    if comment.trim().is_empty() && diagnostics.is_none() {
        return Err("Write a comment or include the diagnostics.".into());
    }
    if image.is_some_and(|png| png.len() > IMAGE_MAX) {
        return Err("The picture is too large.".into());
    }
    Ok(json!({
        "kind": kind,
        "comment": comment,
        "diagnostics": diagnostics.map(|d| d.0).unwrap_or_default(),
        "log": diagnostics.map(|d| d.1).filter(|log| !log.is_empty()).map(base64),
        "image": image.map(base64),
        "version": VERSION,
        "os": std::env::consts::OS,
        "lang": lang,
    }))
}

pub fn send_feedback(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let url = url().ok_or("Feedback is not set up in this build.")?;
    let (kind, comment): (String, String) = (arg(&args, "kind")?, arg(&args, "comment")?);
    let (with_diag, with_image): (bool, bool) = (arg(&args, "diagnostics")?, arg(&args, "image")?);
    let (diagnostics, image, log) = SHOWN.lock().unwrap().clone();
    let lang = crate::language(core);
    let body = body(&kind, &comment, with_diag.then_some((diagnostics.as_str(), log.as_slice())), if with_image { image.as_deref() } else { None }, &lang)?;
    let path = core.dir.join("feedback.json");
    std::fs::write(&path, body.to_string()).map_err(|e| e.to_string())?;
    let mut command = Command::new("powershell");
    command.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", SEND]).env("EZH_URL", url).env("EZH_BODY", &path);
    let sent = proc::run(&mut command, 0, SEND_WAIT);
    let _ = std::fs::remove_file(&path);
    match sent {
        Ok(reply) => {
            // The script writes the server's reply to stderr, the one stream proc::run keeps.
            let id = reply_id(&reply);
            tracing::info!("feedback sent as {} ({kind}, diagnostics {with_diag}, picture {with_image})", id.as_deref().unwrap_or("?"));
            *SHOWN.lock().unwrap() = (String::new(), None, Vec::new());
            Ok(json!({"id": id}))
        }
        Err(e) => {
            tracing::warn!("feedback not sent: {}", e.lines().last().unwrap_or_default());
            Err(send_error(&e).into())
        }
    }
}

/// The report's ID from the server's reply: the time it arrived, like 20261023-232413 (or 20261023-232413-2).
fn reply_id(reply: &str) -> Option<String> {
    let id = serde_json::from_str::<Value>(reply.trim()).ok()?["id"].as_str()?.to_string();
    let parts: Vec<&str> = id.split('-').collect();
    let digits = |p: &str, n: usize| p.len() == n && p.bytes().all(|b| b.is_ascii_digit());
    let ok = matches!(parts.as_slice(), [d, t] if digits(d, 8) && digits(t, 6)) || matches!(parts.as_slice(), [d, t, n] if digits(d, 8) && digits(t, 6) && !n.is_empty() && n.len() <= 3 && digits(n, n.len()));
    ok.then_some(id)
}

pub fn send_error(e: &str) -> &'static str {
    if e.contains("(429)") {
        "Too many reports from this PC. Try again later."
    } else if e.contains("(400)") || e.contains("(413)") {
        "The report server did not accept this report."
    } else {
        "Couldn't send. Check your connection."
    }
}

pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (*b as u32) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() { ABC[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

pub fn rgb(bgra: &[u8], (w, h): (u32, u32), step: u32) -> (Vec<u8>, (u32, u32)) {
    let (ow, oh) = (w.div_ceil(step), h.div_ceil(step));
    let mut out = Vec::with_capacity((ow * oh * 3) as usize);
    for y in (0..h).step_by(step as usize) {
        for x in (0..w).step_by(step as usize) {
            let i = ((y * w + x) * 4) as usize;
            out.extend_from_slice(&[bgra[i + 2], bgra[i + 1], bgra[i]]);
        }
    }
    (out, (ow, oh))
}

pub fn png(rgb: &[u8], (w, h): (u32, u32)) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(rgb).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[cfg(windows)]
fn snap(core: &Core) -> Result<Vec<u8>, String> {
    use ::windows::Win32::Foundation::{HWND, RECT};
    use ::windows::Win32::Graphics::Gdi::{CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS};
    use ::windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS, PW_CLIENTONLY};
    use ::windows::Win32::UI::WindowsAndMessaging::{GetClientRect, PW_RENDERFULLCONTENT};
    use tauri::Manager;
    let window = core.app.get_webview_window("main").ok_or("no window")?;
    let hwnd = HWND(window.hwnd().map_err(|e| e.to_string())?.0);
    let mut rect = RECT::default();
    unsafe { GetClientRect(hwnd, &mut rect) }.map_err(|e| e.to_string())?;
    let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
    if w <= 0 || h <= 0 {
        return Err("The window is hidden.".into());
    }
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    let lines = unsafe {
        let screen = GetDC(Some(hwnd));
        let dc = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bitmap.into());
        let printed = PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(PW_CLIENTONLY.0 | PW_RENDERFULLCONTENT)).as_bool();
        SelectObject(dc, old);
        let mut info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let lines = if printed { GetDIBits(dc, bitmap, 0, h as u32, Some(pixels.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS) } else { 0 };
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        ReleaseDC(Some(hwnd), screen);
        lines
    };
    if lines <= 0 {
        return Err("The window could not be captured.".into());
    }
    let (w, h) = (w as u32, h as u32);
    let (rgb, size) = rgb(&pixels, (w, h), w.div_ceil(IMAGE_WIDTH_MAX));
    let out = png(&rgb, size)?;
    if out.len() > IMAGE_MAX { Err("The picture is too large.".into()) } else { Ok(out) }
}

#[cfg(not(windows))]
fn snap(_: &Core) -> Result<Vec<u8>, String> {
    Err("Pictures of the window are taken on Windows only.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_report_loses_its_middle_and_keeps_the_top_and_the_newest_log() {
        assert_eq!(fit("short", 10), "short");
        let text = format!("TOP{}END", "x".repeat(1000));
        let out = fit(&text, 200);
        assert_eq!(out.chars().count(), 200);
        assert!(out.starts_with("TOP") && out.ends_with("END") && out.contains("cut to fit"));
        assert_eq!(fit(&"あ".repeat(300), 100).chars().count(), 100);
    }

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        assert_eq!([base64(b""), base64(b"f"), base64(b"fo"), base64(b"foo"), base64(b"foobar"), base64(&[0xff, 0xfe])], ["", "Zg==", "Zm8=", "Zm9v", "Zm9vYmFy", "//4="]);
    }

    #[test]
    fn a_report_keeps_only_a_known_kind_a_short_comment_and_what_was_ticked() {
        assert!(body("spam", "hi", None, None, "en").is_err());
        assert!(body("bug", "   ", None, None, "en").is_err());
        let long = "x".repeat(COMMENT_MAX + 10);
        let b = body("idea", &long, None, None, "ja").unwrap();
        assert_eq!(b["comment"].as_str().unwrap().chars().count(), COMMENT_MAX);
        assert_eq!((b["diagnostics"].as_str(), b["image"].is_null(), b["lang"].as_str(), b["version"].as_str()), (Some(""), true, Some("ja"), Some(VERSION)));
        assert!(b["log"].is_null());
        let b = body("bug", "", Some(("report", &b"\x1f\x8b"[..])), Some(&b"\x89PNG"[..]), "en").unwrap();
        assert_eq!((b["diagnostics"].as_str(), b["image"].as_str(), b["log"].as_str()), (Some("report"), Some("iVBORw=="), Some("H4s=")));
        assert!(body("bug", "", Some(("report", &[][..])), None, "en").unwrap()["log"].is_null());
        assert!(body("bug", "x", None, Some(vec![0; IMAGE_MAX + 1].as_slice()), "en").is_err());
    }

    #[test]
    fn a_window_picture_turns_bgra_into_rgb_and_shrinks_by_the_step() {
        let bgra: Vec<u8> = (0..4u8).flat_map(|i| [i, 10 + i, 20 + i, 255]).collect();
        assert_eq!(rgb(&bgra, (2, 2), 1), (vec![20, 10, 0, 21, 11, 1, 22, 12, 2, 23, 13, 3], (2, 2)));
        assert_eq!(rgb(&bgra, (2, 2), 2), (vec![20, 10, 0], (1, 1)));
        let png = png(&[1, 2, 3], (1, 1)).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn the_report_id_is_read_from_the_reply() {
        assert_eq!(reply_id(r#"{"ok": true, "id": "20261023-232413"}"#).as_deref(), Some("20261023-232413"));
        assert_eq!(reply_id("{\"ok\": true, \"id\": \"20261023-232413-2\"}\n").as_deref(), Some("20261023-232413-2"));
        assert_eq!(reply_id(r#"{"ok": true, "id": "<b>x</b>"}"#), None);
        assert_eq!(reply_id(r#"{"ok": true}"#), None);
        assert_eq!(reply_id("not json"), None);
    }

    #[test]
    fn send_errors_say_what_to_do() {
        assert_eq!(send_error("The remote server returned an error: (429) Too Many Requests."), "Too many reports from this PC. Try again later.");
        assert_eq!(send_error("Unable to connect"), "Couldn't send. Check your connection.");
    }
}

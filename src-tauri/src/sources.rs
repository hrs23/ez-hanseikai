use std::collections::HashMap;

use serde_json::{json, Value};
use windows_capture::monitor::Monitor;
use windows_capture::window::Window;

use crate::Core;

const GAME_EXE: &str = "r5apex";

#[derive(Clone, Copy)]
pub enum Item {
    Window(Window),
    Display(Monitor),
}

pub struct Found {
    pub item: Item,
    pub key: usize,
    pub kind: &'static str,
    pub name: String,
    pub game: (String, u32),
}

pub fn is_game(exe: &str) -> bool {
    exe.to_lowercase().starts_with(GAME_EXE)
}

pub fn processes() -> Vec<(String, u32)> {
    use ::windows::Win32::Foundation::CloseHandle;
    use ::windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let mut out = Vec::new();
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            let len = entry.szExeFile.iter().position(|c| *c == 0).unwrap_or(entry.szExeFile.len());
            out.push((String::from_utf16_lossy(&entry.szExeFile[..len]), entry.th32ProcessID));
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        let _ = CloseHandle(snapshot);
    }
    out
}

pub fn audio_target(wanted: &str) -> (String, u32) {
    processes().into_iter().find(|(name, _)| if wanted.is_empty() { is_game(name) } else { name.eq_ignore_ascii_case(wanted) }).unwrap_or_default()
}

pub fn pid_of(name: &str) -> Option<u32> {
    processes().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, pid)| pid)
}

fn cloaked(w: &Window) -> bool {
    use ::windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    let mut value = 0u32;
    let hwnd = ::windows::Win32::Foundation::HWND(w.as_raw_hwnd());
    let got = unsafe { DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED, &mut value as *mut u32 as *mut _, std::mem::size_of::<u32>() as u32) };
    got.is_ok() && value != 0
}

fn minimized(w: &Window) -> bool {
    unsafe { ::windows::Win32::UI::WindowsAndMessaging::IsIconic(::windows::Win32::Foundation::HWND(w.as_raw_hwnd())) }.as_bool()
}

fn windows() -> Vec<(Window, String, String, u32)> {
    let names: HashMap<u32, String> = processes().into_iter().map(|(name, pid)| (pid, name)).collect();
    let own = std::process::id();
    Window::enumerate()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|w| {
            let pid = w.process_id().ok().filter(|p| *p != own)?;
            let title = w.title().ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())?;
            if w.width().ok()? <= 0 || w.height().ok()? <= 0 || cloaked(&w) {
                return None;
            }
            Some((w, names.get(&pid)?.clone(), title, pid))
        })
        .collect()
}

pub fn list() -> Vec<Value> {
    let mut list: Vec<(Window, String, String, u32)> = windows().into_iter().filter(|w| !minimized(&w.0)).collect();
    list.sort_by_key(|w| !is_game(&w.1));
    let windows = list.into_iter().map(|(_, exe, title, _)| json!({"id": {"kind": "window", "exe": exe, "title": title}, "name": format!("{title} (window)")}));
    let displays = Monitor::enumerate().unwrap_or_default().into_iter().enumerate().map(|(i, m)| {
        let size = m.width().and_then(|w| Ok((w, m.height()?))).map(|(w, h)| format!(" ({w}x{h})")).unwrap_or_default();
        json!({"id": {"kind": "display", "index": i + 1}, "name": format!("Display {}{size}", i + 1)})
    });
    windows.chain(displays).collect()
}

pub fn primary_display() -> Value {
    let all = Monitor::enumerate().unwrap_or_default();
    let index = Monitor::primary().ok().and_then(|p| all.iter().position(|m| *m == p)).unwrap_or(0);
    json!({"kind": "display", "index": index + 1})
}

pub fn valid(source: &Value) -> bool {
    match source["kind"].as_str() {
        Some("window") => source["exe"].is_string() && source["title"].is_string(),
        Some("display") => source["index"].as_u64().is_some_and(|i| i > 0),
        _ => source.is_null(),
    }
}

pub fn find(core: &Core, source: &Value) -> Option<Found> {
    match source["kind"].as_str()? {
        "window" => {
            let (exe, title) = (source["exe"].as_str()?, source["title"].as_str()?);
            let list = windows();
            let (w, exe, title, pid) = list
                .iter()
                .find(|w| w.1.eq_ignore_ascii_case(exe) && w.2 == title)
                .or_else(|| list.iter().find(|w| w.1.eq_ignore_ascii_case(exe)))?
                .clone();
            Some(Found { item: Item::Window(w), key: w.as_raw_hwnd() as usize, kind: "window", name: format!("{title} ({exe})"), game: (exe, pid) })
        }
        "display" => {
            let index = source["index"].as_u64()? as usize;
            let monitor = *Monitor::enumerate().ok()?.get(index.checked_sub(1)?)?;
            let game = audio_target(&core.audio_process.lock().unwrap());
            Some(Found { item: Item::Display(monitor), key: monitor.as_raw_hmonitor() as usize, kind: "display", name: format!("Display {index}"), game })
        }
        _ => None,
    }
}

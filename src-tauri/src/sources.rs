use std::collections::HashMap;

use serde_json::{json, Value};
use windows_capture::monitor::Monitor;
use windows_capture::window::Window;

use crate::Core;

#[derive(Clone, Copy)]
pub enum Item {
    Window(Window),
    Display(Monitor),
    Test,
}

pub struct Found {
    pub item: Item,
    pub key: usize,
    pub kind: &'static str,
    pub name: String,
    pub game: (String, u32),
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

fn process(name: &str) -> Option<(String, u32)> {
    processes().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(name))
}

pub fn audio_target(wanted: &str) -> (String, u32) {
    if wanted.is_empty() {
        return Default::default();
    }
    process(wanted).unwrap_or_default()
}

pub fn pid_of(name: &str) -> Option<u32> {
    process(name).map(|(_, pid)| pid)
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

fn title(w: &Window) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { ::windows::Win32::UI::WindowsAndMessaging::InternalGetWindowText(::windows::Win32::Foundation::HWND(w.as_raw_hwnd()), &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize]).trim().to_string()
}

fn windows() -> Vec<(Window, String, String, u32)> {
    let names: HashMap<u32, String> = processes().into_iter().map(|(name, pid)| (pid, name)).collect();
    let own = std::process::id();
    Window::enumerate()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|w| {
            let pid = w.process_id().ok().filter(|p| *p != own)?;
            let title = Some(title(&w)).filter(|t| !t.is_empty())?;
            if w.width().ok()? <= 0 || w.height().ok()? <= 0 || cloaked(&w) {
                return None;
            }
            Some((w, names.get(&pid)?.clone(), title, pid))
        })
        .collect()
}

pub fn list() -> Vec<Value> {
    let windows = windows().into_iter().filter(|w| !minimized(&w.0)).map(|(_, exe, title, _)| json!({"id": {"kind": "window", "exe": exe, "title": title}, "name": format!("{title} (window)")}));
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

pub fn same_window<'a, T>(list: &'a [(T, String, String, u32)], exe: &str, title: &str, key: impl Fn(&T) -> usize, last: Option<usize>) -> Option<&'a (T, String, String, u32)> {
    let exe_matches = |w: &&(T, String, String, u32)| w.1.eq_ignore_ascii_case(exe);
    list.iter().filter(exe_matches).find(|w| w.2 == title).or_else(|| list.iter().filter(exe_matches).find(|w| Some(key(&w.0)) == last))
}

pub fn find(core: &Core, source: &Value, last: Option<usize>) -> Option<Found> {
    if crate::test_mode() {
        return (source["kind"] == "display").then(|| Found { item: Item::Test, key: 0, kind: "test", name: "Test picture".into(), game: ("Test tone".into(), 0) });
    }
    match source["kind"].as_str()? {
        "window" => {
            let (exe, title) = (source["exe"].as_str()?, source["title"].as_str()?);
            let list = windows();
            let (w, exe, title, pid) = same_window(&list, exe, title, |w| w.as_raw_hwnd() as usize, last)?.clone();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn window(key: usize, exe: &str, title: &str) -> (usize, String, String, u32) {
        (key, exe.into(), title.into(), 1)
    }

    #[test]
    fn a_closed_window_is_never_replaced_by_another_window_of_the_same_program() {
        let list = vec![window(1, "browser.exe", "Private mail"), window(2, "game.exe", "Game - loading")];
        assert!(same_window(&list, "browser.exe", "Team stats", |k| *k, Some(9)).is_none());
        assert_eq!(same_window(&list, "BROWSER.exe", "Private mail", |k| *k, None).unwrap().0, 1);
        assert_eq!(same_window(&list, "game.exe", "Game", |k| *k, Some(2)).unwrap().0, 2);
        assert!(same_window(&list, "game.exe", "Game", |k| *k, Some(1)).is_none());
    }
}

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn migrate(old: &Path, new: &Path) {
    if old == new || !old.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(new);
    let Ok(entries) = std::fs::read_dir(old) else { return };
    for entry in entries.flatten() {
        let to = new.join(entry.file_name());
        if !to.exists() {
            let _ = std::fs::rename(entry.path(), &to);
        }
    }
    if std::fs::read_dir(old).map(|mut d| d.next().is_none()).unwrap_or(false) {
        let _ = std::fs::remove_dir(old);
    }
}

pub fn used_bytes(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => used_bytes(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

pub fn free_bytes(dir: &Path) -> u64 {
    let Some(letter) = dir.to_string_lossy().chars().next() else { return 0 };
    Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!("(Get-PSDrive {letter}).Free")])
        .creation_flags(0x08000000)
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
}

pub fn open_folder(dir: &Path) {
    let _ = Command::new("explorer").arg(dir).spawn();
}

pub fn reset(dir: &Path, everything: bool) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let settings = ["ffmpeg.exe", "ffmpeg.exe.part", "ffmpeg.zip", "log.txt", "log.old.txt", "report.txt", "peers.txt", "settings.json", "id.txt", "version.txt"];
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "EBWebView" || name == "webview" || name == "running.marker" {
            continue;
        }
        if everything || settings.contains(&name.as_str()) {
            let path: PathBuf = entry.path();
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
    }
}

pub fn delete_old(core: &crate::Core) {
    let Some(days) = *core.keep_days.lock().unwrap() else { return };
    let limit = std::time::Duration::from_secs(days as u64 * 86400);
    let active = core.recording_name.lock().unwrap().clone();
    let mut dirs = vec![core.dir.clone()];
    if let Ok(entries) = std::fs::read_dir(core.dir.join("peer")) {
        dirs.extend(entries.flatten().map(|e| e.path()));
    }
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > limit);
            if name.starts_with("rec-") && name.ends_with(".mp4") && Some(&name) != active.as_ref() && old {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

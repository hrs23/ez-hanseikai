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
    let active = core.active_set.lock().unwrap().as_ref().map(|s| s.id.clone());
    let Ok(rooms) = std::fs::read_dir(core.dir.join("rooms")) else { return };
    for room in rooms.flatten() {
        let Ok(sets) = std::fs::read_dir(room.path().join("sets")) else { continue };
        for set in sets.flatten() {
            let name = set.file_name().to_string_lossy().to_string();
            let old = set.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > limit);
            if old && Some(&name) != active.as_ref() {
                let _ = std::fs::remove_dir_all(set.path());
            }
        }
    }
}

pub fn tz_offset_seconds() -> i64 {
    Command::new("powershell")
        .args(["-NoProfile", "-Command", "[int][TimeZoneInfo]::Local.GetUtcOffset([DateTime]::UtcNow).TotalSeconds"])
        .creation_flags(0x08000000)
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
}

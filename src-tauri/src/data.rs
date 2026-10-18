use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::room::{self, Set};
use crate::{debug, hex, now_us, read_json, Core};

pub const LOW_SHARE: f64 = 0.10;
pub const FULL_SHARE: f64 = 0.01;
const KEEP_EVERY: Duration = Duration::from_secs(5);
const AGE_EVERY: Duration = Duration::from_secs(3600);
const DAY_US: i64 = 86_400_000_000;
const CHECKS: &str = "verified.json";

impl Core {
    pub fn json(&self, path: &Path) -> Value {
        self.jsons.lock().unwrap().entry(path.to_path_buf()).or_insert_with(|| read_json(path)).clone()
    }

    pub fn write_json(&self, path: &Path, value: Value) {
        let _ = std::fs::write(path, value.to_string());
        self.jsons.lock().unwrap().insert(path.to_path_buf(), value);
    }
}

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

pub fn refresh_disk(core: &Core) {
    use ::windows::core::PCWSTR;
    use ::windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let wide: Vec<u16> = core.dir.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
    let (mut free, mut total) = (0u64, 0u64);
    if unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut free as *mut u64), Some(&mut total as *mut u64), None) }.is_err() {
        (free, total) = (0, 0);
    }
    core.free_bytes.store(free, Ordering::Relaxed);
    core.total_bytes.store(total, Ordering::Relaxed);
}

pub fn open_folder(dir: &Path) {
    let _ = Command::new("explorer").arg(dir).spawn();
}

pub fn reset(dir: &Path, everything: bool) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let settings = ["ffmpeg.exe", "ffmpeg.exe.part", "ffmpeg.zip", "log.txt", "log.old.txt", "report.txt", "settings.json", "id.txt", "version.txt"];
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

pub fn tz_offset_seconds() -> i64 {
    use ::windows::Win32::System::Time::{GetTimeZoneInformation, TIME_ZONE_INFORMATION};
    let mut tz = TIME_ZONE_INFORMATION::default();
    let bias = match unsafe { GetTimeZoneInformation(&mut tz) } {
        1 => tz.Bias + tz.StandardBias,
        2 => tz.Bias + tz.DaylightBias,
        u32::MAX => 0,
        _ => tz.Bias,
    };
    -(bias as i64) * 60
}

pub fn sha256(path: &Path) -> Option<(u64, String)> {
    let mut hasher = Sha256::new();
    let size = std::io::copy(&mut std::fs::File::open(path).ok()?, &mut hasher).ok()?;
    Some((size, hex(&hasher.finalize())))
}

fn stamp(path: &Path) -> Option<(u64, u128)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_millis()))
}

fn checks(core: &Core, dir: &Path) -> Value {
    Some(core.json(&dir.join(CHECKS))).filter(Value::is_object).unwrap_or_else(|| json!({}))
}

fn current(entry: &Value, path: &Path) -> bool {
    stamp(path).is_some_and(|(size, mtime)| entry["size"] == size && entry["mtime"] == mtime as u64)
}

pub fn verified_files(core: &Core, dir: &Path) -> HashSet<String> {
    let all = checks(core, dir);
    let Some(map) = all.as_object() else { return HashSet::new() };
    map.iter().filter(|(file, e)| e["ok"] == true && current(e, &dir.join(file))).map(|(file, _)| file.clone()).collect()
}

fn verify_set(core: &Core, set: &Set) {
    let dir = core.set_dir(&set.room, &set.id);
    let mut all = checks(core, &dir);
    let mut changed = false;
    for f in &set.files {
        let (Some(total), Some(sha)) = (f.total, f.sha256.as_deref()) else { continue };
        let file = format!("{}.mp4", f.owner);
        let path = dir.join(&file);
        let Some((size, mtime)) = stamp(&path) else { continue };
        let (fresh, was_ok) = {
            let entry = &all[&file];
            (!(current(entry, &path) && entry["sha256"] == sha), entry["ok"] == true)
        };
        if fresh && size < total {
            continue;
        }
        let ok = if fresh {
            let ok = size == total && sha256(&path).is_some_and(|(_, got)| got == sha);
            all[&file] = json!({"size": size, "mtime": mtime as u64, "sha256": sha, "ok": ok});
            changed = true;
            if !ok {
                core.append_log(&json!({"k": "file_bad", "room": set.room, "set": set.id, "file": file, "size": size, "t_us": now_us()}));
                debug::event(core, core.started, format!("{} {file}: copy does not match its size and SHA-256", set.id));
            }
            ok
        } else {
            was_ok
        };
        if !ok && room::someone_else_has(core, &set.id, &f.owner) {
            let _ = std::fs::remove_file(&path);
            all.as_object_mut().unwrap().remove(&file);
            changed = true;
            debug::event(core, core.started, format!("{} {file}: removed the bad copy to fetch it again", set.id));
        }
    }
    if changed {
        core.write_json(&dir.join(CHECKS), all);
    }
}

fn all_sets(core: &Core) -> Vec<(Set, bool)> {
    let rooms: Vec<String> = core.rooms.lock().unwrap().list.iter().map(|r| r.id.clone()).collect();
    let mut out = Vec::new();
    for room in rooms {
        let logs = room::logs(core, &room);
        let notes = room::notes(&logs, &room);
        for set in room::sets(core, &room, &logs) {
            let noted = notes.iter().any(|n| n["local_us"].as_i64().is_some_and(|t| set.covers(t)));
            out.push((set, noted));
        }
    }
    out
}

fn delete_old(core: &Core) {
    let Some(days) = *core.keep_days.lock().unwrap() else { return };
    let limit = days as i64 * DAY_US;
    for (set, _) in all_sets(core) {
        if !core.is_recording(&set.room, &set.id) && set.end().is_some_and(|end| now_us() - end > limit) {
            let _ = room::delete_set(core, &set.room, &set.id);
        }
    }
}

fn free_space(core: &Core) {
    if !core.disk_low() {
        return;
    }
    let mut old: Vec<(bool, i64, Set)> = all_sets(core)
        .into_iter()
        .filter(|(s, _)| !core.is_recording(&s.room, &s.id) && s.end().is_some() && core.set_dir(&s.room, &s.id).exists())
        .map(|(s, noted)| (noted, s.start(), s))
        .collect();
    old.sort_by_key(|(noted, start, _)| (*noted, *start));
    let mut removed = Vec::new();
    for (_, _, set) in old {
        if !core.disk_low() {
            break;
        }
        if room::delete_set(core, &set.room, &set.id).is_ok() {
            removed.push(set.id.clone());
            refresh_disk(core);
        }
    }
    if !removed.is_empty() {
        *core.notice.lock().unwrap() = format!("Low disk space: removed {} old set(s) from this PC: {}.", removed.len(), removed.join(", "));
        debug::event(core, core.started, format!("low disk space, removed sets {}", removed.join(", ")));
    }
}

pub fn unhashed(core: &Core) -> Vec<(String, String)> {
    let own = core.own_log();
    let hashed: HashSet<(&str, &str)> = own.iter().filter(|r| r["k"] == "file").filter_map(|r| Some((r["room"].as_str()?, r["set"].as_str()?))).collect();
    own.iter()
        .filter(|r| r["k"] == "rec")
        .filter_map(|r| Some((r["room"].as_str()?, r["set"].as_str()?)))
        .filter(|key| !hashed.contains(key))
        .map(|(room, set)| (room.to_string(), set.to_string()))
        .collect()
}

pub fn record_file(core: &Core, room: &str, set: &str) {
    if let Some((size, sha)) = sha256(&core.own_file(room, set)) {
        core.append_log(&json!({"k": "file", "room": room, "set": set, "file": format!("{}.mp4", core.ep.id()), "size": size, "sha256": sha, "t_us": now_us()}));
    }
}

pub fn keeper(core: Arc<Core>, unhashed: Vec<(String, String)>) {
    let own = core.own_log();
    let ended: HashSet<&str> = own.iter().filter(|r| r["k"] == "rec_end").filter_map(|r| r["set"].as_str()).collect();
    for (room, set) in unhashed {
        if !ended.contains(set.as_str()) {
            let end = stamp(&core.own_file(&room, &set)).map(|(_, mtime)| mtime as i64 * 1000).unwrap_or_else(now_us);
            core.append_log(&json!({"k": "rec_end", "set": set, "room": room, "t_us": end}));
        }
        record_file(&core, &room, &set);
    }
    let mut aged: Option<Instant> = None;
    loop {
        if core.disk_full() && core.active_set.lock().unwrap().is_some() {
            room::stop_set(&core);
            *core.notice.lock().unwrap() = "Recording stopped: the disk is almost full.".into();
        }
        if aged.is_none_or(|t| t.elapsed() > AGE_EVERY) {
            delete_old(&core);
            aged = Some(Instant::now());
        }
        free_space(&core);
        let deleted: Vec<String> = core.deleted.lock().unwrap().iter().cloned().collect();
        for key in deleted {
            if let Some((room, set)) = key.split_once('/') {
                let dir = core.set_dir(room, set);
                if dir.exists() {
                    let _ = std::fs::remove_dir_all(dir);
                }
            }
        }
        let room = core.active_room_id();
        for set in room::sets(&core, &room, &room::logs(&core, &room)) {
            verify_set(&core, &set);
        }
        std::thread::sleep(KEEP_EVERY);
    }
}

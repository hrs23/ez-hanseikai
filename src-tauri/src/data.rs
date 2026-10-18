use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::room::{self, Room, Set};
use crate::{debug, hex, log, now_us, read_json, short, Core};

pub const LOW_SHARE: f64 = 0.10;
pub const FULL_SHARE: f64 = 0.01;
const KEEP_EVERY: Duration = Duration::from_secs(5);
const AGE_EVERY: Duration = Duration::from_secs(3600);
const DAY_US: i64 = 86_400_000_000;
const CHECKS: &str = "verified.json";
const OLD_AFTER: Duration = Duration::from_secs(7 * 86_400);

impl Core {
    pub fn json(&self, path: &Path) -> Value {
        self.jsons.lock().unwrap().entry(path.to_path_buf()).or_insert_with(|| read_json(path)).clone()
    }

    pub fn write_json(&self, path: &Path, value: Value) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
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

pub fn assign(records: &[serde_json::Value]) -> Vec<Option<String>> {
    let mut sets: HashMap<String, String> = HashMap::new();
    let mut notes: HashMap<i64, String> = HashMap::new();
    let mut out = Vec::new();
    for r in records {
        let id = r["id"].as_i64().or_else(|| r["t_us"].as_i64());
        let room = r["room"]
            .as_str()
            .filter(|x| room::valid_id(x))
            .map(String::from)
            .or_else(|| r["set"].as_str().and_then(|s| sets.get(s).cloned()))
            .or_else(|| r["k"].as_str().filter(|k| k.starts_with("note_")).and_then(|_| notes.get(&r["id"].as_i64()?).cloned()));
        if let Some(room) = &room {
            if let Some(set) = r["set"].as_str() {
                sets.entry(set.to_string()).or_insert_with(|| room.clone());
            }
            if let (Some("note"), Some(id)) = (r["k"].as_str(), id) {
                notes.entry(id).or_insert_with(|| room.clone());
            }
        }
        out.push(room);
    }
    out
}

fn split(path: &Path, owner: &str, keep: impl Fn(&str) -> bool, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
    let (records, _) = log::scan(path, 0);
    for (r, room) in records.iter().zip(assign(&records)) {
        if let Some(room) = room.filter(|room| keep(room)) {
            out.entry(dir.join("rooms").join(&room).join("logs").join(format!("{owner}.bin"))).or_default().extend(log::encode(r));
        }
    }
}

pub fn split_old_logs(dir: &Path, me: &str, rooms: &[Room]) {
    let old = dir.join("log.bin");
    if !old.exists() {
        return;
    }
    log::repair(&old);
    let mut out = BTreeMap::new();
    split(&old, me, |_| true, dir, &mut out);
    for entry in std::fs::read_dir(dir.join("peer")).into_iter().flatten().flatten() {
        let prefix = entry.file_name().to_string_lossy().to_string();
        let ids: HashSet<&String> = rooms.iter().flat_map(|r| r.members.iter()).filter(|m| short(m) == prefix && *m != me).collect();
        for id in ids {
            let shared = |room: &str| rooms.iter().any(|r| r.id == room && r.members.contains(id));
            split(&entry.path().join("log.bin"), id, shared, dir, &mut out);
        }
    }
    for (path, bytes) in out {
        if path.exists() {
            continue;
        }
        let tmp = path.with_extension("tmp");
        let written = path.parent().is_some_and(|p| std::fs::create_dir_all(p).is_ok()) && std::fs::write(&tmp, bytes).is_ok();
        if !written || std::fs::rename(&tmp, &path).is_err() {
            return;
        }
    }
    let legacy = dir.join("legacy");
    let _ = std::fs::create_dir_all(&legacy);
    if std::fs::rename(&old, legacy.join("log.bin")).is_ok() {
        let _ = std::fs::remove_dir_all(dir.join("peer"));
    }
}

pub fn repair_logs(dir: &Path) {
    for room in std::fs::read_dir(dir.join("rooms")).into_iter().flatten().flatten() {
        log::repair(&room.path().join("pending.bin"));
        for file in std::fs::read_dir(room.path().join("logs")).into_iter().flatten().flatten() {
            log::repair(&file.path());
        }
    }
}

fn holds(dir: &Path, ext: &str) -> bool {
    std::fs::read_dir(dir).into_iter().flatten().flatten().any(|e| {
        let path = e.path();
        if path.is_dir() { holds(&path, ext) } else { path.extension().is_some_and(|x| x == ext) }
    })
}

fn older(path: &Path, now: SystemTime) -> bool {
    std::fs::metadata(path).and_then(|m| m.modified()).is_ok_and(|t| now.duration_since(t).is_ok_and(|age| age > OLD_AFTER))
}

pub fn leftovers(dir: &Path, rooms: &HashSet<String>, active: &str, exporting: bool, now: SystemTime) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir.join("rooms")).into_iter().flatten().flatten() {
        let (id, path) = (entry.file_name().to_string_lossy().to_string(), entry.path());
        let left = !rooms.contains(&id) && id != room::SOLO;
        if left && !holds(&path.join("sets"), "mp4") && !holds(&path.join("logs"), "bin") && !path.join("pending.bin").exists() {
            out.push(path);
            continue;
        }
        if id != active && path.join("live").exists() {
            out.push(path.join("live"));
        }
        for file in std::fs::read_dir(path.join("logs")).into_iter().flatten().flatten().map(|e| e.path()) {
            if file.extension().is_some_and(|x| x == "tmp") {
                out.push(file);
            }
        }
    }
    if !exporting && dir.join("export").exists() {
        out.push(dir.join("export"));
    }
    for name in ["report.txt", "log.old.txt"] {
        if older(&dir.join(name), now) {
            out.push(dir.join(name));
        }
    }
    out
}

fn cleanup(core: &Core) {
    let rooms: HashSet<String> = core.rooms.lock().unwrap().list.iter().map(|r| r.id.clone()).collect();
    let exporting = core.export.lock().unwrap().is_some();
    for path in leftovers(&core.dir, &rooms, &core.active_room_id(), exporting, SystemTime::now()) {
        let removed = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        if removed.is_ok() {
            debug::event(core, core.started, format!("removed {}", path.strip_prefix(&core.dir).unwrap_or(&path).display()));
        }
    }
}

fn recreate(core: &Core) {
    if core.dir.join("rooms.json").exists() && core.dir.join("settings.json").exists() && core.dir.join("key.bin").exists() {
        return;
    }
    let _ = std::fs::create_dir_all(&core.dir);
    if !core.dir.join("rooms.json").exists() {
        core.save_rooms();
    }
    if !core.dir.join("settings.json").exists() {
        core.save_settings();
    }
    let _ = std::fs::write(core.dir.join("id.txt"), core.me());
    if !core.dir.join("key.bin").exists() {
        *core.notice.lock().unwrap() = "key.bin was deleted. The app keeps your ID until it restarts, then makes a new one; to keep it, import your identity after the restart.".into();
    }
    debug::event(core, core.started, "the data folder was missing files; recreated them".into());
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
                core.append_log(&set.room, &json!({"k": "file_bad", "room": set.room, "set": set.id, "file": file, "size": size, "t_us": now_us()}));
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
        let events = room::events(&logs, None);
        for set in room::sets(core, &room, &logs) {
            let noted = events.iter().any(|e| room::covers(&set, e));
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

fn room_ids(core: &Core) -> Vec<String> {
    let mut ids: Vec<String> = core.rooms.lock().unwrap().list.iter().map(|r| r.id.clone()).collect();
    ids.push(room::SOLO.to_string());
    ids
}

pub fn unhashed(core: &Core) -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    for room in room_ids(core) {
        let own = core.own_log(&room);
        let hashed: HashSet<&str> = own.iter().filter(|r| r["k"] == "file").filter_map(|r| r["set"].as_str()).collect();
        let ended: HashSet<&str> = own.iter().filter(|r| r["k"] == "rec_end").filter_map(|r| r["set"].as_str()).collect();
        for set in own.iter().filter(|r| r["k"] == "rec").filter_map(|r| r["set"].as_str()).filter(|s| !hashed.contains(s)) {
            out.push((room.clone(), set.to_string(), ended.contains(set)));
        }
    }
    out
}

pub fn record_file(core: &Core, room: &str, set: &str) {
    if let Some((size, sha)) = sha256(&core.own_file(room, set)) {
        core.append_log(room, &json!({"k": "file", "room": room, "set": set, "file": format!("{}.mp4", core.ep.id()), "size": size, "sha256": sha, "t_us": now_us()}));
    }
}

pub fn keeper(core: Arc<Core>, unhashed: Vec<(String, String, bool)>) {
    for (room, set, ended) in unhashed {
        if !ended {
            let end = stamp(&core.own_file(&room, &set)).map(|(_, mtime)| mtime as i64 * 1000).unwrap_or_else(now_us);
            core.append_log(&room, &json!({"k": "rec_end", "set": set, "room": room, "t_us": end}));
        }
        record_file(&core, &room, &set);
    }
    let mut aged: Option<Instant> = None;
    loop {
        if core.disk_full() && core.active_set.lock().unwrap().is_some() {
            room::stop_set(&core);
            *core.notice.lock().unwrap() = "Recording stopped: the disk is almost full.".into();
        }
        recreate(&core);
        if aged.is_none_or(|t| t.elapsed() > AGE_EVERY) {
            delete_old(&core);
            cleanup(&core);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ezh-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn records_find_their_room() {
        let records = vec![
            json!({"k": "rec", "room": "r1", "set": "s1"}),
            json!({"k": "layout", "set": "s1"}),
            json!({"k": "note", "id": 7, "room": "r2", "text": "x"}),
            json!({"k": "note_edit", "id": 7, "text": "y"}),
            json!({"k": "note", "id": 8, "set": "s1"}),
            json!({"k": "offset", "peer": "p"}),
            json!({"k": "note_del", "id": 99}),
            json!({"k": "rec", "room": "../x", "set": "s9"}),
        ];
        let rooms: Vec<Option<&str>> = vec![Some("r1"), Some("r1"), Some("r2"), Some("r2"), Some("r1"), None, None, None];
        assert_eq!(assign(&records), rooms.iter().map(|r| r.map(String::from)).collect::<Vec<_>>());
    }

    #[test]
    fn old_logs_split_into_room_logs_the_same_way_everywhere() {
        let dir = temp("split");
        let (me, peer) = ("a".repeat(64), "b".repeat(64));
        let records = [
            json!({"k": "rec", "room": "r1", "set": "s1", "start_us": 1}),
            json!({"k": "rec", "room": "r2", "set": "s2", "start_us": 2}),
            json!({"k": "note_edit", "id": 3}),
            json!({"k": "offset", "peer": "x"}),
        ];
        for r in &records {
            log::append(&dir.join("log.bin"), r).unwrap();
            log::append(&dir.join("peer").join(short(&peer)).join("log.bin"), r).unwrap();
        }
        let rooms = vec![
            Room { id: "r1".into(), members: vec![me.clone(), peer.clone()], ..Default::default() },
            Room { id: "r2".into(), members: vec![me.clone()], ..Default::default() },
        ];
        split_old_logs(&dir, &me, &rooms);
        let at = |room: &str, id: &str| log::scan(&dir.join("rooms").join(room).join("logs").join(format!("{id}.bin")), 0).0;
        assert_eq!(at("r1", &me), vec![records[0].clone()]);
        assert_eq!(at("r2", &me), vec![records[1].clone()]);
        assert_eq!(at("r1", &peer), vec![records[0].clone()]);
        assert!(at("r2", &peer).is_empty());
        assert_eq!(std::fs::read(dir.join("rooms/r1/logs").join(format!("{me}.bin"))).unwrap(), std::fs::read(dir.join("rooms/r1/logs").join(format!("{peer}.bin"))).unwrap());
        assert!(!dir.join("log.bin").exists() && !dir.join("peer").exists());
        assert_eq!(log::scan(&dir.join("legacy/log.bin"), 0).0.len(), 4);
        split_old_logs(&dir, &me, &rooms);
        assert_eq!(at("r1", &me).len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cleanup_picks_only_leftovers() {
        let dir = temp("clean");
        let touch = |p: &str| {
            let path = dir.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        };
        for p in ["rooms/active/live/a-1-1.mp4", "rooms/active/logs/a.bin", "rooms/other/live/a-1-1.mp4", "rooms/other/sets/s/a.mp4", "rooms/gone/live/a-1-2.mp4", "rooms/gone/sets/s/verified.json", "rooms/kept/logs/a.bin", "rooms/kept/live/x.mp4", "rooms/active/logs/a.tmp", "export/label-0.txt", "report.txt", "key.bin", "legacy/log.bin", "rooms/solo/sets/s/set.json"] {
            touch(p);
        }
        let rooms: HashSet<String> = ["active".to_string(), "other".to_string()].into();
        let later = SystemTime::now() + OLD_AFTER + Duration::from_secs(60);
        let mut got: Vec<String> = leftovers(&dir, &rooms, "active", false, later).iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
        got.sort();
        assert_eq!(got, ["export", "report.txt", "rooms/active/logs/a.tmp", "rooms/gone", "rooms/kept/live", "rooms/other/live"]);
        let now: Vec<PathBuf> = leftovers(&dir, &rooms, "active", true, SystemTime::now());
        assert!(!now.iter().any(|p| p.ends_with("export") || p.ends_with("report.txt")));
        let _ = std::fs::remove_dir_all(dir);
    }
}

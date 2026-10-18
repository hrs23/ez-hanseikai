use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::room::{self, part_of, seg_name, File, Seg, Set, INIT, SERIES};
use crate::{debug, file_len, hex, log, now_us, read_json, record, short, Core};

pub const LOW_SHARE: f64 = 0.10;
pub const FULL_SHARE: f64 = 0.01;
const KEEP_EVERY: Duration = Duration::from_secs(5);
const AGE_EVERY: Duration = Duration::from_secs(3600);
const DAY_US: i64 = 86_400_000_000;
const CHECKS: &str = "verified.json";
const OLD_AFTER: Duration = Duration::from_secs(7 * 86_400);
pub const LIVE_KEEP: u32 = 2 * crate::stream::LIVE_WINDOW + 1;
static CHECKS_LOCK: Mutex<()> = Mutex::new(());

impl Core {
    pub fn with_json<R>(&self, path: &Path, f: impl FnOnce(&Value) -> R) -> R {
        f(self.jsons.lock().unwrap().entry(path.to_path_buf()).or_insert_with(|| read_json(path)))
    }

    pub fn write_json(&self, path: &Path, value: Value) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = crate::write_whole(path, value.to_string().as_bytes());
        self.jsons.lock().unwrap().insert(path.to_path_buf(), value);
    }
}

pub fn repair_logs(dir: &Path) {
    for room in std::fs::read_dir(dir.join("rooms")).into_iter().flatten().flatten() {
        log::repair(&room.path().join("pending.bin"));
        for file in std::fs::read_dir(room.path().join("logs")).into_iter().flatten().flatten() {
            let path = file.path();
            if path.extension().is_some_and(|e| e == "bin") && path.file_stem().is_some_and(|s| s.to_string_lossy().parse::<iroh::EndpointId>().is_ok()) {
                log::repair(&path);
            }
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

pub fn leftovers(dir: &Path, rooms: &HashSet<String>, exporting: bool, now: SystemTime) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir.join("rooms")).into_iter().flatten().flatten() {
        let (id, path) = (entry.file_name().to_string_lossy().to_string(), entry.path());
        let left = !rooms.contains(&id) && id != room::SOLO;
        if left && !holds(&path, "m4s") && !holds(&path, "mp4") && !holds(&path.join("logs"), "bin") && !path.join("pending.bin").exists() {
            out.push(path);
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
    for path in leftovers(&core.dir, &rooms, exporting, SystemTime::now()) {
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
    core.jsons.lock().unwrap().clear();
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

pub fn checked(core: &Core, dir: &Path, name: &str, size: u64) -> Option<bool> {
    core.with_json(&dir.join(CHECKS), |all| {
        let entry = &all[name];
        (entry["size"] == size).then(|| entry["ok"] == true)
    })
}

fn forget_deleted(core: &Core, path: &Path) {
    if !path.exists() {
        core.jsons.lock().unwrap().remove(path);
    }
}

pub fn mark(core: &Core, dir: &Path, name: &str, size: u64, ok: bool) {
    let _guard = CHECKS_LOCK.lock().unwrap();
    forget_deleted(core, &dir.join(CHECKS));
    let mut all = core.with_json(&dir.join(CHECKS), |v| if v.is_object() { v.clone() } else { json!({}) });
    all[name] = json!({"size": size, "ok": ok});
    core.write_json(&dir.join(CHECKS), all);
}

fn unmark(core: &Core, dir: &Path, name: &str) {
    let _guard = CHECKS_LOCK.lock().unwrap();
    forget_deleted(core, &dir.join(CHECKS));
    let mut all = core.with_json(&dir.join(CHECKS), |v| v.clone());
    if let Some(map) = all.as_object_mut() {
        if map.remove(name).is_some() {
            core.write_json(&dir.join(CHECKS), all);
        }
    }
}

fn seg_file(seg: &Seg) -> String {
    format!("{}/{}", seg.series, seg_name(seg.n))
}

pub fn seg_check(core: &Core, room: &str, owner: &str, seg: &Seg) -> Option<bool> {
    checked(core, &core.capture_dir(room, owner, &seg.capture), &seg_file(seg), seg.size)
}

pub fn init_check(core: &Core, room: &str, f: &File, capture: &str, series: &str) -> Option<bool> {
    let (size, _) = f.init(capture, series)?;
    checked(core, &core.capture_dir(room, &f.owner, capture), &format!("{series}/{INIT}"), size)
}

pub fn prefix(oks: impl IntoIterator<Item = bool>) -> u64 {
    oks.into_iter().take_while(|ok| *ok).count() as u64
}

pub fn count(core: &Core, room: &str, f: &File, series: &str) -> u64 {
    prefix(f.series(series).into_iter().map(|seg| init_check(core, room, f, &seg.capture, series) == Some(true) && seg_check(core, room, &f.owner, seg) == Some(true)))
}

pub fn matches(path: &Path, size: u64, sha: &str) -> bool {
    sha256(path).is_some_and(|(got_size, got)| got_size == size && got == sha)
}

pub fn have_counts(core: &Core, room: &str, sets: &[Set]) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    for set in sets {
        for f in &set.files {
            for series in SERIES {
                let n = count(core, room, f, series);
                if n > 0 {
                    out.insert(room::have_key(&set.id, &f.owner, series), n);
                }
            }
        }
    }
    out
}

pub fn held(core: &Core, room: &str, f: &File) -> (u64, u64) {
    let mut have: u64 = f.segs.iter().filter(|seg| seg_check(core, room, &f.owner, seg) == Some(true)).map(|seg| seg.size).sum();
    for capture in f.captures() {
        for series in SERIES {
            if init_check(core, room, f, capture, series) == Some(true) {
                have += f.init(capture, series).map_or(0, |(size, _)| size);
            }
        }
    }
    (have, f.total())
}

pub struct Run {
    pub capture: String,
    pub init: bool,
    pub from: u32,
    pub at: u64,
    pub to: u32,
    pub index: u64,
}

fn present(core: &Core, path: &Path) -> bool {
    path.exists() || core.writing.lock().unwrap().contains(path)
}

pub fn missing(core: &Core, room: &str, f: &File, series: &str) -> Option<Run> {
    let segs = f.series(series);
    for (i, seg) in segs.iter().enumerate() {
        if init_check(core, room, f, &seg.capture, series) == Some(true) && seg_check(core, room, &f.owner, seg) == Some(true) {
            continue;
        }
        let lacks_init = !present(core, &core.capture_dir(room, &f.owner, &seg.capture).join(series).join(INIT));
        let path = core.seg_path(room, &f.owner, seg);
        let here = present(core, &path);
        if !lacks_init && here {
            continue;
        }
        let mut to = if here { seg.n - 1 } else { seg.n };
        for next in segs[i + 1..].iter().filter(|_| !here) {
            if next.capture != seg.capture || next.n != to + 1 || present(core, &core.seg_path(room, &f.owner, next)) {
                break;
            }
            to = next.n;
        }
        let at = if here { 0 } else { file_len(&part_of(&path)) };
        return Some(Run { capture: seg.capture.clone(), init: lacks_init, from: seg.n, at, to, index: i as u64 });
    }
    None
}

fn bad(core: &Core, set: &Set, f: &File, path: &Path, dir: &Path, name: &str, key: &str, index: u64) {
    if room::someone_else_has(core, key, index) {
        let _ = std::fs::remove_file(path);
        unmark(core, dir, name);
        debug::event(core, core.started, format!("{} {}: removed the bad copy of {name} to fetch it again", set.id, short(&f.owner)));
    }
}

fn verify(core: &Core, set: &Set) {
    for f in &set.files {
        for series in SERIES {
            let key = room::have_key(&set.id, &f.owner, series);
            for (i, seg) in f.series(series).into_iter().enumerate() {
                let i = i as u64;
                let dir = core.capture_dir(&set.room, &f.owner, &seg.capture);
                let name = format!("{series}/{INIT}");
                let path = dir.join(&name);
                match (f.init(&seg.capture, series), init_check(core, &set.room, f, &seg.capture, series)) {
                    (Some((size, sha)), None) if path.exists() => {
                        let ok = matches(&path, size, &sha);
                        mark(core, &dir, &name, size, ok);
                        if !ok {
                            bad(core, set, f, &path, &dir, &name, &key, i);
                        }
                    }
                    (Some(_), Some(false)) => bad(core, set, f, &path, &dir, &name, &key, i),
                    _ => {}
                }
                let name = seg_file(seg);
                let path = dir.join(&name);
                match seg_check(core, &set.room, &f.owner, seg) {
                    Some(true) => {}
                    Some(false) => bad(core, set, f, &path, &dir, &name, &key, i),
                    None => {
                        if !path.exists() {
                            continue;
                        }
                        let len = file_len(&path);
                        let ok = matches(&path, seg.size, &seg.sha256);
                        mark(core, &dir, &name, seg.size, ok);
                        if !ok {
                            debug::hash_fail(&f.owner, &seg.capture, series, seg.n, len);
                            core.append_log(&set.room, &json!({"k": "file_bad", "room": set.room, "set": set.id, "file": format!("{}/{}/{name}", f.owner, seg.capture), "size": len, "t_us": now_us()}));
                            debug::event(core, core.started, format!("{} {}: {name} does not match its size and SHA-256", set.id, short(&f.owner)));
                            bad(core, set, f, &path, &dir, &name, &key, i);
                        }
                    }
                }
            }
        }
    }
}

pub fn remove_media(core: &Core, set: &Set) {
    for f in &set.files {
        for seg in &f.segs {
            let path = core.seg_path(&set.room, &f.owner, seg);
            let _ = std::fs::remove_file(part_of(&path));
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn doomed(present: &[u32], kept: &HashSet<u32>, live_from: Option<u32>) -> Vec<u32> {
    present.iter().copied().filter(|n| !kept.contains(n) && live_from.is_none_or(|from| *n < from)).collect()
}

fn numbers(dir: &Path) -> Vec<u32> {
    let mut out: Vec<u32> = std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| room::seg_number(&e.file_name().to_string_lossy())).collect();
    out.sort();
    out.dedup();
    out
}

fn changed_since(path: &Path, since: SystemTime) -> bool {
    std::fs::metadata(path).and_then(|m| m.modified()).is_ok_and(|t| t >= since)
}

fn clean_media(core: &Core) {
    let since = SystemTime::now() - KEEP_EVERY;
    let me = core.me();
    let active = core.active_room_id();
    let own_live = core.engine.live().map(|(room, capture, _)| (room, capture));
    let writing: Vec<PathBuf> = core.writing.lock().unwrap().iter().cloned().collect();
    for room in room_ids(core) {
        let mut kept: HashMap<(String, String, String), HashSet<u32>> = HashMap::new();
        for set in room::sets(core, &room, &room::logs(core, &room)) {
            for f in &set.files {
                for seg in &f.segs {
                    kept.entry((f.owner.clone(), seg.capture.clone(), seg.series.clone())).or_default().insert(seg.n);
                }
            }
        }
        for owner in std::fs::read_dir(core.dir.join("rooms").join(&room)).into_iter().flatten().flatten() {
            let id = owner.file_name().to_string_lossy().to_string();
            if id.parse::<iroh::EndpointId>().is_err() || (id == me && !core.log_ready(&room)) {
                continue;
            }
            let mut captures: Vec<String> = std::fs::read_dir(owner.path()).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
            captures.sort_by_key(|c| c.parse::<u64>().unwrap_or(0));
            let newest = captures.last().cloned();
            for capture in captures {
                let dir = owner.path().join(&capture);
                let live = room == active && if id == me { own_live.as_ref() == Some(&(room.clone(), capture.clone())) } else { newest.as_ref() == Some(&capture) };
                let any_kept = kept.keys().any(|(o, c, _)| *o == id && *c == capture);
                let fresh = SERIES.iter().any(|series| changed_since(&dir.join(series), since) || dir.join(series).join(record::OPEN).exists());
                if !any_kept && !live && !fresh && !writing.iter().any(|w| w.starts_with(&dir)) {
                    if std::fs::remove_dir_all(&dir).is_ok() {
                        core.jsons.lock().unwrap().remove(&dir.join(CHECKS));
                        debug::event(core, core.started, format!("removed {}", dir.strip_prefix(&core.dir).unwrap_or(&dir).display()));
                    }
                    continue;
                }
                for series in SERIES {
                    let sdir = dir.join(series);
                    let present = numbers(&sdir);
                    let empty = HashSet::new();
                    let keep = kept.get(&(id.clone(), capture.clone(), series.to_string())).unwrap_or(&empty);
                    let live_from = live.then(|| present.last().copied().unwrap_or(0).saturating_sub(LIVE_KEEP - 1));
                    for n in doomed(&present, keep, live_from) {
                        let path = sdir.join(seg_name(n));
                        if writing.contains(&path) || changed_since(&path, since) || changed_since(&part_of(&path), since) {
                            continue;
                        }
                        let _ = std::fs::remove_file(part_of(&path));
                        let _ = std::fs::remove_file(path);
                        unmark(core, &dir, &format!("{series}/{}", seg_name(n)));
                    }
                }
            }
        }
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
        if !core.in_use(&set.room, &set.id) && set.end().is_some_and(|end| now_us() - end > limit) {
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
        .filter(|(s, _)| !core.in_use(&s.room, &s.id) && s.end().is_some() && s.files.iter().any(|f| held(core, &s.room, f).0 > 0))
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

pub fn keeper(core: Arc<Core>) {
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
        let room = core.active_room_id();
        for set in room::sets(&core, &room, &room::logs(&core, &room)) {
            verify(&core, &set);
        }
        clean_media(&core);
        std::thread::sleep(KEEP_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ezh-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn cleanup_picks_only_leftovers() {
        let dir = temp("clean");
        let touch = |p: &str| {
            let path = dir.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"x").unwrap();
        };
        for p in ["rooms/active/logs/a.bin", "rooms/other/b/1/v/000001.m4s", "rooms/gone/b/1/v/verified.json", "rooms/kept/logs/a.bin", "rooms/waiting/pending.bin", "rooms/held/b/1/v/000001.m4s", "export/label-0.txt", "report.txt", "key.bin", "rooms/solo/b/1/v/verified.json", "rooms/old/sets/s1/set.json", "rooms/old/sets/s1/a.mp4", "rooms/oldlive/live/a.mp4", "rec-1.mp4", "log.bin", "peer.txt", "peers.txt", "peer/a/log.bin", "legacy/x.bin"] {
            touch(p);
        }
        let rooms: HashSet<String> = ["active".to_string(), "other".to_string()].into();
        let later = SystemTime::now() + OLD_AFTER + Duration::from_secs(60);
        let mut got: Vec<String> = leftovers(&dir, &rooms, false, later).iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
        got.sort();
        assert_eq!(got, ["export", "report.txt", "rooms/gone"]);
        let now: Vec<PathBuf> = leftovers(&dir, &rooms, true, SystemTime::now());
        assert!(!now.iter().any(|p| p.ends_with("export") || p.ends_with("report.txt")));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cleanup_keeps_set_segments_and_the_live_window() {
        let kept: HashSet<u32> = [2, 3].into();
        assert_eq!(doomed(&[1, 2, 3, 4, 5, 6, 7], &kept, None), [1, 4, 5, 6, 7]);
        assert_eq!(doomed(&[1, 2, 3, 4, 5, 6, 7], &kept, Some(6)), [1, 4, 5]);
        assert!(doomed(&[], &kept, Some(1)).is_empty());
    }

    #[test]
    fn copies_are_checked_by_size_and_hash_and_counted_in_order() {
        let dir = temp("verify");
        let path = dir.join("000001.m4s");
        std::fs::write(&path, b"fragment").unwrap();
        let (size, sha) = sha256(&path).unwrap();
        assert!(matches(&path, size, &sha));
        assert!(!matches(&path, size + 1, &sha));
        std::fs::write(&path, b"fragmenT").unwrap();
        assert!(!matches(&path, size, &sha));
        assert!(!matches(&dir.join("missing.m4s"), size, &sha));
        assert_eq!(prefix([true, true, false, true]), 2);
        assert_eq!(prefix([false, true]), 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}

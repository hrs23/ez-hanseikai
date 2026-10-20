use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::room::{self, part_of, seg_name, File, Seg, Set, INIT, SERIES};
use crate::{debug, file_len, hex, log, now_us, read_json, record, short, Core};

pub const FULL_SHARE: f64 = 0.01;
pub const WARN_HOURS: u32 = 10;
pub const WARN_CHOICES: [u64; 5] = [2, 5, 10, 20, 50];
const RATE_SETS: usize = 5;
const KEEP_EVERY: Duration = Duration::from_secs(5);
const AGE_EVERY: Duration = Duration::from_secs(3600);
const MAX_REMOVED: usize = 3;
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
    for name in ["report.txt", "log.old.txt", "log.older.txt", "crash.txt"] {
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

pub fn bytes_per_hour(sets: &[(u64, i64)]) -> Option<f64> {
    let recent: Vec<&(u64, i64)> = sets.iter().filter(|(bytes, us)| *bytes > 0 && *us > 0).take(RATE_SETS).collect();
    let (bytes, us) = recent.iter().fold((0u64, 0i64), |(b, u), (bytes, us)| (b + bytes, u + us));
    (us > 0).then(|| bytes as f64 * 3600e6 / us as f64)
}

pub fn disk_json(free: u64, total: u64, per_hour: Option<f64>, warn_hours: u32, full: bool) -> Value {
    let hours_left = per_hour.map(|rate| free as f64 / rate);
    let low = full || hours_left.is_some_and(|h| h < warn_hours as f64);
    json!({"free": free, "total": total, "per_hour": per_hour, "hours_left": hours_left, "low": low, "full": full})
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

pub fn reset(dir: &Path, keep_rooms: bool) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let rooms = ["key.bin", "rooms.json", "rooms"];
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == "EBWebView" || name == "webview" || name == LOCK || (keep_rooms && rooms.contains(&name.as_str())) {
            continue;
        }
        let path: PathBuf = entry.path();
        let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
    }
}

pub const LOCK: &str = "app.lock";
static HELD: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn lock(dir: &Path) -> std::io::Result<bool> {
    use std::os::windows::fs::OpenOptionsExt;
    let path = dir.join(LOCK);
    let left = path.exists();
    let file = std::fs::OpenOptions::new().write(true).create(true).truncate(false).share_mode(0).open(&path)?;
    *HELD.lock().unwrap() = Some(file);
    Ok(left)
}

pub fn unlock(dir: &Path) {
    if HELD.lock().unwrap().take().is_some() {
        let _ = std::fs::remove_file(dir.join(LOCK));
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
    core.dirs(room, &seg.set, owner, &seg.capture).iter().find_map(|dir| checked(core, dir, &seg_file(seg), seg.size))
}

pub fn init_check(core: &Core, room: &str, f: &File, capture: &str, series: &str) -> Option<bool> {
    let (size, _) = f.init(capture, series)?;
    core.dirs(room, &f.set, &f.owner, capture).iter().find_map(|dir| checked(core, dir, &format!("{series}/{INIT}"), size))
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
    let ok = |seg: &&Seg| seg_check(core, room, &f.owner, seg) == Some(true);
    let mut have: u64 = f.segs.iter().filter(ok).map(|seg| seg.size).sum();
    for capture in f.captures() {
        for series in SERIES {
            if init_check(core, room, f, capture, series) == Some(true) && f.segs.iter().any(|seg| seg.capture == capture && seg.series == series && ok(&seg)) {
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

fn present(core: &Core, paths: &[PathBuf]) -> bool {
    paths.iter().any(|path| path.exists() || core.writing.lock().unwrap().contains(path))
}

pub fn missing(core: &Core, room: &str, f: &File, series: &str) -> Option<Run> {
    let segs = f.series(series);
    for (i, seg) in segs.iter().enumerate() {
        if init_check(core, room, f, &seg.capture, series) == Some(true) && seg_check(core, room, &f.owner, seg) == Some(true) {
            continue;
        }
        let lacks_init = !present(core, &core.init_paths(room, &f.set, &f.owner, &seg.capture, series));
        let paths = core.seg_paths(room, &f.owner, seg);
        let path = paths[0].clone();
        let here = present(core, &paths);
        if !lacks_init && here {
            continue;
        }
        let mut to = if here { seg.n - 1 } else { seg.n };
        for next in segs[i + 1..].iter().filter(|_| !here) {
            if next.capture != seg.capture || next.n != to + 1 || present(core, &core.seg_paths(room, &f.owner, next)) {
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

fn home(path: &Path) -> PathBuf {
    path.parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or_default()
}

pub fn adopt(core: &Core, set: &Set) {
    for f in set.files.iter().filter(|f| f.end_us.is_some()) {
        for seg in &f.segs {
            let [kept, live] = core.seg_paths(&set.room, &f.owner, seg);
            if kept.exists() || !live.exists() || seg_check(core, &set.room, &f.owner, seg) != Some(true) {
                continue;
            }
            let (kept_dir, live_dir) = (home(&kept), home(&live));
            if kept.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::rename(&live, &kept).is_ok() {
                mark(core, &kept_dir, &seg_file(seg), seg.size, true);
                unmark(core, &live_dir, &seg_file(seg));
            }
        }
        for capture in f.captures() {
            for series in SERIES {
                let [kept, live] = core.init_paths(&set.room, &set.id, &f.owner, capture, series);
                let Some((size, _)) = f.init(capture, series) else { continue };
                let moved = f.series(series).iter().any(|seg| seg.capture == capture && core.seg_paths(&set.room, &f.owner, seg)[0].exists());
                if !moved || kept.exists() || !live.exists() || init_check(core, &set.room, f, capture, series) != Some(true) {
                    continue;
                }
                if kept.parent().is_some_and(|d| std::fs::create_dir_all(d).is_ok()) && std::fs::copy(&live, &kept).is_ok() {
                    mark(core, &home(&kept), &format!("{series}/{INIT}"), size, true);
                }
            }
        }
    }
}

fn verify(core: &Core, set: &Set) {
    adopt(core, set);
    for f in &set.files {
        for series in SERIES {
            let key = room::have_key(&set.id, &f.owner, series);
            for (i, seg) in f.series(series).into_iter().enumerate() {
                let i = i as u64;
                let name = format!("{series}/{INIT}");
                let path = core.init_path(&set.room, &set.id, &f.owner, &seg.capture, series);
                let dir = home(&path);
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
                let path = core.seg_path(&set.room, &f.owner, seg);
                let dir = home(&path);
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
            let [kept, live] = core.seg_paths(&set.room, &f.owner, seg);
            for path in [kept, live.clone()] {
                let _ = std::fs::remove_file(part_of(&path));
                let _ = std::fs::remove_file(path);
            }
            unmark(core, &home(&live), &seg_file(seg));
        }
    }
    for f in &set.files {
        for capture in f.captures() {
            let live = core.capture_dir(&set.room, &f.owner, capture);
            if !holds(&live, "m4s") && !live.join("v").join(record::OPEN).exists() && !live.join("m").join(record::OPEN).exists() {
                let _ = std::fs::remove_dir_all(&live);
                core.jsons.lock().unwrap().remove(&live.join(CHECKS));
            }
        }
    }
    let dir = core.recording_dir(&set.room, &set.id);
    let _ = std::fs::remove_dir_all(&dir);
    core.jsons.lock().unwrap().retain(|path, _| !path.starts_with(&dir));
}

pub fn remove_room(core: &Core, room: &str) {
    let dir = core.dir.join("rooms").join(room);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => tracing::warn!("could not remove a room folder: {e}"),
        _ => {}
    }
    core.jsons.lock().unwrap().retain(|path, _| !path.starts_with(&dir));
    core.logs.lock().unwrap().retain(|path, _| !path.starts_with(&dir));
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
        let all = room::sets(core, &room, &room::logs(core, &room));
        let referenced = referenced_captures(&all);
        let mut stale: Vec<PathBuf> = Vec::new();
        for set in &all {
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
                let any_kept = referenced.contains(&(id.clone(), capture.clone()));
                let fresh = SERIES.iter().any(|series| changed_since(&dir.join(series), since) || dir.join(series).join(record::OPEN).exists());
                if !any_kept && !live && !fresh && !writing.iter().any(|w| w.starts_with(&dir)) {
                    stale.push(dir);
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
        drop_stale(core, stale);
    }
}

pub fn referenced_captures(sets: &[Set]) -> HashSet<(String, String)> {
    let mut out = HashSet::new();
    for f in sets.iter().flat_map(|s| &s.files) {
        for capture in f.captures() {
            out.insert((f.owner.clone(), capture.to_string()));
        }
    }
    out
}

pub fn verified_media(dir: &Path) -> bool {
    let all: Value = read_json(&dir.join(CHECKS));
    all.as_object().is_some_and(|map| map.iter().any(|(name, entry)| entry["ok"] == true && dir.join(name).exists()))
}

pub fn safe_to_remove(stale: &[PathBuf]) -> bool {
    stale.len() <= MAX_REMOVED && !stale.iter().any(|dir| verified_media(dir))
}

static KEPT_STALE: AtomicUsize = AtomicUsize::new(0);

fn drop_stale(core: &Core, stale: Vec<PathBuf>) {
    if stale.is_empty() {
        return;
    }
    if !safe_to_remove(&stale) {
        if KEPT_STALE.swap(stale.len(), Ordering::Relaxed) != stale.len() {
            debug::event(core, core.started, format!("kept {} capture folders that no recording refers to; a delete removes them", stale.len()));
        }
        return;
    }
    for dir in stale {
        if std::fs::remove_dir_all(&dir).is_ok() {
            core.jsons.lock().unwrap().remove(&dir.join(CHECKS));
            debug::event(core, core.started, format!("removed {}", dir.strip_prefix(&core.dir).unwrap_or(&dir).display()));
        }
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
            cleanup(&core);
            aged = Some(Instant::now());
        }
        let room = core.active_room_id();
        let logs = room::logs(&core, &room);
        room::sweep_dropped(&core, &room, &logs);
        for set in room::sets(&core, &room, &logs) {
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
    fn one_app_holds_a_data_folder_and_a_leftover_lock_is_noticed() {
        let dir = temp("lock");
        assert!(!lock(&dir).unwrap());
        assert!(std::fs::OpenOptions::new().write(true).open(dir.join(LOCK)).is_err());
        unlock(&dir);
        assert!(!dir.join(LOCK).exists());
        std::fs::write(dir.join(LOCK), b"").unwrap();
        assert!(lock(&dir).unwrap());
        unlock(&dir);
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
    fn a_reset_keeps_only_the_rooms_when_asked() {
        let files = ["key.bin", "rooms.json", "rooms/r/logs/a.bin", "settings.json", "ffmpeg.exe", "plugins/x/m.bin", "EBWebView/x", LOCK];
        for keep in [true, false] {
            let dir = temp(if keep { "resetkeep" } else { "resetall" });
            for f in files {
                let path = dir.join(f);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, b"x").unwrap();
            }
            reset(&dir, keep);
            let left: Vec<&str> = files.iter().copied().filter(|f| dir.join(f).exists()).collect();
            let kept: &[&str] = if keep { &["key.bin", "rooms.json", "rooms/r/logs/a.bin", "EBWebView/x", LOCK] } else { &["EBWebView/x", LOCK] };
            assert_eq!(left, kept);
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn hours_left_come_from_the_newest_sessions() {
        let hour = 3600e6 as i64;
        assert_eq!(bytes_per_hour(&[]), None);
        assert_eq!(bytes_per_hour(&[(0, hour), (100, 0)]), None);
        assert_eq!(bytes_per_hour(&[(3_000, hour), (1_000, hour)]), Some(2_000.0));
        let many: Vec<(u64, i64)> = (0..RATE_SETS + 3).map(|i| (if i < RATE_SETS { 10 } else { 1_000 }, hour)).collect();
        assert_eq!(bytes_per_hour(&many), Some(10.0));
        let disk = disk_json(20_000, 100_000, Some(2_000.0), WARN_HOURS, false);
        assert_eq!((disk["hours_left"].as_f64(), disk["low"].as_bool()), (Some(10.0), Some(false)));
        assert_eq!(disk_json(19_999, 100_000, Some(2_000.0), WARN_HOURS, false)["low"], true);
        assert_eq!(disk_json(1, 100_000, None, WARN_HOURS, false)["low"], false);
        assert_eq!(disk_json(1, 100_000, None, WARN_HOURS, true)["low"], true);
    }

    #[test]
    fn cleanup_keeps_set_segments_and_the_live_window() {
        let kept: HashSet<u32> = [2, 3].into();
        assert_eq!(doomed(&[1, 2, 3, 4, 5, 6, 7], &kept, None), [1, 4, 5, 6, 7]);
        assert_eq!(doomed(&[1, 2, 3, 4, 5, 6, 7], &kept, Some(6)), [1, 4, 5]);
        assert!(doomed(&[], &kept, Some(1)).is_empty());
    }

    #[test]
    fn old_layout_captures_stay_and_the_safety_net_holds() {
        let dir = temp("oldlayout");
        let touch = |p: &str, body: &str| {
            let path = dir.join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        };
        touch("a/1700000000000/v/init.mp4", "i");
        touch("a/1700000000000/v/000001.m4s", "s");
        touch("a/1700000000000/verified.json", r#"{"v/000001.m4s": {"size": 1, "ok": true}}"#);
        touch("a/1700000009999/v/000001.m4s", "x");
        let records = vec![
            json!({"k": "rec", "set": "s1", "start_us": 100}),
            json!({"k": "seg", "set": "s1", "capture": "1700000000000", "series": "v", "n": 1, "t_us": 100, "d_us": 10, "size": 1, "sha256": "ab"}),
            json!({"k": "rec_end", "set": "s1", "t_us": 900}),
        ];
        let logs = [room::Log { owner: "a".into(), offset_us: 0, records: Arc::new(records) }];
        let sets = room::derive_sets("r", &logs);
        let referenced = referenced_captures(&sets);
        assert!(referenced.contains(&("a".to_string(), "1700000000000".to_string())));
        assert!(!referenced.contains(&("a".to_string(), "1700000009999".to_string())));
        assert!(verified_media(&dir.join("a/1700000000000")));
        assert!(!verified_media(&dir.join("a/1700000009999")));
        assert!(safe_to_remove(&[dir.join("a/1700000009999")]));
        assert!(!safe_to_remove(&[dir.join("a/1700000000000")]));
        let many: Vec<PathBuf> = (0..=MAX_REMOVED).map(|i| dir.join(format!("none/{i}"))).collect();
        assert!(!safe_to_remove(&many));
        assert!(safe_to_remove(&many[..MAX_REMOVED]));
        let _ = std::fs::remove_dir_all(dir);
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

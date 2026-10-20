use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::room::{self, part_of, seg_name, File, Set, INIT, SERIES};
use crate::{debug, file_len, hex, log, Core};

pub const FULL_SHARE: f64 = 0.01;
pub const WARN_HOURS: u32 = 10;
pub const WARN_CHOICES: [u64; 5] = [2, 5, 10, 20, 50];
const RATE_SETS: usize = 5;
const KEEP_EVERY: Duration = Duration::from_secs(5);
const AGE_EVERY: Duration = Duration::from_secs(3600);
const OLD_AFTER: Duration = Duration::from_secs(7 * 86_400);
pub const LIVE_KEEP: u32 = 2 * crate::stream::LIVE_WINDOW + 1;

pub type Listing = Arc<HashMap<String, u64>>;
pub type Files = Mutex<HashMap<PathBuf, Listing>>;

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

fn older(path: &Path, now: SystemTime) -> bool {
    std::fs::metadata(path).and_then(|m| m.modified()).is_ok_and(|t| now.duration_since(t).is_ok_and(|age| age > OLD_AFTER))
}

pub fn leftovers(dir: &Path, exporting: bool, now: SystemTime) -> Vec<PathBuf> {
    let mut out = Vec::new();
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
    let exporting = core.export.lock().unwrap().is_some();
    for path in leftovers(&core.dir, exporting, SystemTime::now()) {
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
    forget(&core.files, &core.dir);
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

pub fn clear_standby(dir: &Path) {
    for room in std::fs::read_dir(dir.join("rooms")).into_iter().flatten().flatten() {
        let _ = std::fs::remove_dir_all(room.path().join(room::STANDBY));
    }
}

fn old_folders(dir: &Path) -> usize {
    let rooms = std::fs::read_dir(dir.join("rooms")).into_iter().flatten().flatten();
    rooms.flat_map(|room| std::fs::read_dir(room.path()).into_iter().flatten().flatten()).filter(|e| e.file_name().to_string_lossy().parse::<iroh::EndpointId>().is_ok() && e.path().is_dir()).count()
}

pub fn note_old_layout(core: &Core) {
    let old = old_folders(&core.dir);
    if old > 0 {
        debug::event(core, core.started, format!("{old} media folders of an older version are kept and not read"));
    }
}

fn in_standby(dir: &Path, depth: usize) -> bool {
    dir.ancestors().nth(depth).and_then(Path::file_name).is_some_and(|name| name == room::STANDBY)
}

fn ring_trim(names: &[String], n: u32) -> Vec<String> {
    let keep = n.saturating_sub(LIVE_KEEP - 1);
    names.iter().filter(|name| room::seg_number(name).is_some_and(|k| k < keep)).cloned().collect()
}

pub fn trim_ring(dir: &Path, n: u32) {
    if !in_standby(dir, 3) {
        return;
    }
    let names: Vec<String> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    for name in ring_trim(&names, n) {
        let _ = std::fs::remove_file(dir.join(name));
    }
}

pub fn keep_capture(dir: &Path) {
    let (Some(owner), Some(capture)) = (dir.parent(), dir.file_name()) else { return };
    if !in_standby(dir, 2) {
        return;
    }
    for entry in std::fs::read_dir(owner).into_iter().flatten().flatten() {
        if entry.file_name() != capture && room::capture_id(&entry.file_name().to_string_lossy()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
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

pub fn sha256(path: &Path) -> std::io::Result<(u64, String)> {
    let mut hasher = Sha256::new();
    let size = std::io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    Ok((size, hex(&hasher.finalize())))
}

pub fn matches(path: &Path, size: u64, sha: &str) -> std::io::Result<bool> {
    sha256(path).map(|(got_size, got)| got_size == size && got == sha)
}

fn names(dir: &Path) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let kept = name == INIT || room::seg_number(&name).is_some_and(|n| seg_name(n) == name);
        if let Some(meta) = entry.metadata().ok().filter(|m| kept && m.is_file()) {
            out.insert(name, meta.len());
        }
    }
    out
}

fn listing(files: &Files, dir: &Path) -> Listing {
    files.lock().unwrap().entry(dir.to_path_buf()).or_insert_with(|| Arc::new(names(dir))).clone()
}

pub fn forget(files: &Files, prefix: &Path) {
    files.lock().unwrap().retain(|dir, _| !dir.starts_with(prefix));
}

pub fn has(files: &Files, dir: &Path, name: &str, size: Option<u64>) -> bool {
    listing(files, dir).get(name).is_some_and(|len| size.is_none_or(|size| size == *len))
}

pub fn file_dir(core: &Core, room: &str, f: &File) -> PathBuf {
    core.recording_dir(room, &f.set).join(&f.owner)
}

pub fn presence(files: &Files, base: &Path, f: &File, series: &str) -> Vec<(bool, bool)> {
    let mut out = Vec::new();
    let mut at: Option<(&str, PathBuf, bool)> = None;
    for seg in f.series(series) {
        if at.as_ref().is_none_or(|(capture, ..)| *capture != seg.capture) {
            let dir = base.join(&seg.capture).join(series);
            let init = has(files, &dir, INIT, f.init(&seg.capture, series).map(|(size, _)| size));
            at = Some((seg.capture.as_str(), dir, init));
        }
        if let Some((_, dir, init)) = &at {
            out.push((has(files, dir, &seg_name(seg.n), Some(seg.size)), *init));
        }
    }
    out
}

pub fn prefix(oks: impl IntoIterator<Item = bool>) -> u64 {
    oks.into_iter().take_while(|ok| *ok).count() as u64
}

fn count_in(files: &Files, base: &Path, f: &File, series: &str) -> u64 {
    prefix(presence(files, base, f, series).into_iter().map(|(seg, init)| seg && init))
}

pub fn count(core: &Core, room: &str, f: &File, series: &str) -> u64 {
    count_in(&core.files, &file_dir(core, room, f), f, series)
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

fn held_in(files: &Files, base: &Path, f: &File) -> u64 {
    let mut have = 0;
    for series in SERIES {
        let mut inits: Vec<&str> = Vec::new();
        for (seg, (here, init)) in f.series(series).into_iter().zip(presence(files, base, f, series)) {
            if !here {
                continue;
            }
            have += seg.size;
            if init && !inits.contains(&seg.capture.as_str()) {
                inits.push(&seg.capture);
                have += f.init(&seg.capture, series).map_or(0, |(size, _)| size);
            }
        }
    }
    have
}

pub fn held(core: &Core, room: &str, f: &File) -> (u64, u64) {
    (held_in(&core.files, &file_dir(core, room, f), f), f.total())
}

pub struct Run {
    pub capture: String,
    pub init: bool,
    pub from: u32,
    pub at: u64,
    pub to: u32,
    pub index: u64,
}

fn missing_in(files: &Files, writing: &Mutex<HashSet<PathBuf>>, base: &Path, f: &File, series: &str) -> Option<Run> {
    let segs = f.series(series);
    let found = presence(files, base, f, series);
    let dir = |capture: &str| base.join(capture).join(series);
    let busy = |path: PathBuf| writing.lock().unwrap().contains(&path);
    let here = |i: usize| found[i].0 || busy(dir(&segs[i].capture).join(seg_name(segs[i].n)));
    for (i, seg) in segs.iter().enumerate() {
        let init = found[i].1 || busy(dir(&seg.capture).join(INIT));
        let present = here(i);
        if init && present {
            continue;
        }
        let mut to = if present { seg.n - 1 } else { seg.n };
        for j in (i + 1..segs.len()).filter(|_| !present) {
            if segs[j].capture != seg.capture || segs[j].n != to + 1 || here(j) {
                break;
            }
            to = segs[j].n;
        }
        let at = if present { 0 } else { file_len(&part_of(&dir(&seg.capture).join(seg_name(seg.n)))).min(seg.size) };
        return Some(Run { capture: seg.capture.clone(), init: !init, from: seg.n, at, to, index: i as u64 });
    }
    None
}

pub fn missing(core: &Core, room: &str, f: &File, series: &str) -> Option<Run> {
    missing_in(&core.files, &core.writing, &file_dir(core, room, f), f, series)
}

pub fn expected(f: &File, run: &Run, series: &str) -> HashMap<u32, (u64, String)> {
    let mut out: HashMap<u32, (u64, String)> = f.series(series).into_iter().filter(|s| s.capture == run.capture && (run.from..=run.to).contains(&s.n)).map(|s| (s.n, (s.size, s.sha256.clone()))).collect();
    if let Some(init) = f.init(&run.capture, series) {
        out.insert(0, init);
    }
    out
}

pub fn remove_room(core: &Core, room: &str) {
    let dir = core.room_dir(room);
    match std::fs::remove_dir_all(&dir) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => tracing::warn!("could not remove a room folder: {e}"),
        _ => {}
    }
    forget(&core.files, &dir);
    core.logs.lock().unwrap().retain(|path, _| !path.starts_with(&dir));
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
        room::sweep_dropped(&core, &room, &room::logs(&core, &room));
        std::thread::sleep(KEEP_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::tests::fragment;
    use std::collections::{BTreeMap, BTreeSet};

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ezh-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, name: &str, body: &[u8]) {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn snapshot(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(at) = stack.pop() {
            for entry in std::fs::read_dir(&at).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    out.insert(path.strip_prefix(dir).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(&path).unwrap());
                }
            }
        }
        out
    }

    fn file(records: Vec<Value>) -> File {
        let logs = [room::Log { owner: "a".into(), offset_us: 0, records: Arc::new(records) }];
        room::derive_sets("r", &logs).remove(0).files.remove(0)
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
        for p in ["rooms/active/logs/a.bin", "rooms/other/b/1/v/000001.m4s", "rooms/gone/b/1/v/verified.json", "rooms/gone/logs/a.bin", "rooms/left/recordings/s1/b/1/v/000001.m4s", "rooms/waiting/pending.bin", "export/label-0.txt", "report.txt", "key.bin", "rooms/solo/b/1/v/verified.json", "rooms/old/sets/s1/set.json", "rooms/old/sets/s1/a.mp4", "rooms/oldlive/live/a.mp4", "rec-1.mp4", "log.bin", "peer.txt", "peers.txt", "peer/a/log.bin", "legacy/x.bin"] {
            put(&dir, p, b"x");
        }
        let later = SystemTime::now() + OLD_AFTER + Duration::from_secs(60);
        let mut got: Vec<String> = leftovers(&dir, false, later).iter().map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/")).collect();
        got.sort();
        assert_eq!(got, ["export", "report.txt"]);
        assert!(leftovers(&dir, true, SystemTime::now()).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_reset_keeps_only_the_rooms_when_asked() {
        let files = ["key.bin", "rooms.json", "rooms/r/logs/a.bin", "settings.json", "ffmpeg.exe", "plugins/x/m.bin", "EBWebView/x", LOCK];
        for keep in [true, false] {
            let dir = temp(if keep { "resetkeep" } else { "resetall" });
            for f in files {
                put(&dir, f, b"x");
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
    fn a_restart_removes_nothing_under_recordings_and_never_touches_old_folders() {
        let dir = temp("restart");
        let (me, other) = (iroh::SecretKey::generate().public().to_string(), iroh::SecretKey::generate().public().to_string());
        let (one, two, three) = (fragment(&[(1, 0)], b"aaaa"), fragment(&[(1, 1000)], b"bbbb"), fragment(&[(1, 2000)], b"cccc"));
        let torn = [one.clone(), two.clone(), three[..three.len() / 2].to_vec()].concat();
        let files = [
            (format!("rooms/r/recordings/s1/{me}/1000/v/init.mp4"), b"init v".to_vec()),
            (format!("rooms/r/recordings/s1/{me}/1000/v/000001.m4s"), one.clone()),
            (format!("rooms/r/recordings/s1/{me}/1000/v/000002.m4s"), two.clone()),
            (format!("rooms/r/recordings/s1/{me}/1000/m/init.mp4"), b"init m".to_vec()),
            (format!("rooms/r/recordings/s1/{me}/1000/m/000001.m4s"), one.clone()),
            (format!("rooms/r/recordings/s1/{me}/1000/verified.json"), br#"{"v/000001.m4s": {"size": 1, "ok": false}}"#.to_vec()),
            (format!("rooms/r/recordings/s1/{other}/2000/v/init.mp4"), b"init".to_vec()),
            (format!("rooms/r/recordings/s1/{other}/2000/v/000001.m4s"), one.clone()),
            (format!("rooms/r/recordings/s1/{other}/2000/v/000002.m4s.part"), torn.clone()),
            (format!("rooms/r/recordings/s2/{me}/3000/v/init.mp4"), b"init v".to_vec()),
            (format!("rooms/r/recordings/s2/{me}/3000/v/000001.m4s"), one.clone()),
            (format!("rooms/r/recordings/s2/{me}/3000/v/000002.m4s.part"), torn.clone()),
            (format!("rooms/r/recordings/s2/{me}/3000/m/init.mp4"), b"init m".to_vec()),
            (format!("rooms/r/recordings/s2/{me}/3000/m/000001.m4s.part"), two.clone()),
            (format!("rooms/r/recordings/x9/{other}/7000/v/000001.m4s"), one.clone()),
            (format!("rooms/r/recordings/s3/{me}/4000/v/init.mp4"), b"init v".to_vec()),
            (format!("rooms/r/recordings/s3/{me}/4000/v/000001.m4s"), one.clone()),
            ("rooms/r/thumbs/s1.jpg".to_string(), b"jpg".to_vec()),
            ("rooms/r/thumbs/s3.jpg".to_string(), b"jpg".to_vec()),
            (format!("rooms/r/standby/{me}/5000/v/init.mp4"), b"init".to_vec()),
            (format!("rooms/r/standby/{me}/5000/v/000009.m4s"), one.clone()),
            (format!("rooms/r/standby/{other}/6000/m/000004.m4s.part"), torn.clone()),
            (format!("rooms/r/{me}/1000/v/init.mp4"), b"init v".to_vec()),
            (format!("rooms/r/{me}/1000/v/000001.m4s"), one.clone()),
            (format!("rooms/r/{me}/1000/v/000003.m4s.part"), torn.clone()),
            (format!("rooms/r/{me}/1000/v/open.json"), br#"{"n": 3, "set": "s1"}"#.to_vec()),
            (format!("rooms/r/{me}/1000/cap.json"), br#"{"start_us": 1}"#.to_vec()),
            (format!("rooms/r/{me}/1000/verified.json"), br#"{"v/000001.m4s": {"size": 1, "ok": true}}"#.to_vec()),
            (format!("rooms/r/{other}/8000/m/000001.m4s"), one.clone()),
            ("rooms/r/sets/s1/a.mp4".to_string(), b"old".to_vec()),
            ("rooms/r/live/someone.mp4".to_string(), b"old".to_vec()),
            (format!("rooms/r/logs/{me}.bin"), b"log".to_vec()),
            ("rooms/gone/recordings/s5/x/1/v/000001.m4s".to_string(), one.clone()),
            ("export/x".to_string(), b"partial".to_vec()),
            ("report.txt".to_string(), b"report".to_vec()),
        ];
        for (name, body) in &files {
            put(&dir, name, body);
        }
        let before = snapshot(&dir);
        let joined: HashSet<&str> = ["s1", "s2", "s3"].into();
        let logged: HashSet<(&str, &str, u32)> = [("1000", "v", 1), ("1000", "v", 2), ("1000", "m", 1), ("3000", "v", 1), ("3000", "m", 1), ("4000", "v", 1)].into();
        let found = crate::record::recover_files(&dir, &me, |r| !joined.contains(r.set.as_str()) || logged.contains(&(r.capture.as_str(), r.series, r.n)));
        clear_standby(&dir);
        assert_eq!(old_folders(&dir), 2);
        for path in leftovers(&dir, false, SystemTime::now() + OLD_AFTER + Duration::from_secs(60)) {
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
        assert_eq!(room::sweep(&dir.join("rooms/r"), &["s3".to_string()].into(), None), ["s3"]);
        let after = snapshot(&dir);
        assert_eq!(found.iter().map(|r| (r.room.as_str(), r.set.as_str(), r.capture.as_str(), r.series, r.n)).collect::<Vec<_>>(), [("r", "s2", "3000", "v", 2)]);
        let recovered = format!("rooms/r/recordings/s2/{me}/3000/v/000002.m4s");
        let gone: Vec<&str> = before.keys().filter(|k| !after.contains_key(*k)).map(String::as_str).collect();
        let mut expected = vec![
            format!("rooms/r/recordings/s2/{me}/3000/v/000002.m4s.part"),
            format!("rooms/r/recordings/s3/{me}/4000/v/000001.m4s"),
            format!("rooms/r/recordings/s3/{me}/4000/v/init.mp4"),
            format!("rooms/r/standby/{me}/5000/v/000009.m4s"),
            format!("rooms/r/standby/{me}/5000/v/init.mp4"),
            format!("rooms/r/standby/{other}/6000/m/000004.m4s.part"),
            "export/x".to_string(),
            "report.txt".to_string(),
            "rooms/r/thumbs/s3.jpg".to_string(),
        ];
        expected.sort();
        assert_eq!(gone, expected);
        assert_eq!(after.keys().filter(|k| !before.contains_key(*k)).collect::<Vec<_>>(), [&recovered]);
        assert_eq!(after[&recovered], [one, two].concat());
        assert!(after.iter().filter(|(k, _)| **k != recovered).all(|(k, body)| before[k] == *body));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn counts_come_from_the_files_present_with_their_logged_size() {
        let dir = temp("counts");
        let base = dir.join("rooms/r/recordings/s1/a");
        let series = base.join("1000/v");
        let mut records = vec![json!({"k": "cap", "capture": "1000", "start_us": 0, "init": {"v": {"size": 4, "sha256": "i"}}}), json!({"k": "rec", "set": "s1", "start_us": 0})];
        records.extend((1..=5).map(|n| json!({"k": "seg", "capture": "1000", "series": "v", "n": n, "set": "s1", "t_us": n * 4, "d_us": 4, "size": 4, "sha256": format!("s{n}")})));
        let f = file(records);
        put(&series, INIT, b"init");
        for n in [1, 2, 3, 5] {
            put(&series, &seg_name(n), b"seg!");
        }
        put(&base, "1000/verified.json", br#"{"v/000004.m4s": {"size": 4, "ok": true}}"#);
        put(&dir, "rooms/r/a/1000/v/000004.m4s", b"seg!");
        let (files, writing) = (Files::default(), Mutex::new(HashSet::new()));
        let run = |files: &Files| missing_in(files, &writing, &base, &f, "v").map(|r| (r.from, r.to, r.at, r.init, r.index));
        assert_eq!(count_in(&files, &base, &f, "v"), 3);
        assert_eq!(held_in(&files, &base, &f), 4 * 4 + 4);
        assert_eq!(run(&files), Some((4, 4, 0, false, 3)));
        put(&series, "000004.m4s.part", b"se");
        assert_eq!(run(&files), Some((4, 4, 2, false, 3)));
        put(&series, "000004.m4s.part", b"seg!more");
        assert_eq!(run(&files), Some((4, 4, 4, false, 3)));
        writing.lock().unwrap().insert(series.join("000004.m4s"));
        assert_eq!(run(&files), None);
        assert_eq!(count_in(&files, &base, &f, "v"), 3);
        writing.lock().unwrap().clear();
        put(&series, "000004.m4s", b"seg!");
        assert_eq!(count_in(&files, &base, &f, "v"), 3);
        forget(&files, &series);
        assert_eq!(count_in(&files, &base, &f, "v"), 5);
        put(&series, "000002.m4s", b"bad");
        forget(&files, &base);
        assert_eq!(count_in(&files, &base, &f, "v"), 1);
        assert_eq!(held_in(&files, &base, &f), 4 * 4 + 4);
        assert_eq!(run(&files), Some((2, 2, 0, false, 1)));
        std::fs::remove_file(series.join(INIT)).unwrap();
        forget(&files, &dir);
        assert_eq!(count_in(&files, &base, &f, "v"), 0);
        assert_eq!(held_in(&files, &base, &f), 4 * 4);
        assert_eq!(run(&files), Some((1, 0, 0, true, 0)));
        let fetch = Run { capture: "1000".into(), init: true, from: 2, at: 0, to: 3, index: 1 };
        let want = expected(&f, &fetch, "v");
        assert_eq!(want.keys().copied().collect::<BTreeSet<_>>(), [0, 2, 3].into());
        assert_eq!((want[&0].clone(), want[&3].clone()), ((4, "i".to_string()), (4, "s3".to_string())));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_is_here_under_its_final_name_with_its_logged_size() {
        let dir = temp("has");
        for (name, body) in [(INIT, b"init".as_slice()), ("000001.m4s", b"seg!"), ("000002.m4s.part", b"seg!"), ("verified.json", b"{}")] {
            put(&dir, name, body);
        }
        let files = Files::default();
        assert!(has(&files, &dir, INIT, Some(4)) && has(&files, &dir, INIT, None) && !has(&files, &dir, INIT, Some(5)));
        assert!(has(&files, &dir, "000001.m4s", Some(4)) && !has(&files, &dir, "000001.m4s", Some(3)));
        assert!(!has(&files, &dir, "000002.m4s", Some(4)) && !has(&files, &dir, "000002.m4s.part", None) && !has(&files, &dir, "verified.json", None));
        put(&dir, "000002.m4s", b"seg!");
        assert!(!has(&files, &dir, "000002.m4s", Some(4)));
        forget(&files, &dir);
        assert!(has(&files, &dir, "000002.m4s", Some(4)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_standby_ring_keeps_the_newest_segments_and_only_standby_is_trimmed() {
        let names: Vec<String> = ["init.mp4", "000001.m4s", "000002.m4s.part", "000003.m4s", "000004.m4s", "000009.m4s", "000010.m4s.part"].map(String::from).to_vec();
        assert_eq!(ring_trim(&names, 10), ["000001.m4s", "000002.m4s.part", "000003.m4s"]);
        assert!(ring_trim(&names, LIVE_KEEP).is_empty());
        let dir = temp("ring");
        let ring = dir.join("rooms/r/standby/a/1000/v");
        let kept = dir.join("rooms/r/recordings/s1/a/1000/v");
        for d in [&ring, &kept] {
            put(d, INIT, b"i");
            for n in 1..=10 {
                put(d, &seg_name(n), b"x");
            }
        }
        trim_ring(&ring, 10);
        trim_ring(&kept, 10);
        let left = |d: &Path| -> BTreeSet<String> { std::fs::read_dir(d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect() };
        assert_eq!(left(&ring), [INIT.to_string()].into_iter().chain((4..=10).map(seg_name)).collect());
        assert_eq!(left(&kept).len(), 11);
        put(&dir, "rooms/r/standby/a/2000/v/init.mp4", b"i");
        put(&dir, "rooms/r/standby/b/1000/v/init.mp4", b"i");
        keep_capture(&dir.join("rooms/r/standby/a/2000"));
        assert!(!dir.join("rooms/r/standby/a/1000").exists());
        assert!(dir.join("rooms/r/standby/a/2000/v/init.mp4").exists() && dir.join("rooms/r/standby/b/1000/v/init.mp4").exists());
        keep_capture(&dir.join("rooms/r/recordings/s1/a/2000"));
        assert_eq!(left(&kept).len(), 11);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn copies_are_checked_by_size_and_hash_and_counted_in_order() {
        let dir = temp("verify");
        let path = dir.join("000001.m4s");
        std::fs::write(&path, b"fragment").unwrap();
        let (size, sha) = sha256(&path).unwrap();
        assert!(matches(&path, size, &sha).unwrap());
        assert!(!matches(&path, size + 1, &sha).unwrap());
        std::fs::write(&path, b"fragmenT").unwrap();
        assert!(!matches(&path, size, &sha).unwrap());
        assert!(matches(&dir.join("missing.m4s"), size, &sha).is_err());
        assert_eq!(prefix([true, true, false, true]), 2);
        assert_eq!(prefix([false, true]), 0);
        let _ = std::fs::remove_dir_all(dir);
    }
}

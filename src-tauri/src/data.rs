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
    for (_, room) in crate::entries(&dir.join("rooms")) {
        log::repair(&room.join("pending.bin"));
        for (_, path) in crate::entries(&room.join("logs")) {
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
    for (_, room) in crate::entries(&dir.join("rooms")) {
        let _ = std::fs::remove_dir_all(room.join(room::STANDBY));
    }
}

fn old_folders(dir: &Path) -> usize {
    let rooms = crate::entries(&dir.join("rooms"));
    rooms.iter().flat_map(|(_, room)| crate::entries(room)).filter(|(name, path)| name.parse::<iroh::EndpointId>().is_ok() && path.is_dir()).count()
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

pub fn trim_ring(dir: &Path, n: u32) {
    let keep = n.saturating_sub(LIVE_KEEP - 1);
    for (name, path) in crate::entries(dir).into_iter().filter(|_| in_standby(dir, 3)) {
        if room::seg_number(&name).is_some_and(|k| k < keep) {
            let _ = std::fs::remove_file(path);
        }
    }
}

pub fn keep_capture(dir: &Path) {
    let (Some(owner), Some(capture)) = (dir.ancestors().nth(2), dir.parent().and_then(Path::file_name)) else { return };
    if !in_standby(dir, 3) {
        return;
    }
    for (name, path) in crate::entries(owner) {
        if name != capture.to_string_lossy() && room::capture_id(&name) {
            let _ = std::fs::remove_dir_all(path);
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
    let rooms = ["key.bin", "rooms.json", "rooms"];
    for (name, path) in crate::entries(dir) {
        if name == "EBWebView" || name == "webview" || name == LOCK || (keep_rooms && rooms.contains(&name.as_str())) {
            continue;
        }
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

pub fn seg_dir(room_dir: &Path, f: &File, capture: &str, series: &str) -> PathBuf {
    room::series_dir(room_dir, Some(&f.set), &f.owner, capture, series)
}

pub fn presence(files: &Files, room_dir: &Path, f: &File, series: &str) -> Vec<(bool, bool)> {
    let here = |seg: &&room::Seg| {
        let dir = seg_dir(room_dir, f, &seg.capture, series);
        (has(files, &dir, &seg_name(seg.n), Some(seg.size)), has(files, &dir, INIT, f.init(&seg.capture, series).map(|(size, _)| size)))
    };
    f.series(series).iter().map(here).collect()
}

fn count(files: &Files, room_dir: &Path, f: &File, series: &str) -> u64 {
    presence(files, room_dir, f, series).into_iter().take_while(|(seg, init)| *seg && *init).count() as u64
}

pub fn have_counts(core: &Core, room: &str, sets: &[Set]) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    for set in sets {
        for f in &set.files {
            for series in SERIES {
                let n = count(&core.files, &core.room_dir(room), f, series);
                if n > 0 {
                    out.insert(room::have_key(&set.id, &f.owner, series), n);
                }
            }
        }
    }
    out
}

fn held_in(files: &Files, room_dir: &Path, f: &File) -> u64 {
    let mut have = 0;
    for series in SERIES {
        let mut inits = HashSet::new();
        for (seg, (_, init)) in f.series(series).into_iter().zip(presence(files, room_dir, f, series)).filter(|(_, (here, _))| *here) {
            have += seg.size;
            if init && inits.insert(&seg.capture) {
                have += f.init(&seg.capture, series).map_or(0, |(size, _)| size);
            }
        }
    }
    have
}

pub fn held(core: &Core, room: &str, f: &File) -> (u64, u64) {
    (held_in(&core.files, &core.room_dir(room), f), f.total())
}

pub struct Run {
    pub capture: String,
    pub init: bool,
    pub from: u32,
    pub at: u64,
    pub to: u32,
    pub index: u64,
}

pub fn missing(files: &Files, writing: &Mutex<HashSet<PathBuf>>, room_dir: &Path, f: &File, series: &str) -> Option<Run> {
    let segs = f.series(series);
    let found = presence(files, room_dir, f, series);
    let dir = |capture: &str| seg_dir(room_dir, f, capture, series);
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

pub fn expected(f: &File, run: &Run, series: &str) -> HashMap<u32, (u64, String)> {
    let mut out: HashMap<u32, (u64, String)> = f.series(series).into_iter().filter(|s| s.capture == run.capture && (run.from..=run.to).contains(&s.n)).map(|s| (s.n, (s.size, s.sha256.clone()))).collect();
    if let Some(init) = f.init(&run.capture, series) {
        out.insert(0, init);
    }
    out
}

pub fn remove_room(core: &Core, room: &str) {
    let dir = core.room_dir(room);
    let _ = std::fs::remove_dir_all(&dir);
    forget(&core.files, &dir);
    core.logs.lock().unwrap().retain(|path, _| !path.starts_with(&dir));
}

pub fn keeper(core: Arc<Core>) {
    let mut aged: Option<Instant> = None;
    loop {
        if core.disk_full() && core.active_set.lock().unwrap().is_some() {
            room::end_set(&core, false, None);
            *core.notice.lock().unwrap() = "Recording stopped: the disk is almost full.".into();
        }
        recreate(&core);
        if aged.is_none_or(|t| t.elapsed() > AGE_EVERY) {
            cleanup(&core);
            aged = Some(Instant::now());
        }
        let room = core.active_room_id();
        room::sweep_dropped(&core, &room, &room::logs(&core, &room));
        room::sweep_removed(&core);
        std::thread::sleep(KEEP_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::tests::{present, put, seed, temp, tree};
    use crate::media::tests::fragment;

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
    fn a_reset_keeps_only_the_rooms_when_asked() {
        let files = ["key.bin", "rooms.json", "rooms/r/logs/a.bin", "settings.json", "ffmpeg.exe", "plugins/x/m.bin", "EBWebView/x", LOCK];
        for keep in [true, false] {
            let dir = temp(if keep { "resetkeep" } else { "resetall" });
            seed(&dir, files);
            reset(&dir, keep);
            let kept: &[&str] = if keep { &["key.bin", "rooms.json", "rooms/r/logs/a.bin", "EBWebView/x", LOCK] } else { &["EBWebView/x", LOCK] };
            assert_eq!(present(&dir, &files), kept);
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
        let (one, two) = (fragment(&[(1, 0)], b"aaaa"), fragment(&[(1, 1000)], b"bbbb"));
        let torn = [one.clone(), two.clone(), b"torn".to_vec()].concat();
        let name = |p: &str| p.replace("ME", &me).replace("OT", &other);
        let gone = ["rooms/r/recordings/s2/ME/3000/v/000002.m4s.part", "rooms/r/recordings/s3/ME/4000/v/init.mp4", "rooms/r/recordings/s3/ME/4000/v/000001.m4s", "rooms/r/thumbs/s3.jpg", "rooms/r/standby/ME/5000/v/init.mp4", "rooms/r/standby/ME/5000/v/000009.m4s", "rooms/r/standby/OT/6000/m/000004.m4s.part", "export/x", "report.txt"];
        let kept = ["rooms/r/recordings/s1/ME/1000/v/init.mp4", "rooms/r/recordings/s1/ME/1000/v/000001.m4s", "rooms/r/recordings/s1/ME/1000/v/000002.m4s.part", "rooms/r/recordings/s1/ME/1000/verified.json", "rooms/r/recordings/s1/OT/2000/v/000001.m4s.part", "rooms/r/recordings/s2/ME/3000/v/init.mp4", "rooms/r/recordings/s2/ME/3000/m/000001.m4s.part", "rooms/r/recordings/x9/ME/7000/v/000001.m4s.part", "rooms/r/thumbs/s1.jpg", "rooms/r/ME/1000/v/000003.m4s.part", "rooms/r/ME/1000/v/open.json", "rooms/r/ME/1000/verified.json", "rooms/r/OT/8000/m/000001.m4s", "rooms/r/sets/s1/a.mp4", "rooms/r/live/a.mp4", "rooms/r/logs/ME.bin", "rooms/gone/recordings/s5/OT/1/v/000001.m4s"];
        for p in gone.iter().chain(&kept) {
            put(&dir, &name(p), if p.ends_with(".part") { &torn } else { &one });
        }
        let mut before = tree(&dir);
        let (joined, logged) = (["s1", "s2", "s3"], [("1000", "v", 1), ("1000", "v", 2), ("3000", "m", 1), ("4000", "v", 1)]);
        let found = crate::record::recover_files(&dir, &me, |r| !joined.contains(&r.set.as_str()) || logged.contains(&(r.capture.as_str(), r.series, r.n)));
        clear_standby(&dir);
        assert_eq!(old_folders(&dir), 2);
        assert!(leftovers(&dir, true, SystemTime::now()).is_empty());
        for path in leftovers(&dir, false, SystemTime::now() + OLD_AFTER + Duration::from_secs(60)) {
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
        assert_eq!(room::sweep(&dir.join("rooms/r"), &["s3".to_string()].into(), None), ["s3"]);
        assert_eq!(found.iter().map(|r| (r.room.as_str(), r.set.as_str(), r.capture.as_str(), r.series, r.n)).collect::<Vec<_>>(), [("r", "s2", "3000", "v", 2)]);
        for p in gone {
            before.remove(&name(p));
        }
        before.insert(name("rooms/r/recordings/s2/ME/3000/v/000002.m4s"), [one, two].concat());
        assert_eq!(tree(&dir), before);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn counts_come_from_the_files_present_with_their_logged_size() {
        let dir = temp("counts");
        let (room_dir, f) = (dir.join("rooms/r"), room::tests::small_file(5, false));
        let series = seg_dir(&room_dir, &f, "1000", "v");
        for name in [INIT, "000001.m4s", "000002.m4s", "000003.m4s", "000005.m4s"] {
            put(&series, name, b"seg!");
        }
        put(&room_dir, "recordings/s1/a/1000/verified.json", br#"{"v/000004.m4s": {"size": 4, "ok": true}}"#);
        put(&dir, "rooms/r/a/1000/v/000004.m4s", b"seg!");
        let (files, writing) = (Files::default(), Mutex::new(HashSet::new()));
        let run = |files: &Files| missing(files, &writing, &room_dir, &f, "v").map(|r| (r.from, r.to, r.at, r.init, r.index));
        let state = |files: &Files| (count(files, &room_dir, &f, "v"), held_in(files, &room_dir, &f), run(files));
        assert_eq!(state(&files), (3, 4 * 4 + 4, Some((4, 4, 0, false, 3))));
        put(&series, "000004.m4s.part", b"se");
        assert_eq!(run(&files), Some((4, 4, 2, false, 3)));
        put(&series, "000004.m4s.part", b"seg!more");
        assert_eq!(run(&files), Some((4, 4, 4, false, 3)));
        writing.lock().unwrap().insert(series.join("000004.m4s"));
        assert_eq!(run(&files), None);
        writing.lock().unwrap().clear();
        put(&series, "000004.m4s", b"seg!");
        assert_eq!(count(&files, &room_dir, &f, "v"), 3);
        forget(&files, &series);
        assert_eq!(count(&files, &room_dir, &f, "v"), 5);
        put(&series, "000002.m4s", b"bad");
        forget(&files, &room_dir);
        assert_eq!(state(&files), (1, 4 * 4 + 4, Some((2, 2, 0, false, 1))));
        std::fs::remove_file(series.join(INIT)).unwrap();
        forget(&files, &dir);
        assert_eq!(state(&files), (0, 4 * 4, Some((1, 0, 0, true, 0))));
        let want = expected(&f, &Run { capture: "1000".into(), init: true, from: 2, at: 0, to: 3, index: 1 }, "v");
        assert_eq!(want, [(0, (4, "i".to_string())), (2, (4, "s2".to_string())), (3, (4, "s3".to_string()))].into());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_standby_ring_keeps_the_newest_segments_and_only_standby_is_trimmed() {
        let dir = temp("ring");
        let (ring, kept) = ("rooms/r/standby/a/1000/v", "rooms/r/recordings/s1/a/1000/v");
        seed(&dir, ["rooms/r/standby/a/2000/v/init.mp4", "rooms/r/standby/b/1000/v/init.mp4"]);
        seed(&dir.join(ring), (1..=10).map(seg_name).chain([INIT.into(), format!("{}.part", seg_name(3))]));
        seed(&dir.join(kept), (1..=10).map(seg_name));
        trim_ring(&dir.join(ring), 10);
        trim_ring(&dir.join(kept), 10);
        assert_eq!(tree(&dir.join(ring)).into_keys().collect::<Vec<_>>(), (4..=10).map(seg_name).chain([INIT.into()]).collect::<Vec<_>>());
        assert_eq!(tree(&dir.join(kept)).len(), 10);
        keep_capture(&dir.join("rooms/r/standby/a/2000/v"));
        keep_capture(&dir.join("rooms/r/recordings/s1/a/2000/v"));
        let dirs = ["rooms/r/standby/a/1000", "rooms/r/standby/a/2000", "rooms/r/standby/b/1000", kept];
        assert_eq!(present(&dir, &dirs), &dirs[1..]);
        let _ = std::fs::remove_dir_all(dir);
    }
}

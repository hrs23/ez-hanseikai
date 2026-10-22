use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use serde_json::{json, Value};
use windows::core::{Ref, Result as WinResult, HRESULT, PCWSTR};
use windows::Win32::UI::Shell::{IFileOperationProgressSink, IFileOperationProgressSink_Impl, IShellItem};

use crate::room::{self, seg_name, INIT, LOGS, PENDING, SERIES};
use crate::{data, debug, jobs, log, Core};

pub const MIN_AGE: Duration = Duration::from_secs(86_400);
const EXAMPLES: usize = 5;
const BUSY: &str = "Try again when no recording, export or analysis is running.";
const NEVER: [&str; 12] = ["key.bin", "rooms.json", "settings.json", "app.lock", "EBWebView", "crash.txt", "log.txt", "log.old.txt", "log.older.txt", "export", "ffmpeg.exe", "api.json"];
const WHOLE: [&str; 4] = ["rooms.json", "settings.json", "resume.json", "encoders.json"];
const THUMB: &str = ".16x9.jpg";
const OLD_ROOT: [&str; 5] = ["log.bin", "peer", "peer.txt", "peers.txt", "running.marker"];
const OLD_ROOM: [&str; 2] = ["sets", "live"];
const OLD_CAPTURE: [&str; 2] = ["verified.json", "cap.json"];

static SHOWN: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Way {
    Bin,
    Delete,
}

pub struct Spot<'a> {
    dir: &'a Path,
    parts: &'a [&'a str],
    refs: Option<&'a Refs>,
}

pub struct Rule {
    pub what: &'static str,
    pub way: Way,
    test: fn(&Spot) -> bool,
}

pub const RULES: &[Rule] = &[
    Rule { what: "unfinished thumbnail: rooms/<room>/thumbs/*.tmp.*", way: Way::Delete, test: |s| matches!(s.parts, ["rooms", _, "thumbs", name] if name.contains(".tmp.")) },
    Rule { what: "unfinished write beside its finished file: rooms.json.part, settings.json.part, resume.json.part, encoders.json.part", way: Way::Delete, test: |s| matches!(s.parts, [name] if name.strip_suffix(".part").is_some_and(|whole| WHOLE.contains(&whole) && s.dir.join(whole).is_file())) },
    Rule { what: "unfinished ffmpeg download: ffmpeg.exe.part, ffmpeg*.zip.part", way: Way::Delete, test: |s| matches!(s.parts, [name] if *name == "ffmpeg.exe.part" || name.starts_with("ffmpeg") && name.ends_with(".zip.part")) },
    Rule { what: "thumbnail of a recording no log names: rooms/<room>/thumbs/<set>.16x9.jpg", way: Way::Bin, test: |s| matches!(s.parts, ["rooms", _, "thumbs", name] if name.strip_suffix(THUMB).is_some_and(|set| room::valid_id(set) && s.refs.is_some_and(|r| !r.sets.contains(set)))) },
    Rule { what: "recording media no log names: rooms/<room>/recordings/<set>/<member>/<capture>/<v|m>/<init.mp4|NNNNNN.m4s>[.part]", way: Way::Bin, test: unlogged_media },
    Rule { what: "live piece of an earlier run: rooms/<room>/standby/<member>/<capture>/<v|m>/<init.mp4|NNNNNN.m4s>[.part]", way: Way::Bin, test: |s| matches!(s.parts, ["rooms", _, "standby", owner, capture, series, name] if media(owner, capture, series, name).is_some()) },
    Rule { what: "older version files: log.bin, peer/, peer.txt, peers.txt, running.marker", way: Way::Bin, test: |s| s.parts.first().is_some_and(|name| OLD_ROOT.contains(name)) },
    Rule { what: "extension work of older versions, such as transcription: plugins/", way: Way::Bin, test: |s| matches!(s.parts, ["plugins", _, ..]) },
    Rule { what: "recording of the first versions: rec-<digits>.mp4", way: Way::Bin, test: |s| matches!(s.parts, [name] if name.strip_prefix("rec-").and_then(|rest| rest.strip_suffix(".mp4")).is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))) },
    Rule { what: "room folders of older versions: rooms/<room>/sets/, live/, <member>/", way: Way::Bin, test: |s| matches!(s.parts, ["rooms", _, top, _, ..] if OLD_ROOM.contains(top) || room::member_id(top)) },
    Rule { what: "log under an older name: rooms/<room>/logs/<not a member ID>.bin", way: Way::Bin, test: |s| matches!(s.parts, ["rooms", _, "logs", name] if name.strip_suffix(".bin").is_some_and(|stem| !room::member_id(stem))) },
    Rule { what: "capture records of older versions: verified.json, cap.json, <v|m>/open.json in a capture folder", way: Way::Bin, test: |s| matches!(s.parts, ["rooms", _, "recordings", _, _, _, name] if OLD_CAPTURE.contains(name)) || matches!(s.parts, ["rooms", _, "recordings", _, _, _, series, "open.json"] if SERIES.contains(series)) },
    Rule { what: "thumbnail of the size before 16:9: rooms/<room>/thumbs/<set>.jpg", way: Way::Bin, test: |s| matches!(s.parts, ["rooms", _, "thumbs", name] if name.strip_suffix(".jpg").is_some_and(room::valid_id)) },
];

fn media(owner: &str, capture: &str, series: &str, name: &str) -> Option<Option<u32>> {
    let whole = name.strip_suffix(".part").unwrap_or(name);
    let n = if whole == INIT { None } else { Some(room::seg_number(whole).filter(|n| *n > 0 && seg_name(*n) == whole)?) };
    (room::member_id(owner) && room::capture_id(capture) && SERIES.contains(&series)).then_some(n)
}

fn media_key(set: &str, owner: &str, capture: &str, series: &str, n: Option<u32>) -> String {
    format!("{set}/{owner}/{capture}/{series}/{}", n.map_or("init".into(), |n| n.to_string()))
}

fn unlogged_media(s: &Spot) -> bool {
    let ["rooms", _, "recordings", set, owner, capture, series, name] = s.parts else { return false };
    let (Some(n), Some(refs)) = (media(owner, capture, series, name), s.refs) else { return false };
    !refs.media.contains(&media_key(set, owner, capture, series, n))
}

#[derive(Default)]
pub struct Refs {
    sets: HashSet<String>,
    media: HashSet<String>,
}

fn refs(room_dir: &Path) -> Option<Refs> {
    if room_dir.join(PENDING).exists() {
        return None;
    }
    let mut out = Refs::default();
    for (name, path) in crate::entries(&room_dir.join(LOGS)) {
        let Some(owner) = name.strip_suffix(".bin").filter(|stem| room::member_id(stem)) else { continue };
        for r in log::scan(&path, 0).0 {
            out.sets.extend(["set", "of"].iter().filter_map(|k| r[*k].as_str().map(String::from)));
            let (Some(set), Some(capture), Some(series), Some(n)) = (r["set"].as_str(), r["capture"].as_str(), r["series"].as_str(), r["n"].as_u64()) else { continue };
            if r["k"] == "seg" {
                out.media.insert(media_key(set, owner, capture, series, Some(n as u32)));
                out.media.insert(media_key(set, owner, capture, series, None));
            }
        }
    }
    Some(out)
}

pub struct Item {
    pub path: PathBuf,
    pub rule: usize,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Default)]
struct Node {
    items: Vec<Item>,
    whole: bool,
    files: u64,
    bytes: u64,
}

struct Walk<'a> {
    dir: &'a Path,
    settled: &'a HashSet<String>,
    now: SystemTime,
    refs: HashMap<String, Option<Refs>>,
}

impl Walk<'_> {
    fn classify(&mut self, parts: &[&str]) -> Option<usize> {
        let refs = match parts {
            [first, ..] if NEVER.contains(first) => return None,
            ["rooms", room, ..] => {
                let dir = self.dir.join("rooms").join(room);
                self.refs.entry(room.to_string()).or_insert_with(|| refs(&dir)).as_ref()
            }
            _ => None,
        };
        let spot = Spot { dir: self.dir, parts, refs };
        RULES.iter().position(|rule| (rule.test)(&spot))
    }

    fn kept(&self, parts: &[&str]) -> bool {
        match parts {
            [name] => NEVER.contains(name) || name.starts_with("key.bin"),
            ["rooms", room] => !self.settled.contains(*room),
            _ => false,
        }
    }

    fn node(&mut self, path: &Path, parts: &[&str]) -> Node {
        let Ok(meta) = std::fs::symlink_metadata(path) else { return Node::default() };
        if meta.file_type().is_symlink() || self.kept(parts) {
            return Node { files: 1, ..Node::default() };
        }
        if meta.is_file() {
            let old = meta.modified().is_ok_and(|t| self.now.duration_since(t).is_ok_and(|age| age >= MIN_AGE));
            let rule = self.classify(parts).filter(|_| old);
            let bytes = meta.len();
            let items = rule.map(|rule| Item { path: path.to_path_buf(), rule, files: 1, bytes }).into_iter().collect();
            return Node { items, whole: rule.is_some_and(|r| RULES[r].way == Way::Bin), files: 1, bytes };
        }
        let mut names: Vec<(String, PathBuf)> = crate::entries(path);
        names.sort();
        let mut out = Node { whole: true, ..Node::default() };
        for (name, child) in names {
            let below: Vec<&str> = parts.iter().copied().chain([name.as_str()]).collect();
            let node = self.node(&child, &below);
            out.whole &= node.whole || node.files == 0;
            out.files += node.files;
            out.bytes += node.bytes;
            out.items.extend(node.items);
        }
        out.whole &= out.files > 0 && out.items.iter().all(|i| RULES[i.rule].way == Way::Bin);
        let floor = matches!(parts, [] | ["rooms"] | ["rooms", _] | ["rooms", _, "recordings" | "standby" | "thumbs" | "logs"]);
        if out.whole && !floor {
            let rule = out.items[0].rule;
            out.items = vec![Item { path: path.to_path_buf(), rule, files: out.files, bytes: out.bytes }];
        }
        out
    }

    fn at(&mut self, path: &Path) -> Option<Item> {
        let rel = path.strip_prefix(self.dir).ok()?;
        let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
        let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
        let mut items = self.node(path, &parts).items;
        (items.len() == 1 && items[0].path == path).then(|| items.remove(0))
    }
}

fn walker<'a>(dir: &'a Path, settled: &'a HashSet<String>, now: SystemTime) -> Walk<'a> {
    Walk { dir, settled, now, refs: HashMap::new() }
}

pub fn plan(dir: &Path, settled: &HashSet<String>, now: SystemTime) -> Vec<Item> {
    walker(dir, settled, now).node(dir, &[]).items
}

#[derive(Default, Debug, PartialEq)]
pub struct Done {
    pub moved: u64,
    pub deleted: u64,
    pub skipped: u64,
    pub freed: u64,
    pub paths: Vec<PathBuf>,
}

pub fn run(dir: &Path, settled: &HashSet<String>, shown: &[PathBuf], now: SystemTime, busy: &dyn Fn() -> bool, mover: &mut dyn FnMut(&Path, Way) -> bool) -> Result<Done, String> {
    if busy() {
        return Err(BUSY.into());
    }
    let mut walk = walker(dir, settled, now);
    let mut done = Done::default();
    for path in shown {
        if busy() {
            break;
        }
        let Some(item) = walk.at(path) else { continue };
        let way = RULES[item.rule].way;
        if mover(&item.path, way) && std::fs::symlink_metadata(&item.path).is_err() {
            *(if way == Way::Bin { &mut done.moved } else { &mut done.deleted }) += item.files;
            done.freed += item.bytes;
            done.paths.push(item.path);
        } else {
            done.skipped += item.files;
        }
    }
    Ok(done)
}

pub fn summary(dir: &Path, items: &[Item]) -> Value {
    let sum = |way: Way| items.iter().filter(|i| RULES[i.rule].way == way).fold((0, 0), |(f, b), i| (f + i.files, b + i.bytes));
    let ((bin_files, bin_bytes), (temp_files, temp_bytes)) = (sum(Way::Bin), sum(Way::Delete));
    let examples: Vec<String> = items.iter().take(EXAMPLES).map(|i| i.path.strip_prefix(dir).unwrap_or(&i.path).to_string_lossy().replace('\\', "/")).collect();
    json!({"files": bin_files + temp_files, "bytes": bin_bytes + temp_bytes, "temp_files": temp_files, "temp_bytes": temp_bytes, "examples": examples})
}

fn busy(core: &Core) -> bool {
    core.any_recording() || core.export.lock().unwrap().is_some() || jobs::busy()
}

fn settled(core: &Core) -> HashSet<String> {
    let ids: Vec<String> = core.rooms.lock().unwrap().list.iter().filter(|r| r.invite.is_none()).map(|r| r.id.clone()).collect();
    ids.into_iter().filter(|id| core.log_ready(id)).collect()
}

pub fn plan_op(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    if busy(core) {
        return Err(BUSY.into());
    }
    let items = plan(&core.dir, &settled(core), SystemTime::now());
    *SHOWN.lock().unwrap() = items.iter().map(|i| i.path.clone()).collect();
    Ok(summary(&core.dir, &items))
}

pub fn run_op(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    if busy(core) {
        return Err(BUSY.into());
    }
    let shown = std::mem::take(&mut *SHOWN.lock().unwrap());
    let (c, settled) = (core.clone(), settled(core));
    let done = std::thread::spawn(move || {
        let _com = Com::new();
        run(&c.dir, &settled, &shown, SystemTime::now(), &|| busy(&c), &mut move_away)
    })
    .join()
    .map_err(|_| "Clean up failed.".to_string())??;
    for path in &done.paths {
        data::forget(&core.files, path.parent().unwrap_or(path));
    }
    debug::event(core, core.started, format!("cleaned up {} files to the bin, {} deleted, {} skipped, {} bytes", done.moved, done.deleted, done.skipped, done.freed));
    Ok(json!({"moved": done.moved, "deleted": done.deleted, "skipped": done.skipped, "freed": done.freed}))
}

fn move_away(path: &Path, way: Way) -> bool {
    match way {
        Way::Delete => std::fs::remove_file(path).is_ok(),
        Way::Bin => to_bin(path),
    }
}

struct Com;

impl Com {
    fn new() -> Self {
        use ::windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        let _ = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        Com
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        unsafe { ::windows::Win32::System::Com::CoUninitialize() };
    }
}

#[windows::core::implement(IFileOperationProgressSink)]
struct OnlyToBin;

#[allow(non_snake_case)]
impl IFileOperationProgressSink_Impl for OnlyToBin_Impl {
    fn StartOperations(&self) -> WinResult<()> {
        Ok(())
    }
    fn FinishOperations(&self, _: HRESULT) -> WinResult<()> {
        Ok(())
    }
    fn PreRenameItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> WinResult<()> {
        Ok(())
    }
    fn PostRenameItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR, _: HRESULT, _: Ref<IShellItem>) -> WinResult<()> {
        Ok(())
    }
    fn PreMoveItem(&self, _: u32, _: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR) -> WinResult<()> {
        Ok(())
    }
    fn PostMoveItem(&self, _: u32, _: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR, _: HRESULT, _: Ref<IShellItem>) -> WinResult<()> {
        Ok(())
    }
    fn PreCopyItem(&self, _: u32, _: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR) -> WinResult<()> {
        Ok(())
    }
    fn PostCopyItem(&self, _: u32, _: Ref<IShellItem>, _: Ref<IShellItem>, _: &PCWSTR, _: HRESULT, _: Ref<IShellItem>) -> WinResult<()> {
        Ok(())
    }
    fn PreDeleteItem(&self, flags: u32, _: Ref<IShellItem>) -> WinResult<()> {
        let to_bin = flags & ::windows::Win32::UI::Shell::TSF_DELETE_RECYCLE_IF_POSSIBLE.0 as u32 != 0;
        if to_bin { Ok(()) } else { Err(::windows::Win32::Foundation::E_ABORT.into()) }
    }
    fn PostDeleteItem(&self, _: u32, _: Ref<IShellItem>, _: HRESULT, _: Ref<IShellItem>) -> WinResult<()> {
        Ok(())
    }
    fn PreNewItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR) -> WinResult<()> {
        Ok(())
    }
    fn PostNewItem(&self, _: u32, _: Ref<IShellItem>, _: &PCWSTR, _: &PCWSTR, _: u32, _: HRESULT, _: Ref<IShellItem>) -> WinResult<()> {
        Ok(())
    }
    fn UpdateProgress(&self, _: u32, _: u32) -> WinResult<()> {
        Ok(())
    }
    fn ResetTimer(&self) -> WinResult<()> {
        Ok(())
    }
    fn PauseTimer(&self) -> WinResult<()> {
        Ok(())
    }
    fn ResumeTimer(&self) -> WinResult<()> {
        Ok(())
    }
}

fn to_bin(path: &Path) -> bool {
    use ::windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
    use ::windows::Win32::UI::Shell::{FileOperation, IFileOperation, SHCreateItemFromParsingName, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT};
    let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
    let _ = (|| -> WinResult<()> {
        unsafe {
            let op: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL)?;
            op.SetOperationFlags(FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT | FOFX_RECYCLEONDELETE | FOFX_EARLYFAILURE)?;
            let sink: IFileOperationProgressSink = OnlyToBin.into();
            let cookie = op.Advise(&sink)?;
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None::<&::windows::Win32::System::Com::IBindCtx>)?;
            op.DeleteItem(&item, None::<&IFileOperationProgressSink>)?;
            let done = op.PerformOperations();
            let _ = op.Unadvise(cookie);
            done
        }
    })();
    std::fs::symlink_metadata(path).is_err()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::tests::{seed, temp, tree};
    use std::collections::BTreeMap;

    const LATER: Duration = Duration::from_secs(2 * 86_400);

    struct Folder {
        dir: PathBuf,
        me: String,
        other: String,
    }

    impl Folder {
        fn new(name: &str) -> Self {
            let (me, other) = (iroh::SecretKey::generate().public().to_string(), iroh::SecretKey::generate().public().to_string());
            Folder { dir: temp(name), me, other }
        }

        fn name(&self, p: &str) -> String {
            p.replace("ME", &self.me).replace("OT", &self.other)
        }

        fn seed(&self, names: &[&str]) {
            seed(&self.dir, names.iter().map(|p| self.name(p)));
        }

        fn log(&self, path: &str, records: &[Value]) {
            let bytes: Vec<u8> = records.iter().flat_map(log::encode).collect();
            crate::log::tests::put(&self.dir, &self.name(path), &bytes);
        }

        fn plan(&self, now: SystemTime) -> Vec<(String, &'static str, u64)> {
            let rooms: HashSet<String> = ["r".to_string()].into();
            plan(&self.dir, &rooms, now).into_iter().map(|i| (self.rel(&i.path), RULES[i.rule].what, i.files)).collect()
        }

        fn rel(&self, path: &Path) -> String {
            path.strip_prefix(&self.dir).unwrap().to_string_lossy().replace('\\', "/").replace(&self.me, "ME").replace(&self.other, "OT")
        }

        fn left(&self) -> BTreeMap<String, Vec<u8>> {
            tree(&self.dir).into_iter().map(|(k, v)| (k.replace(&self.me, "ME").replace(&self.other, "OT"), v)).collect()
        }
    }

    fn seg(set: &str, capture: &str, series: &str, n: u32) -> Value {
        json!({"k": "seg", "set": set, "capture": capture, "series": series, "n": n, "t_us": 0, "d_us": 4, "size": 1, "sha256": "x"})
    }

    const KEPT: [&str; 28] = [
        "key.bin",
        "resume.json",
        "key.bin.part",
        "rooms.json",
        "settings.json",
        "app.lock",
        "log.txt",
        "crash.txt",
        "EBWebView/Default/x",
        "ffmpeg.exe",
        "export/part-0.mp4",
        "notes.txt",
        "keep/rec-1.mp4",
        "settings.json.bak.part",
        "rooms/r/logs/ME.bin.part",
        "rooms/r/logs/OT.bin.part",
        "rooms/r/newer.dat",
        "rooms/r/recordings/s1/ME/1000/v/init.mp4",
        "rooms/r/recordings/s1/ME/1000/v/000001.m4s",
        "rooms/r/recordings/s1/ME/1000/v/000002.m4s.part",
        "rooms/r/recordings/s1/ME/1000/newer.json",
        "rooms/r/recordings/s1/ME/1000/v/clip.mp4",
        "rooms/r/recordings/s1/OT/2000/m/init.mp4",
        "rooms/r/recordings/s1/OT/2000/m/000001.m4s",
        "rooms/r/recordings/odd name!/ME/1/v/000001.m4s.x",
        "rooms/r/thumbs/s1.16x9.jpg",
        "rooms/r/thumbs/s9.16x9.jpg",
        "rooms/unlisted/recordings/gone/ME/1/v/000001.m4s",
    ];

    fn seeded(name: &str) -> Folder {
        let f = Folder::new(name);
        f.seed(&KEPT);
        f.log("rooms/r/logs/ME.bin", &[json!({"k": "rec", "set": "s1", "start_us": 0}), seg("s1", "1000", "v", 1), seg("s1", "1000", "v", 2)]);
        f.log("rooms/r/logs/OT.bin", &[json!({"k": "rec", "set": "s1", "start_us": 0}), seg("s1", "2000", "m", 1), json!({"k": "rec", "set": "d1", "of": "s9"})]);
        f.seed(&[
            "rooms/r/thumbs/s1.tmp.mp4",
            "settings.json.part",
            "resume.json.part",
            "ffmpeg-8.1-essentials_build.zip.part",
            "rooms/r/thumbs/gone.16x9.jpg",
            "rooms/r/recordings/gone/ME/3000/v/init.mp4",
            "rooms/r/recordings/gone/ME/3000/v/000001.m4s",
            "rooms/r/recordings/gone/OT/4000/m/000001.m4s.part",
            "rooms/r/recordings/s1/ME/1000/v/000003.m4s",
            "rooms/r/recordings/s1/ME/5000/m/init.mp4",
            "rooms/r/recordings/s1/ME/5000/m/000001.m4s",
            "rooms/r/standby/OT/6000/v/000009.m4s",
            "log.bin",
            "peer/a/log.bin",
            "plugins/whisper/whisper-cli.exe",
            "plugins/work/1/voice.wav",
            "rec-1700000000000.mp4",
            "rooms/r/sets/s1/a.mp4",
            "rooms/r/live/a.mp4",
            "rooms/r/ME/1000/v/init.mp4",
            "rooms/r/logs/someone.bin",
            "rooms/r/recordings/s1/ME/1000/verified.json",
            "rooms/r/recordings/s1/ME/1000/v/open.json",
            "rooms/r/thumbs/s1.jpg",
        ]);
        f
    }

    #[test]
    fn the_plan_lists_only_known_app_files_that_nothing_refers_to_with_one_item_per_whole_folder() {
        let f = seeded("cleanplan");
        assert!(f.plan(SystemTime::now()).is_empty());
        let mut got: Vec<(String, usize, u64)> = f.plan(SystemTime::now() + LATER).into_iter().map(|(p, what, n)| (p, RULES.iter().position(|r| r.what == what).unwrap(), n)).collect();
        got.sort();
        let want = [
            ("ffmpeg-8.1-essentials_build.zip.part", 2, 1),
            ("log.bin", 6, 1),
            ("peer", 6, 1),
            ("plugins", 7, 2),
            ("rec-1700000000000.mp4", 8, 1),
            ("resume.json.part", 1, 1),
            ("rooms/r/ME", 9, 1),
            ("rooms/r/live", 9, 1),
            ("rooms/r/logs/someone.bin", 10, 1),
            ("rooms/r/recordings/gone", 4, 3),
            ("rooms/r/recordings/s1/ME/1000/v/000003.m4s", 4, 1),
            ("rooms/r/recordings/s1/ME/1000/v/open.json", 11, 1),
            ("rooms/r/recordings/s1/ME/1000/verified.json", 11, 1),
            ("rooms/r/recordings/s1/ME/5000", 4, 2),
            ("rooms/r/sets", 9, 1),
            ("rooms/r/standby/OT", 5, 1),
            ("rooms/r/thumbs/gone.16x9.jpg", 3, 1),
            ("rooms/r/thumbs/s1.jpg", 12, 1),
            ("rooms/r/thumbs/s1.tmp.mp4", 0, 1),
            ("settings.json.part", 1, 1),
        ];
        assert_eq!(got, want.map(|(p, r, n)| (p.to_string(), r, n)));
        assert!((0..RULES.len()).all(|r| got.iter().any(|g| g.1 == r)), "a rule has no case");
        let summary = summary(&f.dir, &plan(&f.dir, &["r".to_string()].into(), SystemTime::now() + LATER));
        assert_eq!((summary["files"].as_u64(), summary["bytes"].as_u64(), summary["temp_files"].as_u64(), summary["temp_bytes"].as_u64()), (Some(24), Some(24), Some(4), Some(4)));
        assert_eq!(summary["examples"].as_array().map(Vec::len), Some(EXAMPLES));
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn a_run_moves_exactly_the_plan_through_the_mover_and_keeps_everything_else() {
        let f = seeded("cleanrun");
        let (rooms, later) = (["r".to_string()].into(), SystemTime::now() + LATER);
        let shown: Vec<PathBuf> = plan(&f.dir, &rooms, later).into_iter().map(|i| i.path).collect();
        f.seed(&["rooms/r/recordings/gone2/ME/1/v/000001.m4s"]);
        let bin = f.dir.with_extension("bin");
        let _ = std::fs::remove_dir_all(&bin);
        let mut ways = Vec::new();
        let mut mover = |path: &Path, way: Way| {
            ways.push((f.rel(path), way));
            if f.rel(path) == "peer" {
                return false;
            }
            let to = bin.join(ways.len().to_string());
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::rename(path, to).is_ok()
        };
        let done = run(&f.dir, &rooms, &shown, later, &|| false, &mut mover).unwrap();
        assert_eq!((done.moved, done.deleted, done.skipped, done.freed), (19, 4, 1, 23));
        assert_eq!(ways.iter().filter(|(_, w)| *w == Way::Delete).map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["ffmpeg-8.1-essentials_build.zip.part", "resume.json.part", "rooms/r/thumbs/s1.tmp.mp4", "settings.json.part"]);
        let mut left: Vec<String> = f.left().into_keys().collect();
        left.retain(|p| !p.starts_with("rooms/r/recordings/gone2/") && !p.starts_with("peer/"));
        let mut kept: Vec<String> = KEPT.iter().map(|p| p.to_string()).chain(["rooms/r/logs/ME.bin".into(), "rooms/r/logs/OT.bin".into()]).collect();
        kept.sort();
        assert_eq!(left, kept);
        assert!(f.dir.join(f.name("rooms/r/recordings/gone2/ME/1/v/000001.m4s")).exists(), "a file that was not in the plan went");
        let _ = std::fs::remove_dir_all(&f.dir);
        let _ = std::fs::remove_dir_all(&bin);
    }

    #[test]
    fn a_run_checks_again_before_each_move_and_refuses_while_busy() {
        let f = seeded("cleancheck");
        let (rooms, later) = (["r".to_string()].into(), SystemTime::now() + LATER);
        let shown: Vec<PathBuf> = plan(&f.dir, &rooms, later).into_iter().map(|i| i.path).collect();
        let mut moved = 0;
        let mut mover = |path: &Path, _: Way| {
            moved += 1;
            std::fs::remove_dir_all(path).or_else(|_| std::fs::remove_file(path)).is_ok()
        };
        assert_eq!(run(&f.dir, &rooms, &shown, later, &|| true, &mut mover), Err(BUSY.to_string()));
        let fresh = run(&f.dir, &rooms, &shown, SystemTime::now(), &|| false, &mut mover).unwrap();
        assert_eq!((fresh.moved, fresh.skipped), (0, 0));
        f.log("rooms/r/logs/ME.bin", &[json!({"k": "rec", "set": "s1", "start_us": 0}), seg("s1", "1000", "v", 1), seg("s1", "1000", "v", 2), seg("s1", "1000", "v", 3), seg("gone", "3000", "v", 1)]);
        crate::log::tests::put(&f.dir, "rooms/r/pending.bin", b"");
        let calls = std::cell::Cell::new(0);
        let busy_later = || {
            calls.set(calls.get() + 1);
            calls.get() > 4
        };
        let done = run(&f.dir, &rooms, &shown, later, &busy_later, &mut mover).unwrap();
        assert_eq!((done.moved + done.deleted, moved), (3, 3));
        let left = f.left();
        for p in ["rooms/r/recordings/gone/ME/3000/v/000001.m4s", "rooms/r/recordings/s1/ME/1000/v/000003.m4s", "rooms/r/thumbs/gone.16x9.jpg", "rooms/r/sets/s1/a.mp4"] {
            assert!(left.contains_key(p), "{p} went");
        }
        let _ = std::fs::remove_dir_all(&f.dir);
    }

    #[test]
    fn every_pattern_names_what_it_removes() {
        assert!(RULES.iter().all(|r| r.what.contains(':')));
        assert_eq!(RULES.iter().filter(|r| r.way == Way::Delete).count(), 3);
    }
}

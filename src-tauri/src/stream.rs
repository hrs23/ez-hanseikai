use std::collections::HashMap;
use std::fs::File as Disk;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Value};
use tauri::http::{header, Request, Response, StatusCode};

use crate::room::{self, capture_id, part_of, seg_name, valid_id, File, INIT, SERIES};
use crate::{data, file_len, media, Core};

const INDEX_MAX: usize = 1 << 16;

pub type Indexes = Mutex<HashMap<PathBuf, (u64, Option<media::Time>)>>;

#[derive(Clone)]
pub struct Piece {
    pub path: PathBuf,
    pub len: u64,
}

pub fn size(pieces: &[Piece]) -> u64 {
    pieces.iter().map(|p| p.len).sum()
}

fn piece(path: PathBuf) -> Piece {
    Piece { len: file_len(&path), path }
}

pub fn set_pieces(core: &Core, room: &str, f: &File, capture: &str, series: &str) -> Vec<Piece> {
    listed(&core.files, &data::seg_dir(&core.room_dir(room), f, capture, series), false)
}

pub fn run_of(numbers: &[u32], live: bool) -> &[u32] {
    if live {
        let mut from = numbers.len().saturating_sub(1);
        while from > 0 && numbers.len() - from < data::LIVE_KEEP as usize && numbers[from - 1] + 1 == numbers[from] {
            from -= 1;
        }
        &numbers[from..]
    } else {
        let end = (1..numbers.len()).find(|&i| numbers[i - 1] + 1 != numbers[i]).unwrap_or(numbers.len());
        &numbers[..end]
    }
}

fn run(files: &data::Files, dir: &Path, live: bool) -> Option<(data::Listing, Vec<u32>)> {
    let names = data::listing(files, dir);
    names.get(&0)?;
    let mut numbers: Vec<u32> = names.keys().copied().filter(|n| *n > 0).collect();
    numbers.sort_unstable();
    let next = numbers.last().map_or(1, |n| n + 1);
    numbers.extend(Some(next).filter(|n| part_of(&dir.join(seg_name(*n))).exists()));
    let run = run_of(&numbers, live).to_vec();
    Some((names, run))
}

fn listed(files: &data::Files, dir: &Path, live: bool) -> Vec<Piece> {
    let Some((names, run)) = run(files, dir, live) else { return Vec::new() };
    let segments = run.into_iter().map(|n| {
        let path = dir.join(seg_name(n));
        match names.get(&n) {
            Some(len) => Piece { path, len: *len },
            None => piece(part_of(&path)),
        }
    });
    std::iter::once(Piece { path: dir.join(INIT), len: names[&0] }).chain(segments).collect()
}

pub fn folder_size(files: &data::Files, dir: &Path) -> (u64, bool) {
    let Some((names, run)) = run(files, dir, false) else { return (0, false) };
    let len = |n: &u32| names.get(n).copied().unwrap_or_else(|| file_len(&part_of(&dir.join(seg_name(*n)))));
    (names[&0] + run.iter().map(len).sum::<u64>(), run.last().is_some_and(|n| !names.contains_key(n)))
}

fn live_folder(core: &Core, room: &str, owner: &str) -> Option<(PathBuf, String, Option<i64>)> {
    let (capture, set, start_us) = if owner == core.me() {
        let live = core.engine.live().filter(|l| l.room == room)?;
        (live.capture, live.recent[0].last().and_then(|(_, set)| set.clone()), live.start_us)
    } else {
        let live = core.peers.lock().unwrap().iter().find(|p| p.id.to_string() == owner)?.live.clone()?;
        (live.capture, live.set[0].clone(), live.start_us)
    };
    Some((core.series_dir(room, set.as_deref(), owner, &capture, SERIES[0]), capture, start_us))
}

pub fn live_json(core: &Core, room: &str, owner: &str) -> Option<Value> {
    let (dir, capture, start_us) = live_folder(core, room, owner)?;
    let pieces = listed(&core.files, &dir, true);
    (pieces.len() > 1).then(|| json!({"path": format!("live/{room}/{owner}/{capture}"), "size": size(&pieces), "start_us": start_us}))
}

#[derive(Debug, PartialEq)]
enum Target<'a> {
    Live { room: &'a str, owner: &'a str, capture: &'a str },
    Set { room: &'a str, set: &'a str, owner: &'a str, series: &'a str, capture: &'a str },
}

fn target<'a>(parts: &[&'a str]) -> Option<(Target<'a>, &'a str)> {
    match *parts {
        ["live", room, owner, capture, what] if valid_id(room) && room::member_id(owner) && capture_id(capture) => Some((Target::Live { room, owner, capture }, what)),
        ["set", room, set, owner, series, capture, what] if valid_id(room) && valid_id(set) && room::member_id(owner) && SERIES.contains(&series) && capture_id(capture) => Some((Target::Set { room, set, owner, series, capture }, what)),
        _ => None,
    }
}

fn first_time(indexes: &Indexes, piece: &Piece) -> Option<media::Time> {
    let mut cache = indexes.lock().unwrap();
    if let Some((len, time)) = cache.get(&piece.path) {
        if *len == piece.len {
            return time.clone();
        }
    }
    if cache.len() > INDEX_MAX {
        cache.clear();
    }
    let (times, _) = media::scan(&piece.path).or_else(|_| media::scan(&piece.path.with_extension(""))).ok()?;
    let time = times.into_iter().next();
    cache.insert(piece.path.clone(), (piece.len, time.clone()));
    time
}

fn open(pieces: &[Piece]) -> bool {
    pieces.last().is_some_and(|p| p.path.extension().is_some_and(|e| e == "part"))
}

fn index_json(indexes: &Indexes, dir: &Path, pieces: &[Piece], from: u64) -> Value {
    let scales = media::timescales(&std::fs::read(dir.join(INIT)).unwrap_or_default());
    let number = |p: &Piece| p.path.file_name().and_then(|name| room::seg_number(&name.to_string_lossy()));
    let pieces = &pieces[pieces.iter().take_while(|p| number(p).is_some_and(|n| u64::from(n) < from)).count()..];
    let mut segs = Vec::new();
    for p in pieces {
        let Some((n, time)) = number(p).zip(first_time(indexes, p).map(|t| media::micros(&t, &scales))) else { break };
        segs.push(json!({"n": n, "len": p.len, "t_us": time}));
    }
    json!({"segs": segs, "open": segs.len() == pieces.len() && open(pieces)})
}

fn serve(files: &data::Files, indexes: &Indexes, dir: &Path, live: bool, kind: &'static str, what: &str, from: u64) -> Option<(&'static str, Vec<u8>)> {
    let mut file = match what {
        "index" => return Some(("application/json", index_json(indexes, dir, listed(files, dir, live).get(1..).unwrap_or_default(), from).to_string().into_bytes())),
        "init" => Disk::open(dir.join(INIT)).ok()?,
        n => {
            let path = dir.join(seg_name(n.parse().ok()?));
            Disk::open(&path).or_else(|_| Disk::open(part_of(&path))).ok()?
        }
    };
    let mut bytes = Vec::with_capacity(file.metadata().ok()?.len().checked_sub(from)? as usize);
    file.seek(SeekFrom::Start(from)).ok()?;
    file.read_to_end(&mut bytes).ok()?;
    Some((kind, bytes))
}

fn decoded(path: &str) -> Option<String> {
    crate::api::decode(path.trim_start_matches('/'), false)
}

fn answer(core: &Core, path: &str, from: u64) -> Option<(&'static str, Vec<u8>)> {
    let path = decoded(path)?;
    let (found, what) = target(&path.split('/').collect::<Vec<_>>())?;
    match found {
        Target::Live { room, owner, capture } => {
            let (dir, _, _) = live_folder(core, room, owner).filter(|(_, current, _)| current == capture)?;
            serve(&core.files, &core.indexes, &dir, true, "video/mp4", what, from)
        }
        Target::Set { room, set, owner, series, capture } => serve(&core.files, &core.indexes, &core.series_dir(room, Some(set), owner, capture, series), false, if series == SERIES[1] { "audio/mp4" } else { "video/mp4" }, what, from),
    }
}

pub fn read(pieces: &[Piece], start: u64, len: u64) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(len as usize);
    let (end, mut at) = (start + len, 0u64);
    for p in pieces {
        let (from, to) = (start.max(at), end.min(at + p.len));
        if from < to {
            let mut file = Disk::open(&p.path).or_else(|_| Disk::open(p.path.with_extension("")))?;
            file.seek(SeekFrom::Start(from - at))?;
            let mut buf = vec![0u8; (to - from) as usize];
            file.read_exact(&mut buf)?;
            out.extend_from_slice(&buf);
        }
        at += p.len;
    }
    Ok(out)
}

fn thumb(core: &Core, path: &str) -> Option<Vec<u8>> {
    let path = decoded(path)?;
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["thumb", room, set] if valid_id(room) && valid_id(set) => std::fs::read(crate::thumbs::path(&core.room_dir(room), set)).ok(),
        _ => None,
    }
}

fn reply(status: StatusCode, kind: &str, cache: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder().status(status).header(header::CONTENT_TYPE, kind).header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*").header(header::CACHE_CONTROL, cache).body(body).unwrap()
}

pub fn respond(core: &Core, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let path = request.uri().path();
    if let Some(body) = thumb(core, path) {
        return reply(StatusCode::OK, "image/jpeg", "max-age=3600", body);
    }
    let from = request.uri().query().and_then(|q| q.strip_prefix("from=")).and_then(|v| v.parse().ok()).unwrap_or(0);
    match answer(core, path, from) {
        Some((kind, body)) => reply(StatusCode::OK, kind, "no-store", body),
        None => reply(StatusCode::NOT_FOUND, "text/plain", "no-store", Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::tests::{put, temp};
    use crate::media::tests::{fragment, init};

    fn names(pieces: &[Piece]) -> Vec<String> {
        pieces.iter().map(|p| p.path.file_name().unwrap().to_string_lossy().to_string()).collect()
    }

    fn folder(name: &str, segments: &[(u32, u64)], open: Option<(u32, u64)>) -> PathBuf {
        let dir = temp(name);
        put(&dir, INIT, &init(&[(1, 1000)]));
        for (n, t) in segments {
            put(&dir, &seg_name(*n), &fragment(&[(1, *t)], b"data"));
        }
        if let Some((n, t)) = open {
            put(&dir, &format!("{}.part", seg_name(n)), &fragment(&[(1, t)], b"open"));
        }
        dir
    }

    #[test]
    fn the_live_folder_lists_its_newest_run_and_a_recording_its_oldest() {
        let live = data::LIVE_KEEP as usize;
        let numbers: Vec<u32> = (4..=10).chain(12..=12 + live as u32).collect();
        assert_eq!(run_of(&numbers, true), (12..=12 + live as u32).skip(1).collect::<Vec<_>>());
        assert_eq!(run_of(&numbers[..9], true), [12, 13]);
        assert_eq!(run_of(&numbers, false), (4..=10).collect::<Vec<_>>());
        assert_eq!(run_of(&[], true), [] as [u32; 0]);
        assert_eq!(run_of(&[], false), [] as [u32; 0]);
        let dir = folder("listed", &[(4, 0), (5, 4000), (7, 12000)], Some((8, 16000)));
        let files = data::Files::default();
        assert_eq!(names(&listed(&files, &dir, true)), [INIT, "000007.m4s", "000008.m4s.part"]);
        assert_eq!(names(&listed(&files, &dir, false)), [INIT, "000004.m4s", "000005.m4s"]);
        assert_eq!(folder_size(&files, &dir), (size(&listed(&files, &dir, false)), false));
        assert_eq!(folder_size(&files, &dir.join("missing")), (0, false));
        assert!(listed(&files, &dir.join("missing"), true).is_empty());
        std::fs::remove_file(dir.join(INIT)).unwrap();
        data::forget(&files, &dir);
        assert!(listed(&files, &dir, false).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_index_lists_pieces_with_their_first_time_and_marks_an_open_tail() {
        let dir = folder("index", &[(1, 0), (2, 4000), (3, 8000)], Some((4, 12000)));
        let (files, indexes) = (data::Files::default(), Indexes::default());
        let json = |dir: &Path| index_json(&indexes, dir, &listed(&files, dir, false)[1..], 0);
        let whole = fragment(&[(1, 0)], b"data").len();
        assert_eq!(json(&dir), json!({"segs": [{"n": 1, "len": whole, "t_us": 0}, {"n": 2, "len": whole, "t_us": 4_000_000}, {"n": 3, "len": whole, "t_us": 8_000_000}, {"n": 4, "len": whole, "t_us": 12_000_000}], "open": true}));
        let tail = |from: u64| index_json(&indexes, &dir, &listed(&files, &dir, false)[1..], from);
        assert_eq!(tail(3), json!({"segs": [{"n": 3, "len": whole, "t_us": 8_000_000}, {"n": 4, "len": whole, "t_us": 12_000_000}], "open": true}));
        assert_eq!(tail(5), json!({"segs": [], "open": false}));
        assert_eq!(folder_size(&files, &dir), (size(&listed(&files, &dir, false)), true));
        put(&dir, "000004.m4s.part", b"no fragment yet");
        data::forget(&files, &dir);
        assert_eq!(json(&dir)["segs"].as_array().unwrap().len(), 3);
        assert_eq!(json(&dir)["open"], false);
        std::fs::rename(dir.join("000004.m4s.part"), dir.join("000004.m4s")).unwrap();
        data::forget(&files, &dir);
        assert_eq!(json(&dir)["segs"].as_array().unwrap().len(), 3);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_folder_serves_its_index_init_and_segments_and_a_tail_from_a_byte() {
        let dir = folder("serve", &[(1, 0)], Some((2, 4000)));
        let (files, indexes) = (data::Files::default(), Indexes::default());
        let get = |what: &str, from: u64| serve(&files, &indexes, &dir, false, "video/mp4", what, from);
        assert_eq!(get("index", 0).unwrap().0, "application/json");
        assert_eq!(get("init", 0).unwrap().1, init(&[(1, 1000)]));
        assert_eq!(get("1", 0).unwrap().1, fragment(&[(1, 0)], b"data"));
        let tail = fragment(&[(1, 4000)], b"open");
        assert_eq!(get("2", 0).unwrap().1, tail);
        assert_eq!(get("2", 5).unwrap().1, tail[5..]);
        assert_eq!(get("2", tail.len() as u64).unwrap().1, Vec::<u8>::new());
        assert!(get("2", tail.len() as u64 + 1).is_none());
        assert!(get("3", 0).is_none() && get("x", 0).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn paths_resolve_to_a_folder_without_a_log_and_bad_ids_are_refused() {
        let owner = iroh::SecretKey::generate().public().to_string();
        let split = |path: String| target(&path.split('/').collect::<Vec<_>>()).map(|(_, what)| what.to_string());
        assert_eq!(split(format!("live/room/{owner}/1000/index")).as_deref(), Some("index"));
        assert_eq!(split(format!("set/room/2026-01-02 12-00-00/{owner}/m/1000/7")).as_deref(), Some("7"));
        for bad in [format!("live/../{owner}/1000/index"), format!("live/room/nobody/1000/index"), format!("live/room/{owner}/10x/index"), format!("set/room/set/{owner}/x/1000/init"), format!("set/room/a%2Fb/{owner}/v/1000/init"), format!("set/room/set/{owner}/v/1000"), format!("live/room/{owner}/1000/index/more")] {
            assert!(split(bad.clone()).is_none(), "{bad}");
        }
    }
}

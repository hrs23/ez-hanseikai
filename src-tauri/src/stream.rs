use std::collections::HashMap;
use std::fs::File as Disk;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::http::{header, Request, Response, StatusCode};

use crate::room::{self, capture_id, part_of, seg_name, valid_id, File, INIT, SERIES};
use crate::{data, file_len, media, Core};

pub const LIVE_WINDOW: u32 = 3;
const RANGE_MAX: u64 = 4 << 20;
const INDEX_MAX: usize = 4096;

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

fn growing(dir: &Path, mut n: u32, out: &mut Vec<Piece>) {
    loop {
        let path = dir.join(seg_name(n));
        if !path.exists() {
            out.extend(Some(part_of(&path)).filter(|part| part.exists()).map(piece));
            return;
        }
        out.push(piece(path));
        n += 1;
    }
}

fn pieces_in(files: &data::Files, dir: &Path, f: &File, capture: &str, series: &str) -> Vec<Piece> {
    if !data::has(files, dir, INIT, f.init(capture, series).map(|(size, _)| size)) {
        return Vec::new();
    }
    let mut out = vec![piece(dir.join(INIT))];
    let mut next = None;
    let open = f.end_us.is_none();
    for seg in f.segs.iter().filter(|s| s.capture == capture && s.series == series) {
        let name = seg_name(seg.n);
        if !data::has(files, dir, &name, Some(seg.size)) {
            out.extend(Some(part_of(&dir.join(name))).filter(|part| open && part.exists()).map(piece));
            return out;
        }
        out.push(Piece { path: dir.join(name), len: seg.size });
        next = Some(seg.n + 1);
    }
    if let Some(n) = next.filter(|_| open) {
        growing(dir, n, &mut out);
    }
    out
}

pub fn set_pieces(core: &Core, room: &str, f: &File, capture: &str, series: &str) -> Vec<Piece> {
    pieces_in(&core.files, &data::seg_dir(&core.room_dir(room), f, capture, series), f, capture, series)
}

pub fn window(newest: u32) -> u32 {
    if newest < 2 { 1 } else { (newest - 2) / LIVE_WINDOW * LIVE_WINDOW + 1 }
}

fn live_from(numbers: &[u32]) -> Option<u32> {
    let newest = *numbers.iter().max()?;
    let mut from = newest;
    while from > window(newest) && numbers.contains(&(from - 1)) {
        from -= 1;
    }
    Some(from)
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

fn pieces_from(dir: &Path, from: u32) -> Vec<Piece> {
    let init = dir.join(INIT);
    if !init.exists() {
        return Vec::new();
    }
    let mut out = vec![piece(init)];
    growing(dir, from, &mut out);
    out
}

fn live_start(dir: &Path) -> Option<(u32, Vec<Piece>)> {
    let numbers: Vec<u32> = crate::entries(dir).iter().filter_map(|(name, _)| room::seg_number(name)).collect();
    let from = live_from(&numbers)?;
    let pieces = pieces_from(dir, from);
    (pieces.len() > 1).then_some((from, pieces))
}

fn first_us(core: &Core, pieces: &[Piece]) -> Option<i64> {
    let scales = media::timescales(&std::fs::read(&pieces.first()?.path).ok()?);
    Some(media::micros(index(core, pieces.get(1)?).first()?, &scales))
}

pub fn live_json(core: &Core, room: &str, owner: &str) -> Option<Value> {
    let (dir, capture, start_us) = live_folder(core, room, owner)?;
    let (from, pieces) = live_start(&dir)?;
    let start_us = start_us.zip(first_us(core, &pieces)).map(|(start, first)| start + first);
    Some(json!({"path": format!("live/{room}/{owner}/{capture}/{from}"), "size": size(&pieces), "start_us": start_us}))
}

fn decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%').then(|| path.get(i + 1..i + 3)).flatten().and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(b) => {
                out.push(b);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn pieces_for(core: &Core, path: &str) -> Option<(Vec<Piece>, &'static str)> {
    let path = decode(path.trim_start_matches('/'));
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["live", room, owner, capture, from] if valid_id(room) && room::member_id(owner) && capture_id(capture) => {
            let (dir, _, _) = live_folder(core, room, owner).filter(|(_, current, _)| current == capture)?;
            Some((pieces_from(&dir, from.parse().ok()?), "video/mp4"))
        }
        ["set", room, set, owner, series, capture] if valid_id(room) && valid_id(set) && room::member_id(owner) && SERIES.contains(series) && capture_id(capture) => {
            let found = room::sets(core, room, &room::logs(core, room)).into_iter().find(|s| s.id == *set)?;
            let f = found.files.iter().find(|f| f.owner == *owner)?;
            Some((set_pieces(core, room, f, capture, series), if *series == SERIES[1] { "audio/mp4" } else { "video/mp4" }))
        }
        _ => None,
    }
}

fn index(core: &Core, piece: &Piece) -> Arc<Vec<media::Time>> {
    let mut cache = core.indexes.lock().unwrap();
    if let Some((len, times)) = cache.get(&piece.path) {
        if *len == piece.len {
            return times.clone();
        }
    }
    if cache.len() > INDEX_MAX {
        cache.clear();
    }
    let Ok((times, _)) = media::scan(&piece.path).or_else(|_| media::scan(&piece.path.with_extension(""))) else { return Arc::default() };
    let times = Arc::new(times);
    cache.insert(piece.path.clone(), (piece.len, times.clone()));
    times
}

pub fn read(core: &Core, pieces: &[Piece], start: u64, len: u64) -> std::io::Result<Vec<u8>> {
    let bases: HashMap<u32, u64> = pieces.get(1).map(|p| media::bases(&index(core, p))).unwrap_or_default();
    let mut out = Vec::with_capacity(len as usize);
    let (end, mut at) = (start + len, 0u64);
    for (i, p) in pieces.iter().enumerate() {
        let (from, to) = (start.max(at), end.min(at + p.len));
        if from < to {
            let mut file = Disk::open(&p.path).or_else(|_| Disk::open(p.path.with_extension("")))?;
            file.seek(SeekFrom::Start(from - at))?;
            let mut buf = vec![0u8; (to - from) as usize];
            file.read_exact(&mut buf)?;
            if i > 0 {
                media::patch(&mut buf, from - at, &index(core, p), &bases);
            }
            out.extend_from_slice(&buf);
        }
        at += p.len;
    }
    Ok(out)
}

pub fn range(text: Option<&str>, total: u64) -> Option<(u64, u64)> {
    let spec = text.unwrap_or("bytes=0-").trim().strip_prefix("bytes=")?;
    let (a, b) = spec.split_once('-')?;
    let start: u64 = a.trim().parse().ok()?;
    if start >= total {
        return None;
    }
    let last = b.trim().parse::<u64>().unwrap_or(total - 1).min(total - 1).min(start + RANGE_MAX - 1);
    (last >= start).then_some((start, last))
}

fn thumb(core: &Core, path: &str) -> Option<Vec<u8>> {
    let path = decode(path.trim_start_matches('/'));
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["thumb", room, set] if valid_id(room) && valid_id(set) => std::fs::read(crate::thumbs::path(&core.room_dir(room), set)).ok(),
        _ => None,
    }
}

pub fn respond(core: &Core, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let empty = |status: StatusCode| Response::builder().status(status).body(Vec::new()).unwrap();
    if let Some(body) = thumb(core, request.uri().path()) {
        return Response::builder().status(StatusCode::OK).header(header::CONTENT_TYPE, "image/jpeg").header(header::CACHE_CONTROL, "max-age=3600").body(body).unwrap();
    }
    let Some((pieces, kind)) = pieces_for(core, request.uri().path()) else { return empty(StatusCode::NOT_FOUND) };
    let total = size(&pieces);
    let wanted = request.headers().get(header::RANGE).and_then(|v| v.to_str().ok());
    let Some((start, last)) = range(wanted, total) else {
        return Response::builder().status(StatusCode::RANGE_NOT_SATISFIABLE).header(header::CONTENT_RANGE, format!("bytes */{total}")).body(Vec::new()).unwrap();
    };
    let Ok(body) = read(core, &pieces, start, last - start + 1) else { return empty(StatusCode::INTERNAL_SERVER_ERROR) };
    Response::builder()
        .status(StatusCode::PARTIAL_CONTENT)
        .header(header::CONTENT_TYPE, kind)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_RANGE, format!("bytes {start}-{}/{total}", start + body.len() as u64 - 1))
        .header(header::CACHE_CONTROL, "no-store")
        .body(body)
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::tests::{put, seed, temp};
    use crate::room::tests::small_file;

    #[test]
    fn ranges_are_capped_and_checked() {
        assert_eq!(range(None, 10), Some((0, 9)));
        assert_eq!(range(Some("bytes=2-"), 10), Some((2, 9)));
        assert_eq!(range(Some("bytes=2-4"), 10), Some((2, 4)));
        assert_eq!(range(Some("bytes=2-40"), 10), Some((2, 9)));
        assert_eq!(range(Some("bytes=10-"), 10), None);
        assert_eq!(range(Some("bytes=0-"), RANGE_MAX * 3), Some((0, RANGE_MAX - 1)));
        assert_eq!(range(Some("items=0-"), 10), None);
    }

    #[test]
    fn a_recording_plays_from_its_own_folder_up_to_the_first_missing_segment() {
        let dir = temp("pieces");
        let (recording, ended, files) = (small_file(4, false), small_file(4, true), data::Files::default());
        let lens = |f: &File| {
            data::forget(&files, &dir);
            pieces_in(&files, &dir, f, "1000", "v").iter().map(|p| (p.path.file_name().unwrap().to_string_lossy().to_string(), p.len)).collect::<Vec<_>>()
        };
        let write = |all: &[(&str, &str)]| all.iter().for_each(|(name, body)| put(&dir, name, body.as_bytes()));
        assert!(lens(&ended).is_empty());
        write(&[(INIT, "init"), ("000001.m4s", "seg!"), ("000002.m4s", "seg!"), ("000003.m4s", "bad"), ("000004.m4s", "seg!")]);
        assert_eq!(lens(&ended), [(INIT.to_string(), 4), ("000001.m4s".to_string(), 4), ("000002.m4s".to_string(), 4)]);
        write(&[("000003.m4s", "seg!"), ("000005.m4s", "next"), ("000006.m4s.part", "open")]);
        assert_eq!(lens(&ended).len(), 4 + 1);
        assert_eq!(lens(&recording)[5..], [("000005.m4s".to_string(), 4), ("000006.m4s.part".to_string(), 4)]);
        assert_eq!(lens(&small_file(6, false)), lens(&recording));
        assert_eq!(lens(&small_file(6, true)).len(), 6);
        write(&[(INIT, "other init")]);
        assert!(lens(&ended).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn live_window_moves_in_steps_and_keeps_a_whole_segment() {
        assert_eq!([1, 2, 3, 4, 5, 6, 7, 8].map(window), [1, 1, 1, 1, 4, 4, 4, 7]);
        let standby = |after: &[u32]| [(4..=10).collect::<Vec<u32>>(), after.to_vec()].concat();
        assert_eq!([&[][..], &[12], &[12, 13], &[12, 13, 14]].map(|after| live_from(&standby(after))), [Some(7), Some(12), Some(12), Some(13)]);
        assert_eq!(live_from(&[]), None);
        let dir = temp("live");
        seed(&dir, (4..=11).map(seg_name).chain([format!("{}.part", seg_name(12))]));
        let start = |dir: &Path| live_start(dir).map(|(from, p)| (from, size(&p)));
        assert_eq!(start(&dir), None);
        put(&dir, INIT, b"init");
        assert_eq!((start(&dir), start(&dir.join("missing"))), (Some((10, 4 + 3)), None));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn paths_are_decoded() {
        assert_eq!(decode("set%2Fr%2F2026-01-02%2012-00-00%2Fa"), "set/r/2026-01-02 12-00-00/a");
        assert_eq!(decode("a%zz"), "a%zz");
    }
}

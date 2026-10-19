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
        if path.exists() {
            out.push(piece(path));
        } else if part_of(&path).exists() {
            out.push(piece(part_of(&path)));
            return;
        } else {
            return;
        }
        n += 1;
    }
}

pub fn set_pieces(core: &Core, room: &str, f: &File, capture: &str, series: &str) -> Vec<Piece> {
    let dir = core.capture_dir(room, &f.owner, capture).join(series);
    let init = dir.join(INIT);
    if !init.exists() {
        return Vec::new();
    }
    let mut out = vec![piece(init)];
    let recording = f.end_us.is_none();
    let own = f.owner == core.me();
    let mut next = None;
    for seg in f.segs.iter().filter(|s| s.capture == capture && s.series == series) {
        let path = dir.join(seg_name(seg.n));
        let check = data::seg_check(core, room, &f.owner, seg);
        if !(check == Some(true) || (own || (recording && check.is_none())) && path.exists()) {
            return out;
        }
        out.push(Piece { len: if check == Some(true) { seg.size } else { file_len(&path) }, path });
        next = Some(seg.n + 1);
    }
    if let Some(n) = next.filter(|_| recording) {
        growing(&dir, n, &mut out);
    }
    out
}

pub fn window(newest: u32) -> u32 {
    if newest < 2 { 1 } else { (newest - 2) / LIVE_WINDOW * LIVE_WINDOW + 1 }
}

fn newest_capture(core: &Core, room: &str, owner: &str) -> Option<String> {
    if owner == core.me() {
        return core.engine.live().filter(|(r, _, _)| r == room).map(|(_, capture, _)| capture);
    }
    let names = std::fs::read_dir(core.dir.join("rooms").join(room).join(owner)).ok()?;
    names.flatten().filter_map(|e| e.file_name().to_string_lossy().parse::<u64>().ok()).max().map(|c| c.to_string())
}

fn live_pieces(core: &Core, room: &str, owner: &str, capture: &str, from: u32) -> Vec<Piece> {
    let dir = core.capture_dir(room, owner, capture).join(SERIES[0]);
    let init = dir.join(INIT);
    if !init.exists() {
        return Vec::new();
    }
    let mut out = vec![piece(init)];
    growing(&dir, from, &mut out);
    out
}

pub fn live_json(core: &Core, room: &str, owner: &str) -> Option<Value> {
    let capture = newest_capture(core, room, owner)?;
    let dir = core.capture_dir(room, owner, &capture).join(SERIES[0]);
    let numbers: Vec<u32> = std::fs::read_dir(&dir).ok()?.flatten().filter_map(|e| room::seg_number(&e.file_name().to_string_lossy())).collect();
    let start = window(*numbers.iter().max()?);
    let from = numbers.iter().copied().filter(|n| *n >= start).min()?;
    let pieces = live_pieces(core, room, owner, &capture, from);
    (pieces.len() > 1).then(|| json!({"path": format!("live/{room}/{owner}/{capture}/{from}"), "size": size(&pieces)}))
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

fn member(id: &str) -> bool {
    id.parse::<iroh::EndpointId>().is_ok()
}

pub fn pieces_for(core: &Core, path: &str) -> Option<(Vec<Piece>, &'static str)> {
    let path = decode(path.trim_start_matches('/'));
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["live", room, owner, capture, from] if valid_id(room) && member(owner) && capture_id(capture) => Some((live_pieces(core, room, owner, capture, from.parse().ok()?), "video/mp4")),
        ["set", room, set, owner, series, capture] if valid_id(room) && valid_id(set) && member(owner) && SERIES.contains(series) && capture_id(capture) => {
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
    let times = Arc::new(media::scan(&piece.path).0);
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

pub fn respond(core: &Core, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
    let empty = |status: StatusCode| Response::builder().status(status).body(Vec::new()).unwrap();
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
    fn live_window_moves_in_steps_and_keeps_a_whole_segment() {
        assert_eq!([1, 2, 3, 4, 5, 6, 7, 8].map(window), [1, 1, 1, 1, 4, 4, 4, 7]);
    }

    #[test]
    fn paths_are_decoded() {
        assert_eq!(decode("set%2Fr%2F2026-01-02%2012-00-00%2Fa"), "set/r/2026-01-02 12-00-00/a");
        assert_eq!(decode("a%zz"), "a%zz");
    }
}

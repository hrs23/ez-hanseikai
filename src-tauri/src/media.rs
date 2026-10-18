use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const HEADER: usize = 8;
const WIDE_HEADER: usize = 16;

#[derive(Clone)]
pub struct Time {
    pub at: u64,
    pub wide: bool,
    pub track: u32,
    pub value: u64,
}

pub enum Out {
    Init(Vec<u8>),
    Fragment(Vec<u8>),
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn be64(data: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(data.get(at..at + 8)?.try_into().ok()?))
}

pub fn header(data: &[u8]) -> Option<([u8; 4], usize, u64)> {
    let size = be32(data, 0)? as u64;
    let kind: [u8; 4] = data.get(4..8)?.try_into().ok()?;
    if size == 1 {
        return Some((kind, WIDE_HEADER, be64(data, HEADER)?));
    }
    Some((kind, HEADER, size))
}

pub fn children(data: &[u8]) -> Vec<([u8; 4], usize, &[u8])> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some((kind, head, size)) = header(&data[at..]) {
        let size = size as usize;
        if size < head || at + size > data.len() {
            break;
        }
        out.push((kind, at + head, &data[at + head..at + size]));
        at += size;
    }
    out
}

fn child<'a>(data: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    children(data).into_iter().find(|(k, _, _)| k == kind).map(|(_, _, body)| body)
}

#[derive(Default)]
pub struct Splitter {
    buf: Vec<u8>,
    init: Vec<u8>,
    moof: Option<Vec<u8>>,
    started: bool,
}

impl Splitter {
    pub fn push(&mut self, data: &[u8]) -> Option<Vec<Out>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        let mut at = 0;
        while let Some((kind, head, size)) = header(&self.buf[at..]) {
            let size = size as usize;
            if size < head {
                return None;
            }
            if at + size > self.buf.len() {
                break;
            }
            let whole = &self.buf[at..at + size];
            match &kind {
                b"moof" => {
                    if !self.started {
                        self.started = true;
                        out.push(Out::Init(std::mem::take(&mut self.init)));
                    }
                    self.moof = Some(whole.to_vec());
                }
                b"mdat" if self.moof.is_some() => {
                    let mut fragment = self.moof.take().unwrap();
                    fragment.extend_from_slice(whole);
                    out.push(Out::Fragment(fragment));
                }
                _ if !self.started => self.init.extend_from_slice(whole),
                _ => {}
            }
            at += size;
        }
        self.buf.drain(..at);
        Some(out)
    }
}

pub fn timescales(init: &[u8]) -> HashMap<u32, u32> {
    let mut out = HashMap::new();
    let Some(moov) = child(init, b"moov") else { return out };
    for (kind, _, trak) in children(moov) {
        if &kind != b"trak" {
            continue;
        }
        let track = child(trak, b"tkhd").and_then(|t| be32(t, if t.first() == Some(&1) { 20 } else { 12 }));
        let scale = child(trak, b"mdia").and_then(|m| child(m, b"mdhd")).and_then(|m| be32(m, if m.first() == Some(&1) { 20 } else { 12 }));
        if let (Some(track), Some(scale)) = (track, scale.filter(|s| *s > 0)) {
            out.insert(track, scale);
        }
    }
    out
}

pub fn fragment_times(moof: &[u8], base: u64) -> Vec<Time> {
    let mut out = Vec::new();
    let Some((kind, head, _)) = header(moof) else { return out };
    if &kind != b"moof" {
        return out;
    }
    for (kind, start, traf) in children(&moof[head..]) {
        if &kind != b"traf" {
            continue;
        }
        let track = child(traf, b"tfhd").and_then(|t| be32(t, 4));
        let tfdt = children(traf).into_iter().find(|(k, _, _)| k == b"tfdt");
        let (Some(track), Some((_, at, body))) = (track, tfdt) else { continue };
        let wide = body.first() == Some(&1);
        let value = if wide { be64(body, 4) } else { be32(body, 4).map(u64::from) };
        if let Some(value) = value {
            out.push(Time { at: base + (head + start + at + 4) as u64, wide, track, value });
        }
    }
    out
}

pub fn micros(time: &Time, scales: &HashMap<u32, u32>) -> i64 {
    let scale = scales.get(&time.track).copied().unwrap_or(1) as i128;
    (time.value as i128 * 1_000_000 / scale) as i64
}

pub fn scan(path: &Path) -> (Vec<Time>, u64) {
    let mut times = Vec::new();
    let Ok(mut file) = File::open(path) else { return (times, 0) };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let (mut at, mut whole, mut moof) = (0u64, 0u64, false);
    let mut head = [0u8; WIDE_HEADER];
    while at + HEADER as u64 <= len {
        let read = (len - at).min(WIDE_HEADER as u64) as usize;
        if file.seek(SeekFrom::Start(at)).is_err() || file.read_exact(&mut head[..read]).is_err() {
            break;
        }
        let Some((kind, size_of_head, size)) = header(&head[..read]) else { break };
        if size < size_of_head as u64 || at + size > len {
            break;
        }
        if &kind == b"moof" {
            let mut body = vec![0u8; size as usize];
            if file.seek(SeekFrom::Start(at)).is_err() || file.read_exact(&mut body).is_err() {
                break;
            }
            times.extend(fragment_times(&body, at));
            moof = true;
        } else if &kind == b"mdat" && moof {
            whole = at + size;
            moof = false;
        }
        at += size;
    }
    (times, whole)
}

pub fn bases(times: &[Time]) -> HashMap<u32, u64> {
    let mut out = HashMap::new();
    for t in times {
        out.entry(t.track).or_insert(t.value);
    }
    out
}

pub fn patch(buf: &mut [u8], start: u64, times: &[Time], bases: &HashMap<u32, u64>) {
    let end = start + buf.len() as u64;
    for t in times {
        let value = t.value.saturating_sub(bases.get(&t.track).copied().unwrap_or(0));
        let bytes = if t.wide { value.to_be_bytes().to_vec() } else { (value as u32).to_be_bytes().to_vec() };
        let field = (t.at, t.at + bytes.len() as u64);
        let (from, to) = (field.0.max(start), field.1.min(end));
        if from >= to {
            continue;
        }
        buf[(from - start) as usize..(to - start) as usize].copy_from_slice(&bytes[(from - field.0) as usize..(to - field.0) as usize]);
    }
}

pub fn cut(open: bool, count: u32, per_segment: u32, current: Option<&str>, wanted: Option<&str>) -> bool {
    !open || count >= per_segment || current != wanted
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn boxed(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + HEADER) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        out
    }

    fn full(kind: &[u8; 4], version: u8, fields: &[u8]) -> Vec<u8> {
        boxed(kind, &[&[version, 0, 0, 0][..], fields].concat())
    }

    pub fn init(tracks: &[(u32, u32)]) -> Vec<u8> {
        let traks: Vec<u8> = tracks
            .iter()
            .flat_map(|(track, scale)| {
                let tkhd = full(b"tkhd", 0, &[&[0u8; 8][..], &track.to_be_bytes(), &[0u8; 4]].concat());
                let mdhd = full(b"mdhd", 0, &[&[0u8; 8][..], &scale.to_be_bytes(), &[0u8; 8]].concat());
                boxed(b"trak", &[tkhd, boxed(b"mdia", &mdhd)].concat())
            })
            .collect();
        [boxed(b"ftyp", b"iso6"), boxed(b"moov", &traks)].concat()
    }

    pub fn fragment(times: &[(u32, u64)], data: &[u8]) -> Vec<u8> {
        let trafs: Vec<u8> = times
            .iter()
            .flat_map(|(track, t)| boxed(b"traf", &[full(b"tfhd", 0, &track.to_be_bytes()), full(b"tfdt", 1, &t.to_be_bytes())].concat()))
            .collect();
        [boxed(b"moof", &[full(b"mfhd", 0, &[0, 0, 0, 1]), trafs].concat()), boxed(b"mdat", data)].concat()
    }

    #[test]
    fn splitter_finds_init_and_whole_fragments_across_reads() {
        let stream = [init(&[(1, 90000)]), fragment(&[(1, 0)], b"aaaa"), fragment(&[(1, 90000)], b"bb"), boxed(b"mfra", b"x")].concat();
        let mut s = Splitter::default();
        let mut outs = Vec::new();
        for chunk in stream.chunks(7) {
            outs.extend(s.push(chunk).unwrap());
        }
        assert_eq!(outs.len(), 3);
        let Out::Init(i) = &outs[0] else { panic!() };
        assert_eq!(timescales(i), HashMap::from([(1, 90000)]));
        let Out::Fragment(f) = &outs[2] else { panic!() };
        assert_eq!(fragment_times(f, 0)[0].value, 90000);
        assert!(Splitter::default().push(&[0, 0, 0, 3, b'b', b'a', b'd', b'!']).is_none());
    }

    #[test]
    fn scan_finds_times_and_the_last_whole_fragment() {
        let dir = crate::log::tests::temp("scan");
        let (a, b) = (fragment(&[(1, 10), (2, 20)], b"aaaa"), fragment(&[(1, 30), (2, 40)], b"bbbb"));
        let path = dir.join("1.m4s");
        std::fs::write(&path, [a.clone(), b.clone(), b[..b.len() - 2].to_vec()].concat()).unwrap();
        let (times, whole) = scan(&path);
        assert_eq!(whole, (a.len() + b.len()) as u64);
        assert_eq!(times.iter().map(|t| (t.track, t.value)).collect::<Vec<_>>(), [(1, 10), (2, 20), (1, 30), (2, 40), (1, 30), (2, 40)]);
        assert_eq!(times[2].at, a.len() as u64 + times[0].at);
        assert_eq!(micros(&times[0], &HashMap::from([(1, 1000)])), 10_000);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn patch_moves_times_to_zero_even_in_partial_ranges() {
        let data = [fragment(&[(1, 900), (2, 50)], b"aa"), fragment(&[(1, 990), (2, 60)], b"bb")].concat();
        let times: Vec<Time> = [fragment_times(&data, 0), fragment_times(&data[data.len() / 2..], (data.len() / 2) as u64)].concat();
        let base = bases(&times);
        let mut whole = data.clone();
        patch(&mut whole, 0, &times, &base);
        let after: Vec<u64> = [fragment_times(&whole, 0), fragment_times(&whole[data.len() / 2..], 0)].concat().iter().map(|t| t.value).collect();
        assert_eq!(after, [0, 0, 90, 10]);
        let at = times[2].at as usize + 3;
        let mut part = data[at..at + 9].to_vec();
        patch(&mut part, at as u64, &times, &base);
        assert_eq!(part, whole[at..at + 9]);
    }

    #[test]
    fn segments_cut_on_count_and_on_set_changes() {
        assert!(cut(false, 0, 4, None, None));
        assert!(!cut(true, 3, 4, None, None));
        assert!(cut(true, 4, 4, None, None));
        assert!(cut(true, 1, 4, None, Some("s")));
        assert!(cut(true, 1, 4, Some("s"), None));
        assert!(!cut(true, 2, 4, Some("s"), Some("s")));
    }
}

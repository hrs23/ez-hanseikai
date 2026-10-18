use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde_json::Value;

pub fn parse(bytes: &[u8]) -> (Vec<Value>, usize) {
    let mut records = Vec::new();
    let mut at = 0usize;
    while at + 8 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap());
        let Some(body) = bytes.get(at + 8..at + 8 + len) else { break };
        if crc32fast::hash(body) != crc {
            break;
        }
        let Ok(v) = serde_json::from_slice(body) else { break };
        records.push(v);
        at += 8 + len;
    }
    (records, at)
}

pub fn scan(path: &Path, from: u64) -> (Vec<Value>, u64) {
    let mut bytes = Vec::new();
    if let Ok(mut f) = File::open(path) {
        if f.seek(SeekFrom::Start(from)).is_ok() {
            let _ = f.read_to_end(&mut bytes);
        }
    }
    let (records, used) = parse(&bytes);
    (records, used as u64)
}

pub fn repair(path: &Path) {
    let (_, valid) = scan(path, 0);
    if let Ok(f) = OpenOptions::new().write(true).open(path) {
        let _ = f.set_len(valid);
    }
}

pub fn encode(record: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(record).unwrap_or_default();
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&crc32fast::hash(&body).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

pub fn append(path: &Path, record: &Value) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    OpenOptions::new().create(true).append(true).open(path)?.write_all(&encode(record))
}

pub fn extend(path: &Path, from: u64, data: &[u8]) -> std::io::Result<()> {
    let whole = parse(data).1;
    if whole == 0 {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if file.metadata()?.len() != from {
        return Ok(());
    }
    file.write_all(&data[..whole])
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;

    pub fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ezh-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn append_scan_and_repair_a_torn_tail() {
        let dir = temp("log");
        let path = dir.join("a").join("log.bin");
        append(&path, &json!({"k": "x", "n": 1})).unwrap();
        append(&path, &json!({"k": "x", "n": 2})).unwrap();
        let whole = std::fs::metadata(&path).unwrap().len();
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        f.write_all(&encode(&json!({"k": "x", "n": 3}))[..5]).unwrap();
        let (records, used) = scan(&path, 0);
        assert_eq!(records.len(), 2);
        assert_eq!(used, whole);
        repair(&path);
        assert_eq!(std::fs::metadata(&path).unwrap().len(), whole);
        let (rest, _) = scan(&path, encode(&json!({"k": "x", "n": 1})).len() as u64);
        assert_eq!(rest, vec![json!({"k": "x", "n": 2})]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn extend_takes_whole_records_at_the_right_offset_only() {
        let dir = temp("extend");
        let path = dir.join("log.bin");
        let (a, b) = (encode(&json!({"n": 1})), encode(&json!({"n": 2})));
        let both = [a.clone(), b.clone()].concat();
        extend(&path, 0, &both[..a.len() + 3]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), a);
        extend(&path, 0, &b).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), a);
        extend(&path, a.len() as u64, &both[3..]).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), a);
        extend(&path, a.len() as u64, &b).unwrap();
        assert_eq!(scan(&path, 0).0.len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }
}

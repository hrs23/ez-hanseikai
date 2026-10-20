use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde_json::Value;

pub const HEADER: usize = 8;

pub fn parse(bytes: &[u8]) -> (Vec<Value>, usize) {
    let mut records = Vec::new();
    let mut at = 0usize;
    while at + HEADER <= bytes.len() {
        let len = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let crc = u32::from_le_bytes(bytes[at + 4..at + HEADER].try_into().unwrap());
        let Some(body) = bytes.get(at + HEADER..at + HEADER + len) else { break };
        if crc32fast::hash(body) != crc {
            break;
        }
        let Ok(v) = serde_json::from_slice(body) else { break };
        records.push(v);
        at += HEADER + len;
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
    let mut out = Vec::with_capacity(body.len() + HEADER);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&crc32fast::hash(&body).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

pub fn append(path: &Path, record: &Value) -> std::io::Result<()> {
    append_bytes(path, &encode(record))
}

pub fn append_bytes(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let len = file.metadata()?.len();
    file.write_all(bytes).inspect_err(|_| {
        let _ = OpenOptions::new().write(true).open(path).and_then(|f| f.set_len(len));
    })
}

pub fn extend(path: &Path, from: u64, data: &[u8]) -> std::io::Result<()> {
    let whole = parse(data).1;
    if whole == 0 {
        return Ok(());
    }
    if std::fs::metadata(path).map_or(0, |m| m.len()) != from {
        return Ok(());
    }
    append_bytes(path, &data[..whole])
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

    pub fn put(dir: &Path, name: &str, body: &[u8]) {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    pub fn seed<'a>(dir: &Path, names: impl IntoIterator<Item = &'a str>) {
        for name in names {
            put(dir, name, b"x");
        }
    }

    pub fn present<'a>(dir: &Path, names: &[&'a str]) -> Vec<&'a str> {
        names.iter().copied().filter(|name| dir.join(name).exists()).collect()
    }

    pub fn tree(dir: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut out = std::collections::BTreeMap::new();
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(tree(&path).into_iter().map(|(name, body)| (format!("{}/{name}", entry.file_name().to_string_lossy()), body)));
            } else {
                out.insert(entry.file_name().to_string_lossy().to_string(), std::fs::read(&path).unwrap());
            }
        }
        out
    }

    pub fn hold(path: &Path) -> std::fs::File {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::OpenOptions::new().read(true).share_mode(::windows::Win32::Storage::FileSystem::FILE_SHARE_DELETE.0).open(path).unwrap()
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

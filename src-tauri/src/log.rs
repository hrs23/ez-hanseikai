use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;

use serde_json::Value;

fn scan(path: &Path) -> (Vec<Value>, u64) {
    let mut bytes = Vec::new();
    if let Ok(mut f) = File::open(path) {
        let _ = f.read_to_end(&mut bytes);
    }
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
    (records, at as u64)
}

pub fn read(path: &Path) -> Vec<Value> {
    scan(path).0
}

pub fn repair(path: &Path) {
    let (_, valid) = scan(path);
    if let Ok(f) = OpenOptions::new().write(true).open(path) {
        let _ = f.set_len(valid);
    }
}

pub fn append(path: &Path, record: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(record)?;
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&crc32fast::hash(&body).to_le_bytes());
    out.extend_from_slice(&body);
    OpenOptions::new().create(true).append(true).open(path)?.write_all(&out)
}

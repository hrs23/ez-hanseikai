use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{data, log, net, now_us, record, short, Core, Peer};

pub const SOLO: &str = "solo";

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub members: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Rooms {
    pub active: Option<String>,
    pub list: Vec<Room>,
}

#[derive(Clone)]
pub struct ActiveSet {
    pub id: String,
    pub room: String,
    pub by: String,
    pub start_us: i64,
}

pub fn random_hex(bytes: usize) -> String {
    iroh::SecretKey::generate().to_bytes()[..bytes].iter().map(|b| format!("{b:02x}")).collect()
}

pub fn valid_id(text: &str) -> bool {
    !text.is_empty() && text.len() <= 40 && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ' ')
}

fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub fn set_id_for(now_us: i64, tz_offset_s: i64) -> String {
    let secs = now_us / 1_000_000 + tz_offset_s;
    let (y, m, d) = civil(secs.div_euclid(86400));
    let rest = secs.rem_euclid(86400);
    format!("{y:04}-{m:02}-{d:02} {:02}-{:02}-{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

impl Core {
    pub fn active_room(&self) -> Option<Room> {
        let rooms = self.rooms.lock().unwrap();
        let id = rooms.active.as_ref()?;
        rooms.list.iter().find(|r| &r.id == id).cloned()
    }

    pub fn active_room_id(&self) -> String {
        self.active_room().map(|r| r.id).unwrap_or_else(|| SOLO.to_string())
    }

    pub fn set_dir(&self, room: &str, set: &str) -> PathBuf {
        self.dir.join("rooms").join(room).join("sets").join(set)
    }

    pub fn live_dir(&self, room: &str) -> PathBuf {
        self.dir.join("rooms").join(room).join("live")
    }

    pub fn save_rooms(&self) {
        let text = serde_json::to_string(&*self.rooms.lock().unwrap()).unwrap_or_default();
        let _ = std::fs::write(self.dir.join("rooms.json"), text);
    }

    pub fn active_set_json(&self) -> Value {
        match self.active_set.lock().unwrap().as_ref() {
            Some(s) => json!({"id": s.id, "room": s.room, "by": s.by, "start_us": s.start_us}),
            None => Value::Null,
        }
    }
}

pub fn apply_active(core: &Arc<Core>) {
    let wanted: Vec<EndpointId> = core
        .active_room()
        .map(|r| r.members)
        .unwrap_or_default()
        .iter()
        .filter_map(|m| m.parse().ok())
        .filter(|id| *id != core.ep.id())
        .collect();
    let mut fresh = Vec::new();
    {
        let mut peers = core.peers.lock().unwrap();
        peers.retain(|p| {
            let keep = wanted.contains(&p.id);
            if !keep {
                for conn in p.conn.iter().chain(p.incoming.iter()) {
                    conn.close(2u32.into(), b"left");
                }
            }
            keep
        });
        for id in &wanted {
            if !peers.iter().any(|p| p.id == *id) {
                let announce = core.ready.load(std::sync::atomic::Ordering::Relaxed);
                peers.push(Peer { id: *id, name: String::new(), conn: None, incoming: None, offset_us: None, failures: 0, announce, codecs: vec!["h264".to_string()], info: None, sharing: false, live: None, have: HashSet::new(), gone: HashSet::new() });
                fresh.push(*id);
            }
        }
    }
    for id in fresh {
        tauri::async_runtime::spawn(net::dial_loop(core.clone(), id));
    }
}

pub fn create_room(core: &Arc<Core>, name: &str) -> String {
    let id = random_hex(4);
    let name = if name.trim().is_empty() { format!("{}'s room", core.name.lock().unwrap()) } else { name.trim().chars().take(40).collect() };
    {
        let mut rooms = core.rooms.lock().unwrap();
        rooms.list.push(Room { id: id.clone(), name, members: vec![core.ep.id().to_string()] });
        rooms.active = Some(id.clone());
    }
    core.save_rooms();
    apply_active(core);
    id
}

pub fn switch_room(core: &Arc<Core>, id: &str) -> Result<(), String> {
    if core.active_set.lock().unwrap().is_some() {
        return Err("Stop recording before switching rooms.".into());
    }
    {
        let mut rooms = core.rooms.lock().unwrap();
        if !rooms.list.iter().any(|r| r.id == id) {
            return Err("That room is not in your list.".into());
        }
        rooms.active = Some(id.to_string());
    }
    core.save_rooms();
    apply_active(core);
    Ok(())
}

pub fn leave_room(core: &Arc<Core>, id: &str) {
    {
        let mut rooms = core.rooms.lock().unwrap();
        rooms.list.retain(|r| r.id != id);
        if rooms.active.as_deref() == Some(id) {
            rooms.active = None;
        }
    }
    core.save_rooms();
    apply_active(core);
}

pub fn add_member(core: &Arc<Core>, room: &str, id: &str) {
    let Ok(parsed) = id.parse::<EndpointId>() else { return };
    if parsed == core.ep.id() || core.removed.lock().unwrap().contains(id) {
        return;
    }
    let changed = {
        let mut rooms = core.rooms.lock().unwrap();
        match rooms.list.iter_mut().find(|r| r.id == room) {
            Some(r) if !r.members.iter().any(|m| m == id) => {
                r.members.push(id.to_string());
                true
            }
            _ => false,
        }
    };
    if changed {
        core.save_rooms();
        if core.active_room_id() == room {
            apply_active(core);
        }
    }
}

pub fn remove_member(core: &Arc<Core>, id: &str) {
    core.removed.lock().unwrap().insert(id.to_string());
    core.save_settings();
    {
        let mut rooms = core.rooms.lock().unwrap();
        if let Some(active) = rooms.active.clone() {
            if let Some(r) = rooms.list.iter_mut().find(|r| r.id == active) {
                r.members.retain(|m| m != id);
            }
        }
    }
    core.save_rooms();
    apply_active(core);
}

pub fn adopt_room(core: &Arc<Core>, pending: &str, id: &str, name: &str, members: &[String]) {
    {
        let mut rooms = core.rooms.lock().unwrap();
        rooms.list.retain(|r| r.id != id);
        if let Some(r) = rooms.list.iter_mut().find(|r| r.id == pending) {
            r.id = id.to_string();
            r.name = name.chars().take(40).collect();
        }
        if rooms.active.as_deref() == Some(pending) {
            rooms.active = Some(id.to_string());
        }
    }
    core.save_rooms();
    for m in members {
        add_member(core, id, m);
    }
}

pub fn write_set_json(core: &Core, set: &ActiveSet, ended: bool) {
    let mut me = json!({"id": core.ep.id().to_string(), "name": *core.name.lock().unwrap()});
    if let Some(rec) = core.own_log().into_iter().find(|r| r["k"] == "rec" && r["set"] == set.id.as_str()) {
        for key in ["source_kind", "source", "width", "height", "fps", "encoder", "codec", "audio", "mic"] {
            me[key] = rec[key].clone();
        }
    }
    let peers: Vec<Value> = core.peers.lock().unwrap().iter().map(|p| json!({"id": p.id.to_string(), "name": p.name, "offset_us": p.offset_us})).collect();
    let value = json!({
        "id": set.id, "room": set.room, "by": set.by, "start_us": set.start_us, "ended": ended,
        "members": [me],
        "peers": peers,
        "layout": *core.layout.lock().unwrap(),
    });
    let _ = std::fs::write(core.set_dir(&set.room, &set.id).join("set.json"), value.to_string());
}

pub fn begin_set(core: &Arc<Core>, set: &str, by: &str, start_us: i64) -> Result<(), String> {
    if !record::ffmpeg_present() {
        return Err("The video tool is still downloading. Wait a minute and try again.".into());
    }
    let room = core.active_room_id();
    if core.is_deleted(&room, set) {
        return Err("That set was deleted on this PC.".into());
    }
    std::fs::create_dir_all(core.set_dir(&room, set)).map_err(|e| e.to_string())?;
    let active = ActiveSet { id: set.to_string(), room, by: by.to_string(), start_us };
    {
        let mut slot = core.active_set.lock().unwrap();
        if slot.is_some() {
            return Err("already recording".into());
        }
        *slot = Some(active.clone());
    }
    write_set_json(core, &active, false);
    Ok(())
}

pub fn end_set(core: &Arc<Core>) {
    let Some(active) = record::end_set(core) else { return };
    data::record_file(core, &active.room, &active.id);
    core.append_log(&json!({"k": "rec_end", "set": active.id, "room": active.room, "t_us": now_us()}));
    write_set_json(core, &active, true);
}

pub fn start_set(core: &Arc<Core>) -> Result<(), String> {
    let start_us = now_us();
    let set = set_id_for(start_us, core.tz_offset_s);
    let by = core.name.lock().unwrap().clone();
    begin_set(core, &set, &by, start_us)?;
    net::broadcast(core, json!({"t": "rec", "action": "start", "set": set, "room": core.active_room_id(), "by": by, "start_us": start_us}));
    Ok(())
}

pub fn stop_set(core: &Arc<Core>) {
    let set = core.active_set.lock().unwrap().as_ref().map(|s| s.id.clone());
    end_set(core);
    if let Some(set) = set {
        net::broadcast(core, json!({"t": "rec", "action": "stop", "set": set, "room": core.active_room_id()}));
    }
}

pub struct Log {
    pub owner: String,
    pub offset_us: i64,
    pub records: Vec<Value>,
}

pub struct File {
    pub owner: String,
    pub start_us: i64,
    pub end_us: Option<i64>,
    pub offset_us: i64,
    pub total: Option<u64>,
    pub sha256: Option<String>,
    pub rec: Value,
}

pub struct Set {
    pub id: String,
    pub room: String,
    pub files: Vec<File>,
}

impl Set {
    pub fn start(&self) -> i64 {
        self.files.iter().map(|f| f.start_us - f.offset_us).min().unwrap_or(0)
    }

    pub fn end(&self) -> Option<i64> {
        self.files.iter().map(|f| f.end_us.map(|e| e - f.offset_us)).collect::<Option<Vec<i64>>>()?.into_iter().max()
    }

    pub fn covers(&self, t: i64) -> bool {
        self.start() <= t && self.end().is_none_or(|end| t <= end)
    }
}

pub fn have_key(set: &str, owner: &str) -> String {
    format!("{set}/{}", owner.chars().take(10).collect::<String>())
}

pub fn logs(core: &Core, room: &str) -> Vec<Log> {
    let own = core.own_log();
    let mut offsets: HashMap<String, i64> =
        own.iter().filter(|r| r["k"] == "offset").filter_map(|r| Some((r["peer"].as_str()?.to_string(), r["offset_us"].as_i64()?))).collect();
    for p in core.peers.lock().unwrap().iter() {
        if let Some(offset) = p.offset_us {
            offsets.insert(short(&p.id), offset);
        }
    }
    let members = core.rooms.lock().unwrap().list.iter().find(|r| r.id == room).map(|r| r.members.clone()).unwrap_or_default();
    let me = core.ep.id().to_string();
    let mut out = vec![Log { owner: me.clone(), offset_us: 0, records: own }];
    for id in members.iter().filter(|m| **m != me).filter_map(|m| m.parse::<EndpointId>().ok()) {
        let records = log::read(&core.peer_dir(&id).join("log.bin"));
        out.push(Log { owner: id.to_string(), offset_us: offsets.get(&short(&id)).copied().unwrap_or(0), records });
    }
    out
}

pub fn sets(core: &Core, room: &str, logs: &[Log]) -> Vec<Set> {
    let mut sets: BTreeMap<String, Set> = BTreeMap::new();
    for log in logs {
        for (i, r) in log.records.iter().enumerate() {
            let Some(id) = r["set"].as_str().filter(|s| r["k"] == "rec" && r["room"] == room && valid_id(s)) else { continue };
            if core.is_deleted(room, id) || sets.get(id).is_some_and(|s| s.files.iter().any(|f| f.owner == log.owner)) {
                continue;
            }
            let find = |kind: &str| log.records[i..].iter().find(|e| e["k"] == kind && e["set"] == id);
            let file = find("file");
            let set = sets.entry(id.to_string()).or_insert_with(|| Set { id: id.to_string(), room: room.to_string(), files: Vec::new() });
            set.files.push(File {
                owner: log.owner.clone(),
                start_us: r["start_us"].as_i64().unwrap_or(0),
                end_us: find("rec_end").and_then(|e| e["t_us"].as_i64()),
                offset_us: log.offset_us,
                total: file.and_then(|f| f["size"].as_u64()),
                sha256: file.and_then(|f| f["sha256"].as_str().map(String::from)),
                rec: r.clone(),
            });
        }
    }
    sets.into_values().rev().collect()
}

pub fn notes(logs: &[Log], room: &str) -> Vec<Value> {
    let old: HashSet<&str> = logs.iter().flat_map(|l| &l.records).filter(|r| r["k"] == "rec" && r["room"] == room).filter_map(|r| r["set"].as_str()).collect();
    let mut out = Vec::new();
    for log in logs {
        let mut notes: Vec<Value> = Vec::new();
        for r in &log.records {
            let id = r["id"].as_i64().or_else(|| r["t_us"].as_i64()).unwrap_or(0);
            let existing = notes.iter().position(|n| n["id"] == id);
            let here = r["room"] == room || r["room"].is_null() && r["set"].as_str().is_some_and(|s| old.contains(s));
            match (r["k"].as_str(), existing) {
                (Some("note"), None) if here => {
                    let at = r["at_us"].as_i64().unwrap_or(id);
                    notes.push(json!({"key": format!("{}/{id}", log.owner), "id": id, "author": log.owner, "at_us": at, "local_us": at - log.offset_us, "text": r["text"], "deleted": false}));
                }
                (Some("note_edit"), Some(i)) => notes[i]["text"] = r["text"].clone(),
                (Some("note_del"), Some(i)) => notes[i]["deleted"] = json!(true),
                (Some("note_restore"), Some(i)) => notes[i]["deleted"] = json!(false),
                _ => {}
            }
        }
        out.extend(notes.into_iter().filter(|n| n["deleted"] == false));
    }
    out.sort_by_key(|n| n["local_us"].as_i64());
    out
}

pub fn have_list(core: &Core, room: &str) -> Vec<String> {
    let Ok(sets) = std::fs::read_dir(core.dir.join("rooms").join(room).join("sets")) else { return Vec::new() };
    let mut out = Vec::new();
    for set in sets.flatten() {
        let id = set.file_name().to_string_lossy().to_string();
        for file in data::verified_files(&set.path()) {
            out.push(have_key(&id, file.trim_end_matches(".mp4")));
        }
    }
    out
}

pub fn gone_list(core: &Core, room: &str) -> Vec<String> {
    core.deleted.lock().unwrap().iter().filter_map(|k| k.strip_prefix(&format!("{room}/")).map(String::from)).collect()
}

pub fn someone_else_has(core: &Core, set: &str, owner: &str) -> bool {
    let key = have_key(set, owner);
    core.peers.lock().unwrap().iter().any(|p| p.have.contains(&key))
}

pub fn delete_set(core: &Core, room: &str, set: &str) -> Result<(), String> {
    if !valid_id(room) || !valid_id(set) {
        return Err("Unknown set.".into());
    }
    if core.active_set.lock().unwrap().as_ref().is_some_and(|a| a.id == set && a.room == room) {
        return Err("This set is being recorded. Stop it first.".into());
    }
    core.deleted.lock().unwrap().insert(format!("{room}/{set}"));
    core.save_settings();
    let _ = std::fs::remove_dir_all(core.set_dir(room, set));
    Ok(())
}

struct Holder {
    key: String,
    name: String,
    have: HashSet<String>,
    gone: bool,
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

pub fn sets_json(core: &Core, room: &str, logs: &[Log], notes: &mut [Value]) -> Vec<Value> {
    let me = core.ep.id().to_string();
    let peers: Vec<(String, String, HashSet<String>, HashSet<String>)> =
        core.peers.lock().unwrap().iter().map(|p| (p.id.to_string(), if p.name.is_empty() { short(&p.id) } else { p.name.clone() }, p.have.clone(), p.gone.clone())).collect();
    let mut out = Vec::new();
    for set in sets(core, room, logs) {
        let dir = core.set_dir(room, &set.id);
        let saved: Value = std::fs::read_to_string(dir.join("set.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let mine: HashSet<String> = data::verified_files(&dir).iter().map(|f| have_key(&set.id, f.trim_end_matches(".mp4"))).collect();
        let mut holders = vec![Holder { key: "me".into(), name: "You".into(), have: mine, gone: false }];
        holders.extend(peers.iter().map(|(id, name, have, gone)| Holder { key: id.clone(), name: name.clone(), have: have.clone(), gone: gone.contains(&set.id) }));
        let key_of = |owner: &str| if owner == me { "me".to_string() } else { owner.to_string() };
        let mut players = Vec::new();
        let (mut have, mut total) = (0u64, 0u64);
        for f in &set.files {
            let path = dir.join(format!("{}.mp4", f.owner));
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            have += f.total.map_or(size, |t| size.min(t));
            total += f.total.unwrap_or(size);
            let hk = have_key(&set.id, &f.owner);
            let by: Vec<&str> = holders.iter().filter(|h| h.have.contains(&hk)).map(|h| h.key.as_str()).collect();
            players.push(json!({
                "key": key_of(&f.owner), "owner": f.owner, "start_us": f.start_us, "offset_us": f.offset_us, "size": size, "total": f.total, "path": path,
                "recording": f.end_us.is_none(), "by": f.rec["by"], "verified": by.contains(&"me"), "verified_by": by, "sha256": f.sha256,
                "meta": {"encoder": f.rec["encoder"], "codec": f.rec["codec"], "width": f.rec["width"], "height": f.rec["height"]},
            }));
        }
        let finished: Vec<&File> = set.files.iter().filter(|f| f.total.is_some()).collect();
        let missing: Vec<Value> = holders
            .iter()
            .filter(|h| !h.gone)
            .filter_map(|h| {
                let lack: Vec<String> = finished.iter().filter(|f| !h.have.contains(&have_key(&set.id, &f.owner))).map(|f| key_of(&f.owner)).collect();
                (!lack.is_empty()).then(|| json!({"key": h.key, "name": h.name, "files": lack}))
            })
            .collect();
        let verified_by: Vec<&str> = holders.iter().filter(|h| !finished.is_empty() && finished.iter().all(|f| h.have.contains(&have_key(&set.id, &f.owner)))).map(|h| h.key.as_str()).collect();
        let live = set.end().is_none();
        let all_verified = missing.is_empty() && finished.len() == set.files.len();
        let status = if live {
            "Recording".to_string()
        } else if all_verified {
            "Everyone has every file".to_string()
        } else if missing.is_empty() {
            "Waiting for file checks".to_string()
        } else {
            missing.iter().map(|m| { let n = m["files"].as_array().map_or(0, |a| a.len()); format!("{}: {n} file{} left", m["name"].as_str().unwrap_or_default(), plural(n)) }).collect::<Vec<_>>().join(", ")
        };
        let mut covered = Vec::new();
        for n in notes.iter_mut().filter(|n| n["local_us"].as_i64().is_some_and(|t| set.covers(t))) {
            covered.push(n["key"].clone());
            n["sets"].as_array_mut().unwrap().push(json!(set.id));
            if have > 0 {
                n["seekable"] = json!(true);
            }
        }
        out.push(json!({
            "id": set.id, "room": room, "start_us": set.start(), "end_us": set.end(), "live": live, "by": players.first().map(|p| p["by"].clone()),
            "players": players, "notes": covered, "progress": {"have": have, "total": total}, "verified_by": verified_by, "missing": missing, "all_verified": all_verified, "status": status, "layout": saved["layout"],
        }));
    }
    out
}

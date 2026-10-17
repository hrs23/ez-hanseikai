use std::path::PathBuf;
use std::sync::Arc;

use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{net, now_us, record, short, Core, Peer};

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
                peers.push(Peer { id: *id, name: String::new(), conn: None, incoming: None, offset_us: None, failures: 0, announce, codecs: vec!["h264".to_string()], info: None, sharing: false, live: None });
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
    });
    let _ = std::fs::write(core.set_dir(&set.room, &set.id).join("set.json"), value.to_string());
}

pub fn begin_set(core: &Arc<Core>, set: &str, by: &str, start_us: i64) -> Result<(), String> {
    if !record::ffmpeg_present() {
        return Err("The video tool is still downloading. Wait a minute and try again.".into());
    }
    let room = core.active_room_id();
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
    core.append_log(&json!({"k": "rec_end", "set": active.id, "t_us": now_us()}));
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

pub fn label(id: &EndpointId) -> String {
    short(id)
}

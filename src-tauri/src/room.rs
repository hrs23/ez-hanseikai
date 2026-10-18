use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use iroh::endpoint::Connection;
use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use tauri::Emitter;

use crate::{clean_text, data, ffmpeg, file_len, hex, net, now_us, party, record, short, Core, Peer, PROTOCOL, ROOM_NAME_MAX, VERSION};

pub const SOLO: &str = "solo";

#[derive(Clone, Serialize, Deserialize, Default)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub members: Vec<String>,
    #[serde(default)]
    pub names: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite: Option<(String, String)>,
    #[serde(default)]
    pub removed: Vec<String>,
    #[serde(skip)]
    pub waiting: bool,
}

impl Room {
    pub fn people(&self) -> Vec<String> {
        self.members.iter().chain(self.removed.iter()).cloned().collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Ask {
    pub token: String,
    pub id: String,
    pub name: String,
    pub answer: Option<bool>,
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
    hex(&iroh::SecretKey::generate().to_bytes()[..bytes])
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

    pub fn room_id(&self) -> Option<String> {
        let rooms = self.rooms.lock().unwrap();
        rooms.active.clone().filter(|id| rooms.list.iter().any(|r| &r.id == id))
    }

    pub fn active_room_id(&self) -> String {
        self.room_id().unwrap_or_else(|| SOLO.to_string())
    }

    pub fn set_dir(&self, room: &str, set: &str) -> PathBuf {
        self.dir.join("rooms").join(room).join("sets").join(set)
    }

    pub fn set_file(&self, room: &str, set: &str, owner: &str) -> PathBuf {
        self.set_dir(room, set).join(format!("{owner}.mp4"))
    }

    pub fn own_file(&self, room: &str, set: &str) -> PathBuf {
        self.set_file(room, set, &self.ep.id().to_string())
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

    pub fn is_recording(&self, room: &str, set: &str) -> bool {
        self.active_set.lock().unwrap().as_ref().is_some_and(|a| a.id == set && a.room == room)
    }
}

fn idle(core: &Core) -> Result<(), String> {
    if core.active_set.lock().unwrap().is_some() { Err("Stop recording before switching rooms.".into()) } else { Ok(()) }
}

pub fn display_name(core: &Core, id: &str) -> String {
    if id == core.ep.id().to_string() {
        return core.name.lock().unwrap().clone();
    }
    let peer = core.peers.lock().unwrap().iter().find(|p| p.id.to_string() == id && !p.name.is_empty()).map(|p| p.name.clone());
    peer.or_else(|| core.active_room().and_then(|r| r.names.get(id).cloned())).unwrap_or_else(|| short(id))
}

pub fn remember_name(core: &Core, id: &str, name: &str, replace: bool) {
    if name.is_empty() {
        return;
    }
    let mut changed = false;
    for r in core.rooms.lock().unwrap().list.iter_mut().filter(|r| r.members.iter().any(|m| m == id)) {
        if r.names.get(id).is_none_or(|old| replace && old != name) {
            r.names.insert(id.to_string(), name.to_string());
            changed = true;
        }
    }
    if changed {
        core.save_rooms();
    }
    if let Ok(id) = id.parse::<EndpointId>() {
        core.update_peer(&id, |p| {
            if p.name.is_empty() || replace {
                p.name = name.to_string();
            }
        });
    }
}

pub fn apply_active(core: &Arc<Core>) {
    let room = core.active_room().unwrap_or_default();
    let wanted: Vec<EndpointId> = room.members.iter().filter_map(|m| m.parse().ok()).filter(|id| *id != core.ep.id()).collect();
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
                peers.push(Peer::new(*id, room.names.get(&id.to_string()).cloned().unwrap_or_default()));
                fresh.push(*id);
            }
        }
    }
    for id in fresh {
        tauri::async_runtime::spawn(net::dial_loop(core.clone(), id));
    }
}

fn activate(core: &Arc<Core>, room: Room) {
    {
        let mut rooms = core.rooms.lock().unwrap();
        rooms.list.retain(|r| r.id != room.id);
        rooms.active = Some(room.id.clone());
        rooms.list.push(room);
    }
    core.save_rooms();
    apply_active(core);
}

pub fn create_room(core: &Arc<Core>, name: &str) -> Result<String, String> {
    idle(core)?;
    let id = random_hex(4);
    let name = match clean_text(name, ROOM_NAME_MAX) {
        n if n.is_empty() => format!("{}'s room", core.name.lock().unwrap()),
        n => n,
    };
    activate(core, Room { id: id.clone(), name, members: vec![core.ep.id().to_string()], ..Default::default() });
    Ok(id)
}

pub fn join_room(core: &Arc<Core>, code: &str) -> Result<(), String> {
    let bad = || "That invite code is not valid. Ask for a new one.".to_string();
    let text: String = code.split_whitespace().collect();
    let (id, token) = text.strip_prefix("ezh-").and_then(|t| t.split_once('-')).ok_or_else(bad)?;
    id.parse::<EndpointId>().map_err(|_| bad())?;
    idle(core)?;
    let members = vec![core.ep.id().to_string(), id.to_string()];
    let invite = Some((id.to_string(), token.to_string()));
    activate(core, Room { id: format!("pending-{token}"), name: "Joining...".into(), members, invite, ..Default::default() });
    Ok(())
}

pub fn switch_room(core: &Arc<Core>, id: &str) -> Result<(), String> {
    idle(core)?;
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
    if parsed == core.ep.id() {
        return;
    }
    let changed = {
        let mut rooms = core.rooms.lock().unwrap();
        match rooms.list.iter_mut().find(|r| r.id == room) {
            Some(r) if !r.members.iter().any(|m| m == id) && !r.removed.iter().any(|m| m == id) => {
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
    let room = core.active_room_id();
    if id.parse::<EndpointId>().is_err() || id == core.me() {
        return;
    }
    core.append_log(&room, &json!({"k": "remove", "member": id, "t_us": now_us()}));
    net::broadcast(core, json!({"t": "remove", "room": room, "member": id}));
    apply_removals(core);
}

pub fn shares(room: &Room, asker: &str, of: &str) -> bool {
    room.invite.is_none() && room.members.iter().any(|m| m == asker) && room.people().iter().any(|m| m == of)
}

pub fn may_share(core: &Core, room: &str, asker: &str, of: &str) -> bool {
    valid_id(room) && of.parse::<EndpointId>().is_ok() && core.rooms.lock().unwrap().list.iter().find(|r| r.id == room).is_some_and(|r| shares(r, asker, of))
}

#[derive(Debug, PartialEq)]
pub enum Step {
    Refused,
    Wait(bool),
    Declined,
    Admitted,
}

pub fn ask_step(invites: &mut Vec<(String, String)>, asks: &mut Vec<Ask>, room: &str, token: &str, id: &str, name: &str) -> Step {
    if !invites.iter().any(|(t, r)| t == token && r == room) {
        return Step::Refused;
    }
    let Some(i) = asks.iter().position(|a| a.token == token) else {
        asks.push(Ask { token: token.to_string(), id: id.to_string(), name: name.to_string(), answer: None });
        return Step::Wait(true);
    };
    if asks[i].id != id {
        return Step::Refused;
    }
    match asks[i].answer {
        None => {
            let changed = asks[i].name != name;
            asks[i].name = name.to_string();
            Step::Wait(changed)
        }
        Some(approve) => {
            asks.remove(i);
            invites.retain(|(t, _)| t != token);
            if approve { Step::Admitted } else { Step::Declined }
        }
    }
}

pub fn answer_ask(asks: &mut [Ask], id: &str, approve: bool) -> bool {
    match asks.iter_mut().find(|a| a.id == id && a.answer.is_none()) {
        Some(a) => {
            a.answer = Some(approve);
            true
        }
        None => false,
    }
}

pub fn admit(core: &Arc<Core>, room: &str, id: &str) {
    if let Some(r) = core.rooms.lock().unwrap().list.iter_mut().find(|r| r.id == room) {
        r.removed.retain(|m| m != id);
    }
    core.append_log(room, &json!({"k": "admit", "member": id, "t_us": now_us()}));
    add_member(core, room, id);
}

pub fn removals(logs: &[Log]) -> HashMap<String, (i64, bool, String)> {
    let mut latest: HashMap<String, (i64, bool, String)> = HashMap::new();
    for log in logs {
        for r in log.records.iter() {
            let (Some(k @ ("remove" | "admit")), Some(member), Some(t)) = (r["k"].as_str(), r["member"].as_str(), r["t_us"].as_i64()) else { continue };
            let t = t - log.offset_us;
            if latest.get(member).is_none_or(|(old, _, _)| t > *old) {
                latest.insert(member.to_string(), (t, k == "remove", log.owner.clone()));
            }
        }
    }
    latest
}

pub fn apply_removals(core: &Arc<Core>) {
    let room_id = core.active_room_id();
    let latest = removals(&logs(core, &room_id));
    let me = core.me();
    let mut news = Vec::new();
    let mut changed = false;
    {
        let mut rooms = core.rooms.lock().unwrap();
        let Some(r) = rooms.list.iter_mut().find(|r| r.id == room_id) else { return };
        for (member, (_, removed, by)) in latest {
            let listed = r.removed.contains(&member);
            if removed && !listed {
                if r.members.contains(&member) {
                    news.push((by, member.clone()));
                }
                if member != me {
                    r.members.retain(|m| *m != member);
                }
                r.removed.push(member);
                changed = true;
            } else if !removed && listed {
                r.removed.retain(|m| *m != member);
                if !r.members.contains(&member) {
                    r.members.push(member);
                }
                changed = true;
            }
        }
    }
    if !changed {
        return;
    }
    core.save_rooms();
    apply_active(core);
    for (by, member) in news {
        let who = if member == me { "you".to_string() } else { display_name(core, &member) };
        let _ = core.app.emit("flash", format!("{} removed {who}", display_name(core, &by)));
    }
}

pub fn adopt_room(core: &Arc<Core>, pending: &str, id: &str, name: &str, members: &[String]) {
    {
        let mut rooms = core.rooms.lock().unwrap();
        rooms.list.retain(|r| r.id != id);
        if let Some(r) = rooms.list.iter_mut().find(|r| r.id == pending) {
            r.id = id.to_string();
            r.name = clean_text(name, ROOM_NAME_MAX);
            r.invite = None;
            r.waiting = false;
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
    if let Some(rec) = core.own_log(&set.room).iter().find(|r| r["k"] == "rec" && r["set"] == set.id.as_str()) {
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
    core.write_json(&core.set_dir(&set.room, &set.id).join("set.json"), value);
}

pub fn begin_set(core: &Arc<Core>, set: &str, by: &str, start_us: i64) -> Result<(), String> {
    ffmpeg::ready()?;
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
    core.append_log(&active.room, &json!({"k": "rec_end", "set": active.id, "room": active.room, "t_us": now_us()}));
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
    pub records: Arc<Vec<Value>>,
}

pub struct File {
    pub owner: String,
    pub start_us: i64,
    pub end_us: Option<i64>,
    pub offset_us: i64,
    pub total: Option<u64>,
    pub sha256: Option<String>,
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
    format!("{set}/{}", short(owner))
}

pub fn logs(core: &Core, room: &str) -> Vec<Log> {
    let own = core.own_log(room);
    let legacy = core.log(&core.dir.join("legacy").join("log.bin"));
    let mut offsets: HashMap<String, i64> =
        legacy.iter().chain(own.iter()).filter(|r| r["k"] == "offset").filter_map(|r| Some((r["peer"].as_str()?.to_string(), r["offset_us"].as_i64()?))).collect();
    for p in core.peers.lock().unwrap().iter() {
        if let Some(offset) = p.offset_us {
            offsets.insert(short(p.id), offset);
        }
    }
    let people = core.rooms.lock().unwrap().list.iter().find(|r| r.id == room).map(Room::people).unwrap_or_default();
    let me = core.me();
    let mut out = vec![Log { owner: me.clone(), offset_us: 0, records: own }];
    for id in people.iter().filter(|id| id.parse::<EndpointId>().is_ok() && **id != me) {
        out.push(Log { owner: id.clone(), offset_us: offsets.get(&short(id)).copied().unwrap_or(0), records: core.room_log(room, id) });
    }
    out
}

pub fn sets(core: &Core, room: &str, logs: &[Log]) -> Vec<Set> {
    derive_sets(room, logs, &core.deleted.lock().unwrap())
}

pub fn derive_sets(room: &str, logs: &[Log], deleted: &HashSet<String>) -> Vec<Set> {
    let mut sets: BTreeMap<String, Set> = BTreeMap::new();
    for log in logs {
        let mut open: HashMap<&str, (usize, bool, bool)> = HashMap::new();
        for r in log.records.iter() {
            let Some(id) = r["set"].as_str() else { continue };
            match r["k"].as_str() {
                Some("rec") if valid_id(id) => {
                    if deleted.contains(&format!("{room}/{id}")) || sets.get(id).is_some_and(|s| s.files.iter().any(|f| f.owner == log.owner)) {
                        continue;
                    }
                    let set = sets.entry(id.to_string()).or_insert_with(|| Set { id: id.to_string(), room: room.to_string(), files: Vec::new() });
                    open.insert(id, (set.files.len(), false, false));
                    set.files.push(File { owner: log.owner.clone(), start_us: r["start_us"].as_i64().unwrap_or(0), end_us: None, offset_us: log.offset_us, total: None, sha256: None });
                }
                Some("rec_end") => {
                    if let Some((i, ended, _)) = open.get_mut(id).filter(|(_, ended, _)| !ended) {
                        *ended = true;
                        sets.get_mut(id).unwrap().files[*i].end_us = r["t_us"].as_i64();
                    }
                }
                Some("file") => {
                    if let Some((i, _, hashed)) = open.get_mut(id).filter(|(_, _, hashed)| !hashed) {
                        *hashed = true;
                        let f = &mut sets.get_mut(id).unwrap().files[*i];
                        f.total = r["size"].as_u64();
                        f.sha256 = r["sha256"].as_str().map(String::from);
                    }
                }
                _ => {}
            }
        }
    }
    sets.into_values().rev().collect()
}

pub fn tags(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split('#').skip(1) {
        let tag: String = word.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect();
        if !tag.is_empty() && !out.iter().any(|t| t.to_lowercase() == tag.to_lowercase()) {
            out.push(tag);
        }
    }
    out
}

fn event_from(r: &Value, owner: &str, id: i64, offset_us: i64) -> Value {
    let note = r["k"] == "note";
    let target = if note { json!({"t_us": r["at_us"].as_i64().unwrap_or(id)}) } else { r["target"].clone() };
    let local = |key: &str| target[key].as_i64().map(|t| t - offset_us);
    let text = r["text"].as_str().unwrap_or_default();
    let plugin = if r["plugin"].is_object() { r["plugin"].clone() } else { Value::Null };
    let tags = if r["tags"].is_array() { r["tags"].clone() } else { json!(tags(text)) };
    json!({
        "key": format!("{owner}/{id}"), "id": id, "author": owner, "plugin": plugin,
        "kind": if note { "note" } else { r["kind"].as_str().unwrap_or("event") },
        "local_us": local("t_us").or_else(|| local("from_us")), "end_us": local("to_us"),
        "set": target["set"], "file": target["file"],
        "text": text, "values": r["values"], "tags": tags, "deleted": false,
    })
}

pub fn events(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> Vec<Value> {
    let mut out = Vec::new();
    for log in logs {
        let cap = caps.map_or(usize::MAX, |c| c.get(&log.owner).copied().unwrap_or(0) as usize);
        let mut list: Vec<Value> = Vec::new();
        let mut index: HashMap<i64, usize> = HashMap::new();
        for r in log.records.iter().take(cap) {
            let id = r["id"].as_i64().or_else(|| r["t_us"].as_i64()).unwrap_or(0);
            let found = index.get(&id).copied().filter(|i| list[*i]["plugin"].is_null());
            match (r["k"].as_str(), found) {
                (Some("note" | "event"), None) if !index.contains_key(&id) => {
                    index.insert(id, list.len());
                    list.push(event_from(r, &log.owner, id, log.offset_us));
                }
                (Some("note_edit" | "event_edit"), Some(i)) => {
                    let text = r["text"].as_str().unwrap_or_default();
                    list[i]["tags"] = if r["tags"].is_array() { r["tags"].clone() } else { json!(tags(text)) };
                    list[i]["text"] = json!(text);
                }
                (Some("note_del" | "event_del"), Some(i)) => list[i]["deleted"] = json!(true),
                (Some("note_restore" | "event_restore"), Some(i)) => list[i]["deleted"] = json!(false),
                _ => {}
            }
        }
        out.extend(list.into_iter().filter(|n| n["deleted"] == false));
    }
    out.sort_by_key(|n| n["local_us"].as_i64());
    out
}

pub fn have_list(core: &Core, room: &str) -> Vec<String> {
    let Ok(sets) = std::fs::read_dir(core.dir.join("rooms").join(room).join("sets")) else { return Vec::new() };
    let mut out = Vec::new();
    for set in sets.flatten() {
        let id = set.file_name().to_string_lossy().to_string();
        for file in data::verified_files(core, &set.path()) {
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
    if core.is_recording(room, set) {
        return Err("This set is being recorded. Stop it first.".into());
    }
    core.deleted.lock().unwrap().insert(format!("{room}/{set}"));
    core.save_settings();
    let _ = std::fs::remove_dir_all(core.set_dir(room, set));
    Ok(())
}

struct Holder<'a> {
    id: &'a str,
    name: String,
    have: &'a HashSet<String>,
    gone: bool,
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn set_status(live: bool, all_verified: bool, missing: &[(&str, usize)]) -> String {
    if live {
        return "Recording".into();
    }
    if all_verified {
        return "Everyone has it".into();
    }
    if missing.is_empty() {
        return "Checking files".into();
    }
    let mut by_count: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    for (name, n) in missing {
        by_count.entry(*n).or_default().push(name);
    }
    by_count.iter().rev().map(|(n, names)| format!("{}: {n} file{} left", names.join(", "), plural(*n))).collect::<Vec<_>>().join(" · ")
}

pub fn covers(set: &Set, event: &Value) -> bool {
    event["set"] == set.id.as_str() || event["set"].is_null() && event["local_us"].as_i64().is_some_and(|t| set.covers(t))
}

pub fn holdings(logs: &[Log]) -> HashMap<String, u64> {
    logs.iter().map(|l| (l.owner.clone(), l.records.len() as u64)).collect()
}

pub fn sets_json(core: &Core, room: &str, logs: &[Log], events: &[Value]) -> Vec<Value> {
    let me = core.ep.id().to_string();
    let peers: Vec<(String, HashSet<String>, HashSet<String>)> = core.peers.lock().unwrap().iter().map(|p| (p.id.to_string(), p.have.clone(), p.gone.clone())).collect();
    let names: Vec<String> = peers.iter().map(|(id, _, _)| display_name(core, id)).collect();
    let mut out = Vec::new();
    for set in sets(core, room, logs) {
        let dir = core.set_dir(room, &set.id);
        let mine: HashSet<String> = data::verified_files(core, &dir).iter().map(|f| have_key(&set.id, f.trim_end_matches(".mp4"))).collect();
        let mut holders = vec![Holder { id: &me, name: "You".into(), have: &mine, gone: false }];
        holders.extend(peers.iter().zip(&names).map(|((id, have, gone), name)| Holder { id, name: name.clone(), have, gone: gone.contains(&set.id) }));
        let mut players = Vec::new();
        let (mut have, mut total) = (0u64, 0u64);
        for f in &set.files {
            let path = core.set_file(room, &set.id, &f.owner);
            let size = file_len(&path);
            have += f.total.map_or(size, |t| size.min(t));
            total += f.total.unwrap_or(size);
            let hk = have_key(&set.id, &f.owner);
            let by: Vec<&str> = holders.iter().filter(|h| h.have.contains(&hk)).map(|h| h.id).collect();
            players.push(json!({
                "owner": f.owner, "start_us": f.start_us, "offset_us": f.offset_us, "size": size, "total": f.total, "path": path,
                "recording": f.end_us.is_none(), "verified_by": by, "sha256": f.sha256,
            }));
        }
        let finished: Vec<&File> = set.files.iter().filter(|f| f.total.is_some()).collect();
        let lacking = |h: &Holder| finished.iter().filter(|f| !h.have.contains(&have_key(&set.id, &f.owner))).count();
        let missing: Vec<(&str, usize)> = holders.iter().filter(|h| !h.gone).map(|h| (h.name.as_str(), lacking(h))).filter(|(_, n)| *n > 0).collect();
        let live = set.end().is_none();
        let all_verified = missing.is_empty() && finished.len() == set.files.len();
        let covered: Vec<&Value> = events.iter().filter(|e| covers(&set, e)).map(|e| &e["key"]).collect();
        out.push(json!({
            "id": set.id, "room": room, "start_us": set.start(), "end_us": set.end(), "live": live,
            "players": players, "events": covered, "progress": {"have": have, "total": total}, "missing": missing.len(), "all_verified": all_verified,
            "status": set_status(live, all_verified, &missing), "layout": core.json(&dir.join("set.json"))["layout"],
        }));
    }
    out
}

fn path_kind(conn: &Connection) -> (&'static str, u64) {
    for path in conn.paths().iter() {
        if path.is_selected() {
            return (if path.is_ip() { "direct" } else { "relay" }, path.rtt().as_millis() as u64);
        }
    }
    ("none", 0)
}

fn live_json(core: &Core, room: &str, name: Option<String>) -> Option<Value> {
    let path = core.live_dir(room).join(name?);
    Some(json!({"size": file_len(&path), "path": path}))
}

fn peer_json(core: &Core, room: &str, p: &Peer) -> Value {
    let alive = p.conn.iter().chain(p.incoming.iter()).find(|c| c.close_reason().is_none());
    let (path, rtt) = alive.map(path_kind).unzip();
    let message = p.info.as_ref().filter(|(_, proto)| *proto != PROTOCOL).map(|(version, proto)| format!("peer has version {version} (protocol {proto}, ours {PROTOCOL}), update needed"));
    let sharing = p.sharing && alive.is_some();
    json!({
        "id": p.id.to_string(),
        "name": p.name,
        "connected": alive.is_some(),
        "path": path,
        "rtt_ms": rtt,
        "failures": p.failures,
        "offset_us": p.offset_us,
        "version": p.info.as_ref().map(|(version, _)| version),
        "message": message,
        "sharing": sharing,
        "live": live_json(core, room, p.live.clone().filter(|_| sharing)),
    })
}

pub fn state_json(core: &Core) -> Value {
    let room_id = core.active_room_id();
    let my_id = core.ep.id().to_string();
    let active = core.active_room();
    let mut names: HashMap<String, String> = active.as_ref().map(|r| r.names.clone()).unwrap_or_default();
    let peers: Vec<Value> = core
        .peers
        .lock()
        .unwrap()
        .iter()
        .map(|p| {
            if !p.name.is_empty() {
                names.insert(p.id.to_string(), p.name.clone());
            }
            peer_json(core, &room_id, p)
        })
        .collect();
    names.insert(my_id.clone(), "You".to_string());
    let logs = logs(core, &room_id);
    let caps = core.party.lock().unwrap().caps();
    let mut events = events(&logs, caps.as_ref());
    let sets = sets_json(core, &room_id, &logs, &events);
    let seekable: HashSet<&str> = sets.iter().filter(|s| s["progress"]["have"].as_u64() > Some(0)).flat_map(|s| s["events"].as_array().into_iter().flatten().filter_map(|k| k.as_str())).collect();
    for e in events.iter_mut() {
        let (can_seek, mine) = (seekable.contains(e["key"].as_str().unwrap_or_default()) && e["local_us"].is_i64(), e["author"] == my_id.as_str() && e["plugin"].is_null());
        e["seekable"] = json!(can_seek);
        e["mine"] = json!(mine);
    }
    let mut asks: Vec<Value> = core.asks.lock().unwrap().iter().filter(|a| a.answer.is_none()).map(|a| json!({"id": a.id, "name": a.name})).collect();
    for p in core.peers.lock().unwrap().iter().filter(|p| p.conn.is_some() || p.incoming.is_some()) {
        for a in &p.asks {
            if !asks.iter().any(|b| b["id"] == a["id"]) {
                asks.push(a.clone());
            }
        }
    }
    let rooms = core.rooms.lock().unwrap();
    let room_list: Vec<Value> = rooms.list.iter().map(|r| json!({"id": r.id, "name": r.name, "active": rooms.active.as_deref() == Some(&r.id), "members": r.members.len()})).collect();
    drop(rooms);
    json!({
        "version": VERSION,
        "ffmpeg": core.ffmpeg_status.lock().unwrap().clone(),
        "id": my_id,
        "name": core.name.lock().unwrap().clone(),
        "share": core.share.load(Ordering::Relaxed),
        "audio_process": core.audio_process.lock().unwrap().clone(),
        "keep_days": *core.keep_days.lock().unwrap(),
        "invites": core.invites.lock().unwrap().len(),
        "limit_mbps": *core.limit_mbps.lock().unwrap(),
        "recovery": core.recovery.lock().unwrap().clone(),
        "notice": core.notice.lock().unwrap().clone(),
        "disk": {"free": core.free_bytes.load(Ordering::Relaxed), "low": core.disk_low()},
        "previous_version": core.previous_version,
        "codec": ffmpeg::pick(&core.codecs.lock().unwrap()).1,
        "recording": core.active_set.lock().unwrap().is_some(),
        "capture": record::capture_json(core, live_json(core, &room_id, core.engine.live_file())),
        "active_set": core.active_set_json(),
        "room": active.map(|r| json!({"id": r.id, "name": r.name, "pending": r.invite.is_some(), "waiting": r.waiting})),
        "rooms": room_list,
        "layout": core.layout.lock().unwrap().clone(),
        "sets": sets,
        "peers": peers,
        "events": events,
        "names": names,
        "asks": asks,
        "party": party::state_json(core, &holdings(&logs)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(owner: &str, offset_us: i64, records: Vec<Value>) -> Log {
        Log { owner: owner.into(), offset_us, records: Arc::new(records) }
    }

    #[test]
    fn sets_come_from_rec_rec_end_and_file_records() {
        let a = log("a", 0, vec![
            json!({"k": "rec", "set": "s1", "start_us": 100}),
            json!({"k": "rec_end", "set": "s1", "t_us": 900}),
            json!({"k": "file", "set": "s1", "size": 10, "sha256": "ab"}),
            json!({"k": "rec", "set": "s2", "start_us": 1000}),
            json!({"k": "rec", "set": "../bad", "start_us": 1}),
        ]);
        let b = log("b", 50, vec![json!({"k": "rec", "set": "s1", "start_us": 200}), json!({"k": "rec", "set": "s1", "start_us": 300})]);
        let sets = derive_sets("r", &[a, b], &HashSet::new());
        assert_eq!(sets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s2", "s1"]);
        let s1 = &sets[1];
        assert_eq!(s1.files.len(), 2);
        assert_eq!(s1.files[0].total, Some(10));
        assert_eq!(s1.start(), 100);
        assert_eq!(s1.end(), None);
        assert!(sets[0].files[0].end_us.is_none());
        let deleted: HashSet<String> = ["r/s2".to_string()].into();
        assert_eq!(derive_sets("r", &[log("a", 0, vec![json!({"k": "rec", "set": "s2"})])], &deleted).len(), 0);
    }

    #[test]
    fn tags_are_read_from_text() {
        assert_eq!(tags("push #Rotate then #rotate and #ジャンプ, #"), vec!["Rotate", "ジャンプ"]);
        assert!(tags("no tags").is_empty());
    }

    #[test]
    fn events_merge_old_notes_new_events_and_edits() {
        let a = log("a", 10, vec![
            json!({"k": "note", "id": 1, "t_us": 1, "at_us": 500, "text": "old #one"}),
            json!({"k": "note_edit", "id": 1, "text": "edited #two"}),
            json!({"k": "event", "id": 2, "kind": "note", "target": {"t_us": 700}, "text": "new", "tags": ["x"]}),
            json!({"k": "event_del", "id": 2}),
            json!({"k": "event", "id": 3, "kind": "marker", "target": {"from_us": 100, "to_us": 200}, "text": ""}),
            json!({"k": "event", "id": 4, "kind": "round", "plugin": {"name": "apex", "version": "1"}, "target": {"set": "s1"}, "values": {"n": 2}, "tags": ["p"]}),
            json!({"k": "event_edit", "id": 4, "text": "hijack"}),
            json!({"k": "note_del", "id": 9}),
        ]);
        let all = events(&[a], None);
        assert_eq!(all.len(), 3);
        let note = all.iter().find(|e| e["id"] == 1).unwrap();
        assert_eq!(note["text"], "edited #two");
        assert_eq!(note["tags"], json!(["two"]));
        assert_eq!(note["local_us"], 490);
        let marker = all.iter().find(|e| e["id"] == 3).unwrap();
        assert_eq!((marker["local_us"].clone(), marker["end_us"].clone()), (json!(90), json!(190)));
        let plugin = all.iter().find(|e| e["id"] == 4).unwrap();
        assert_eq!(plugin["text"], "");
        assert_eq!(plugin["set"], "s1");
        assert_eq!(plugin["plugin"]["name"], "apex");
    }

    #[test]
    fn restore_and_caps() {
        let records = vec![
            json!({"k": "event", "id": 1, "kind": "note", "target": {"t_us": 5}, "text": "a"}),
            json!({"k": "event_del", "id": 1}),
            json!({"k": "event_restore", "id": 1}),
            json!({"k": "event", "id": 2, "kind": "note", "target": {"t_us": 6}, "text": "b"}),
        ];
        assert_eq!(events(&[log("a", 0, records.clone())], None).len(), 2);
        let caps: HashMap<String, u64> = [("a".to_string(), 2)].into();
        assert_eq!(events(&[log("a", 0, records.clone())], Some(&caps)).len(), 0);
        let caps: HashMap<String, u64> = [("a".to_string(), 3)].into();
        assert_eq!(events(&[log("a", 0, records.clone())], Some(&caps)).len(), 1);
        assert_eq!(events(&[log("b", 0, records)], Some(&caps)).len(), 0);
    }

    #[test]
    fn coverage_by_moment_or_set() {
        let set = Set { id: "s1".into(), room: "r".into(), files: vec![File { owner: "a".into(), start_us: 100, end_us: Some(200), offset_us: 0, total: None, sha256: None }] };
        assert!(covers(&set, &json!({"local_us": 150, "set": null})));
        assert!(!covers(&set, &json!({"local_us": 250, "set": null})));
        assert!(covers(&set, &json!({"local_us": null, "set": "s1"})));
        assert!(!covers(&set, &json!({"local_us": 150, "set": "s2"})));
    }

    fn ask(invites: &mut Vec<(String, String)>, asks: &mut Vec<Ask>, token: &str, id: &str) -> Step {
        ask_step(invites, asks, "r", token, id, "Ren")
    }

    #[test]
    fn joining_waits_for_an_answer_and_ends_the_invite() {
        let mut invites = vec![("t1".to_string(), "r".to_string()), ("t2".to_string(), "r".to_string())];
        let mut asks = Vec::new();
        assert_eq!(ask(&mut invites, &mut asks, "nope", "x"), Step::Refused);
        assert_eq!(ask(&mut invites, &mut asks, "t1", "x"), Step::Wait(true));
        assert_eq!(ask(&mut invites, &mut asks, "t1", "x"), Step::Wait(false));
        assert_eq!(ask(&mut invites, &mut asks, "t1", "y"), Step::Refused);
        assert!(answer_ask(&mut asks, "x", true));
        assert!(!answer_ask(&mut asks, "x", false));
        assert_eq!(ask(&mut invites, &mut asks, "t1", "x"), Step::Admitted);
        assert_eq!(ask(&mut invites, &mut asks, "t1", "x"), Step::Refused);
        assert_eq!(ask(&mut invites, &mut asks, "t2", "z"), Step::Wait(true));
        answer_ask(&mut asks, "z", false);
        assert_eq!(ask(&mut invites, &mut asks, "t2", "z"), Step::Declined);
        assert!(invites.is_empty() && asks.is_empty());
        assert_eq!(ask_step(&mut vec![("t3".into(), "other".into())], &mut asks, "r", "t3", "x", ""), Step::Refused);
    }

    #[test]
    fn the_latest_remove_or_admit_wins() {
        let a = log("a", 0, vec![json!({"k": "remove", "member": "b", "t_us": 100}), json!({"k": "admit", "member": "c", "t_us": 50})]);
        let c = log("c", 1000, vec![json!({"k": "admit", "member": "b", "t_us": 1050}), json!({"k": "remove", "member": "d", "t_us": 1200})]);
        let latest = removals(&[a, c]);
        assert_eq!(latest["b"], (100, true, "a".to_string()));
        assert_eq!(latest["c"], (50, false, "a".to_string()));
        assert_eq!(latest["d"], (200, true, "c".to_string()));
        let later = removals(&[log("a", 0, vec![json!({"k": "remove", "member": "b", "t_us": 100}), json!({"k": "admit", "member": "b", "t_us": 300})])]);
        assert!(!later["b"].1);
    }

    #[test]
    fn logs_are_shared_only_with_members_of_that_room() {
        let mut room = Room { id: "r".into(), members: vec!["a".into(), "b".into()], removed: vec!["c".into()], ..Default::default() };
        assert!(shares(&room, "b", "a"));
        assert!(shares(&room, "b", "c"));
        assert!(!shares(&room, "c", "a"));
        assert!(!shares(&room, "x", "a"));
        assert!(!shares(&room, "b", "x"));
        room.invite = Some(("a".into(), "t".into()));
        assert!(!shares(&room, "b", "a"));
    }
}

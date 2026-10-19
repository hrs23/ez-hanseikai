use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use iroh::endpoint::Connection;
use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use tauri::Emitter;

use crate::{clean_text, data, ffmpeg, hex, net, now_us, party, record, short, stream, Core, Peer, PROTOCOL, ROOM_NAME_MAX, VERSION};

pub const SOLO: &str = "solo";
pub const SERIES: [&str; 2] = ["v", "m"];
pub const INIT: &str = "init.mp4";
pub const TOKEN_BYTES: usize = 8;

pub fn seg_name(n: u32) -> String {
    format!("{n:06}.m4s")
}

pub fn seg_number(name: &str) -> Option<u32> {
    name.strip_suffix(".part").unwrap_or(name).strip_suffix(".m4s")?.parse().ok()
}

pub fn part_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

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

pub fn read_rooms(value: &Value) -> Rooms {
    let list: Vec<Room> = value["list"].as_array().into_iter().flatten().filter_map(|r| serde_json::from_value(r.clone()).ok()).filter(|r: &Room| valid_id(&r.id)).collect();
    Rooms { active: value["active"].as_str().map(String::from), list }
}

#[derive(Clone)]
pub struct ActiveSet {
    pub id: String,
    pub room: String,
    pub by: String,
    pub start_us: i64,
    pub voice: bool,
}

pub fn random_hex(bytes: usize) -> String {
    hex(&iroh::SecretKey::generate().to_bytes()[..bytes])
}

pub fn capture_id(text: &str) -> bool {
    !text.is_empty() && text.len() <= 20 && text.chars().all(|c| c.is_ascii_digit())
}

pub fn valid_token(text: &str) -> bool {
    text.len() == 2 * TOKEN_BYTES && text.chars().all(|c| c.is_ascii_hexdigit())
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

pub fn wall_clock(now_us: i64, tz_offset_s: i64) -> String {
    let local = now_us + tz_offset_s * 1_000_000;
    let secs = local.div_euclid(1_000_000);
    let (y, m, d) = civil(secs.div_euclid(86400));
    let rest = secs.rem_euclid(86400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}", rest / 3600, rest % 3600 / 60, rest % 60, local.rem_euclid(1_000_000) / 1000)
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

    pub fn capture_dir(&self, room: &str, owner: &str, capture: &str) -> PathBuf {
        self.dir.join("rooms").join(room).join(owner).join(capture)
    }

    pub fn seg_path(&self, room: &str, owner: &str, seg: &Seg) -> PathBuf {
        self.capture_dir(room, owner, &seg.capture).join(&seg.series).join(seg_name(seg.n))
    }

    pub fn save_rooms(&self) {
        let text = serde_json::to_string(&*self.rooms.lock().unwrap()).unwrap_or_default();
        let _ = crate::write_whole(&self.dir.join("rooms.json"), text.as_bytes());
    }

    pub fn active_set_json(&self) -> Value {
        match self.active_set.lock().unwrap().as_ref() {
            Some(s) => json!({"id": s.id, "room": s.room, "by": s.by, "start_us": s.start_us, "voice": s.voice}),
            None => Value::Null,
        }
    }

    pub fn is_recording(&self, room: &str, set: &str) -> bool {
        self.active_set.lock().unwrap().as_ref().is_some_and(|a| a.id == set && a.room == room)
    }

    pub fn in_use(&self, room: &str, set: &str) -> bool {
        self.is_recording(room, set) || self.export.lock().unwrap().as_ref().is_some_and(|e| e.set.0 == room && e.set.1 == set)
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
                for conn in p.conn.iter() {
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
        tauri::async_runtime::spawn(net::peer_loop(core.clone(), id));
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
    let (id, token) = text.strip_prefix("ezh-").and_then(|t| t.split_once('-')).filter(|(_, token)| valid_token(token)).ok_or_else(bad)?;
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

pub fn close_room(core: &Arc<Core>, id: &str) {
    {
        let mut rooms = core.rooms.lock().unwrap();
        if rooms.active.as_deref() == Some(id) {
            rooms.active = None;
        }
    }
    core.save_rooms();
    apply_active(core);
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

pub fn rename_room(core: &Arc<Core>, name: &str) -> Result<(), String> {
    let name = clean_text(name, ROOM_NAME_MAX);
    let room = core.room_id().ok_or("Join a room first.")?;
    if name.is_empty() {
        return Err("Enter a room name.".into());
    }
    let t_us = named(&logs(core, &room)).map_or(now_us(), |(t, ..)| now_us().max(t + 1));
    core.append_log(&room, &json!({"k": "room_name", "name": name, "t_us": t_us}));
    keep_name(core, &room, &name);
    Ok(())
}

fn named(logs: &[Log]) -> Option<(i64, &String, &str)> {
    logs.iter().flat_map(|log| log.records.iter().filter(|r| r["k"] == "room_name").filter_map(|r| Some((r["t_us"].as_i64()? - log.offset_us, &log.owner, r["name"].as_str()?)))).max()
}

pub fn latest_name(logs: &[Log]) -> Option<String> {
    named(logs).map(|(_, _, name)| clean_text(name, ROOM_NAME_MAX)).filter(|name| !name.is_empty())
}

fn keep_name(core: &Core, room: &str, name: &str) {
    let changed = match core.rooms.lock().unwrap().list.iter_mut().find(|r| r.id == room) {
        Some(r) if r.name != name => {
            r.name = name.to_string();
            true
        }
        _ => false,
    };
    if changed {
        core.save_rooms();
    }
}

pub const PALETTE: [&str; 8] = ["#ff8a00", "#00d5ff", "#7cff4f", "#ff5ad6", "#ffe83d", "#b48cff", "#ff6b6b", "#5affc8"];

fn preferred_color(id: &str) -> usize {
    let hash = id.bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64));
    (hash % PALETTE.len() as u64) as usize
}

pub fn colors(people: &[String]) -> HashMap<String, usize> {
    let mut sorted: Vec<(usize, &String)> = people.iter().map(|id| (preferred_color(id), id)).collect();
    sorted.sort();
    sorted.dedup();
    let mut taken = [false; PALETTE.len()];
    let mut out = HashMap::new();
    for (preferred, id) in sorted {
        let color = (0..PALETTE.len()).map(|step| (preferred + step) % PALETTE.len()).find(|c| !taken[*c]).unwrap_or(preferred);
        taken[color] = true;
        out.insert(id.clone(), color);
    }
    out
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
        let code = if member == me { "removed_you" } else { "removed" };
        let _ = core.app.emit("flash", json!({"code": code, "by": display_name(core, &by), "name": display_name(core, &member)}));
    }
}

pub fn adopt_room(core: &Arc<Core>, pending: &str, id: &str, name: &str, members: &[String]) {
    if !valid_id(id) || id.starts_with("pending-") {
        return;
    }
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

pub fn begin_set(core: &Arc<Core>, set: &str, by: &str, start_us: i64, voice: bool) -> Result<(), String> {
    ffmpeg::ready()?;
    let room = core.active_room_id();
    if core.is_deleted(&room, set) {
        return Err("That recording was deleted on this PC.".into());
    }
    let active = ActiveSet { id: set.to_string(), room, by: by.to_string(), start_us, voice };
    {
        let mut slot = core.active_set.lock().unwrap();
        if slot.is_some() {
            return Err("A recording is already in progress.".into());
        }
        *slot = Some(active);
    }
    tracing::info!("{} {set} started by {by}", if voice { "review session" } else { "session" });
    Ok(())
}

pub fn end_set(core: &Arc<Core>, stop: bool, only: Option<&str>) {
    let Some(active) = record::end_set(core, only) else { return };
    core.append_log(&active.room, &json!({"k": "rec_end", "set": active.id, "room": active.room, "t_us": now_us(), "stop": stop}));
    let own = core.own_log(&active.room);
    let count = |series: &str| own.iter().filter(|r| r["k"] == "seg" && r["set"] == active.id.as_str() && r["series"] == series).count();
    tracing::info!("session {} ended{}: {} picture and {} voice segments", active.id, if stop { " by a stop" } else { "" }, count(SERIES[0]), count(SERIES[1]));
}

pub fn joined(records: &[Value], set: &str) -> bool {
    records.iter().rev().find(|r| (r["k"] == "rec" || r["k"] == "rec_end") && r["set"] == set).is_some_and(|r| r["k"] == "rec")
}

pub fn may_join(records: &[Value], set: &str) -> bool {
    let last = records.iter().rev().find(|r| r["k"] == "rec");
    !joined(records, set) && last.is_none_or(|r| r["set"] == set || !records.iter().any(|r| r["k"] == "rec" && r["set"] == set))
}

pub fn stopped(logs: &[Log], set: &str) -> bool {
    logs.iter().any(|log| log.records.iter().any(|r| r["k"] == "rec_end" && r["set"] == set && r["stop"] == true))
}

pub fn apply_stops(core: &Arc<Core>) {
    let Some(active) = core.active_set.lock().unwrap().clone() else { return };
    if stopped(&logs(core, &active.room), &active.id) {
        let core = core.clone();
        std::thread::spawn(move || end_set(&core, true, Some(&active.id)));
    }
}

pub fn log_set_layout(core: &Core) {
    let active = core.active_set.lock().unwrap().clone();
    if let Some(active) = active {
        core.append_log(&active.room, &json!({"k": "layout", "set": active.id, "t_us": now_us(), "layout": *core.layout.lock().unwrap()}));
    }
}

pub fn start_set(core: &Arc<Core>) -> Result<(), String> {
    let start_us = now_us();
    let set = set_id_for(start_us, core.tz_offset_s);
    let by = core.name.lock().unwrap().clone();
    let voice = core.party.lock().unwrap().reviewing();
    begin_set(core, &set, &by, start_us, voice)?;
    log_set_layout(core);
    party::log(core);
    net::broadcast(core, json!({"t": "rec", "action": "start", "set": set, "room": core.active_room_id(), "by": by, "start_us": start_us, "voice": voice}));
    announce(core, "start", &by);
    Ok(())
}

pub fn announce(core: &Core, action: &str, by: &str) {
    let _ = core.app.emit("session", json!({"action": action, "by": by}));
}

pub fn stop_set(core: &Arc<Core>) {
    let set = core.active_set.lock().unwrap().as_ref().map(|s| s.id.clone());
    end_set(core, true, None);
    if let Some(set) = set {
        let by = core.name.lock().unwrap().clone();
        net::broadcast(core, json!({"t": "rec", "action": "stop", "set": set, "room": core.active_room_id(), "by": by}));
        announce(core, "end", &by);
    }
}

pub struct Log {
    pub owner: String,
    pub offset_us: i64,
    pub records: Arc<Vec<Value>>,
}

pub struct Seg {
    pub capture: String,
    pub series: String,
    pub n: u32,
    pub t_us: i64,
    pub d_us: i64,
    pub size: u64,
    pub sha256: String,
}

pub struct File {
    pub owner: String,
    pub start_us: i64,
    pub end_us: Option<i64>,
    pub offset_us: i64,
    pub segs: Vec<Seg>,
    pub caps: HashMap<String, Value>,
}

impl File {
    pub fn series(&self, series: &str) -> Vec<&Seg> {
        self.segs.iter().filter(|s| s.series == series).collect()
    }

    pub fn captures(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for s in &self.segs {
            if !out.contains(&s.capture.as_str()) {
                out.push(&s.capture);
            }
        }
        out
    }

    pub fn init(&self, capture: &str, series: &str) -> Option<(u64, String)> {
        let init = &self.caps.get(capture)?["init"][series];
        Some((init["size"].as_u64()?, init["sha256"].as_str()?.to_string()))
    }

    pub fn total(&self) -> u64 {
        let mut total: u64 = self.segs.iter().map(|s| s.size).sum();
        for capture in self.captures() {
            for series in SERIES.iter().filter(|x| self.segs.iter().any(|s| s.capture == capture && s.series == **x)) {
                total += self.init(capture, series).map_or(0, |(size, _)| size);
            }
        }
        total
    }
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

pub fn have_key(set: &str, owner: &str, series: &str) -> String {
    format!("{set}/{}/{series}", short(owner))
}

pub fn logs(core: &Core, room: &str) -> Vec<Log> {
    let own = core.own_log(room);
    let mut offsets: HashMap<String, i64> = own.iter().filter(|r| r["k"] == "offset").filter_map(|r| Some((r["peer"].as_str()?.to_string(), r["offset_us"].as_i64()?))).collect();
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

fn seg_from(r: &Value) -> Option<Seg> {
    let capture = r["capture"].as_str().filter(|c| capture_id(c))?;
    let series = r["series"].as_str().filter(|s| SERIES.contains(s))?;
    let n = r["n"].as_u64().filter(|n| *n > 0 && *n <= u32::MAX as u64)? as u32;
    Some(Seg { capture: capture.into(), series: series.into(), n, t_us: r["t_us"].as_i64()?, d_us: r["d_us"].as_i64().unwrap_or(0), size: r["size"].as_u64()?, sha256: r["sha256"].as_str()?.into() })
}

pub fn derive_sets(room: &str, logs: &[Log], deleted: &HashSet<String>) -> Vec<Set> {
    let mut sets: BTreeMap<String, Set> = BTreeMap::new();
    for log in logs {
        let mut open: HashMap<&str, (usize, bool)> = HashMap::new();
        let mut caps: HashMap<&str, Value> = HashMap::new();
        let mut seen: HashSet<(&str, String, String, u32)> = HashSet::new();
        for r in log.records.iter() {
            if r["k"] == "cap" {
                if let Some(capture) = r["capture"].as_str() {
                    crate::merge(caps.entry(capture).or_insert_with(|| json!({})), r);
                }
                continue;
            }
            let Some(id) = r["set"].as_str() else { continue };
            match r["k"].as_str() {
                Some("rec") if valid_id(id) => {
                    if let Some((i, ended)) = open.get_mut(id) {
                        if *ended {
                            *ended = false;
                            sets.get_mut(id).unwrap().files[*i].end_us = None;
                        }
                        continue;
                    }
                    if deleted.contains(&format!("{room}/{id}")) || sets.get(id).is_some_and(|s| s.files.iter().any(|f| f.owner == log.owner)) {
                        continue;
                    }
                    let set = sets.entry(id.to_string()).or_insert_with(|| Set { id: id.to_string(), room: room.to_string(), files: Vec::new() });
                    open.insert(id, (set.files.len(), false));
                    set.files.push(File { owner: log.owner.clone(), start_us: r["start_us"].as_i64().unwrap_or(0), end_us: None, offset_us: log.offset_us, segs: Vec::new(), caps: HashMap::new() });
                }
                Some("rec_end") => {
                    if let Some((i, ended)) = open.get_mut(id).filter(|(_, ended)| !ended) {
                        *ended = true;
                        sets.get_mut(id).unwrap().files[*i].end_us = r["t_us"].as_i64();
                    }
                }
                Some("seg") => {
                    if let (Some((i, _)), Some(seg)) = (open.get(id), seg_from(r).filter(|s| seen.insert((id, s.capture.clone(), s.series.clone(), s.n)))) {
                        let f = &mut sets.get_mut(id).unwrap().files[*i];
                        if let Some(cap) = caps.get(seg.capture.as_str()) {
                            f.caps.entry(seg.capture.clone()).or_insert_with(|| cap.clone());
                        }
                        f.segs.push(seg);
                    }
                }
                _ => {}
            }
        }
    }
    sets.into_values().rev().collect()
}

pub fn user_event(id: i64, kind: &str, text: &str, target: Value, values: Value) -> Value {
    json!({"k": "event", "id": id, "t_us": id, "kind": kind, "target": target, "text": text, "values": values, "tags": tags(text)})
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

fn event_tags(r: &Value, text: &str) -> Value {
    match r["tags"].as_array() {
        Some(list) => json!(list.iter().filter_map(|t| t.as_str()).collect::<Vec<_>>()),
        None => json!(tags(text)),
    }
}

fn event_from(r: &Value, owner: &str, id: i64, offset_us: i64) -> Value {
    let target = &r["target"];
    let local = |key: &str| target[key].as_i64().map(|t| t - offset_us);
    let text: String = r["text"].as_str().unwrap_or_default().chars().take(crate::BOARD_MAX).collect();
    let plugin = if r["plugin"].is_object() { r["plugin"].clone() } else { Value::Null };
    let tags = event_tags(r, &text);
    let file = target["file"].as_str().map_or(Value::Null, |f| json!(f));
    let set = target["set"].as_str().map_or(Value::Null, |s| json!(s));
    json!({
        "key": format!("{owner}/{id}"), "id": id, "author": owner, "plugin": plugin,
        "kind": r["kind"].as_str().unwrap_or("event"),
        "local_us": local("t_us").or_else(|| local("from_us")), "end_us": local("to_us"),
        "set": set, "file": file,
        "text": text, "values": r["values"], "tags": tags, "deleted": false, "run": r["run"],
    })
}

pub fn events(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> Vec<Value> {
    let mut out = Vec::new();
    for log in logs {
        let cap = caps.map_or(usize::MAX, |c| c.get(&log.owner).copied().unwrap_or(0) as usize);
        let mut list: Vec<Value> = Vec::new();
        let mut index: HashMap<i64, usize> = HashMap::new();
        for r in log.records.iter().take(cap) {
            let Some(id) = r["id"].as_i64() else { continue };
            let found = index.get(&id).copied().filter(|i| list[*i]["plugin"].is_null());
            match (r["k"].as_str(), found) {
                (Some("event"), None) if !index.contains_key(&id) => {
                    index.insert(id, list.len());
                    list.push(event_from(r, &log.owner, id, log.offset_us));
                }
                (Some("event_edit"), Some(i)) => {
                    let text: String = r["text"].as_str().unwrap_or_default().chars().take(crate::BOARD_MAX).collect();
                    list[i]["tags"] = event_tags(r, &text);
                    list[i]["text"] = json!(text);
                }
                (Some("event_del"), Some(i)) => list[i]["deleted"] = json!(true),
                (Some("event_restore"), Some(i)) => list[i]["deleted"] = json!(false),
                _ => {}
            }
        }
        out.extend(list.into_iter().filter(|n| n["deleted"] == false));
    }
    crate::jobs::keep_shown(&mut out, logs, caps);
    out.sort_by_key(|n| n["local_us"].as_i64());
    out
}

pub fn gone_json(room: &str, logs: &[Log], deleted: &HashSet<String>, events: &[Value]) -> Vec<Value> {
    derive_sets(room, logs, &HashSet::new())
        .into_iter()
        .filter(|s| deleted.contains(&format!("{room}/{}", s.id)))
        .map(|set| {
            let players: Vec<Value> = set.files.iter().map(|f| json!({"owner": f.owner, "total": f.total(), "voice": f.series(SERIES[0]).is_empty()})).collect();
            let covered: Vec<&Value> = events.iter().filter(|e| covers(&set, e)).map(|e| &e["key"]).collect();
            json!({"id": set.id, "room": room, "start_us": set.start(), "end_us": set.end(), "players": players, "events": covered})
        })
        .collect()
}

pub fn restore_set(core: &Core, room: &str, set: &str) {
    if core.deleted.lock().unwrap().remove(&format!("{room}/{set}")) {
        core.save_settings();
        tracing::info!("session {set} brought back to this PC");
    }
}

pub fn gone_list(core: &Core, room: &str) -> Vec<String> {
    core.deleted.lock().unwrap().iter().filter_map(|k| k.strip_prefix(&format!("{room}/")).map(String::from)).collect()
}

pub fn someone_else_has(core: &Core, key: &str, index: u64) -> bool {
    core.peers.lock().unwrap().iter().any(|p| p.have.get(key).is_some_and(|n| *n > index))
}

pub fn delete_set(core: &Core, room: &str, set: &str) -> Result<(), String> {
    if !valid_id(room) || !valid_id(set) {
        return Err("That recording is not on this PC.".into());
    }
    if core.is_recording(room, set) {
        return Err("This recording is still in progress.".into());
    }
    if core.in_use(room, set) {
        return Err("This recording is being exported. Cancel the export first.".into());
    }
    let found = sets(core, room, &logs(core, room)).into_iter().find(|s| s.id == set);
    core.deleted.lock().unwrap().insert(format!("{room}/{set}"));
    core.save_settings();
    if let Some(found) = found {
        data::remove_media(core, &found);
    }
    Ok(())
}

struct Holder<'a> {
    id: &'a str,
    name: String,
    have: &'a HashMap<String, u64>,
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

pub fn full(have: &HashMap<String, u64>, set: &Set, f: &File) -> bool {
    f.end_us.is_some() && SERIES.iter().all(|s| have.get(&have_key(&set.id, &f.owner, s)).copied().unwrap_or(0) >= f.series(s).len() as u64)
}

pub fn stuck(mine: &HashMap<String, u64>, others: &[&HashMap<String, u64>], set: &Set, f: &File) -> bool {
    let count = |have: &HashMap<String, u64>, s: &str| have.get(&have_key(&set.id, &f.owner, s)).copied().unwrap_or(0);
    let progressing = SERIES.iter().any(|s| others.iter().any(|o| count(o, s) > count(mine, s)));
    !full(mine, set, f) && f.end_us.is_some() && !progressing
}

pub fn set_layout(logs: &[Log], set: &str) -> Value {
    logs.iter().flat_map(|l| l.records.iter()).filter(|r| r["k"] == "layout" && r["set"] == set).max_by_key(|r| r["t_us"].as_i64()).map(|r| r["layout"].clone()).unwrap_or(Value::Null)
}

fn parts_json(core: &Core, room: &str, set: &Set, f: &File) -> Vec<Value> {
    let mut out = Vec::new();
    for capture in f.captures() {
        let mut part = json!({"capture": capture});
        for series in SERIES {
            let Some(first) = f.segs.iter().find(|s| s.capture == capture && s.series == series) else { continue };
            let size = stream::size(&stream::set_pieces(core, room, f, capture, series));
            part[series] = json!({"path": format!("set/{room}/{}/{}/{series}/{capture}", set.id, f.owner), "start_us": first.t_us, "size": size});
        }
        out.push(part);
    }
    out
}

pub fn others_have(core: &Core) -> Vec<(String, HashMap<String, u64>, HashSet<String>, bool)> {
    core.peers.lock().unwrap().iter().map(|p| (p.id.to_string(), p.have.clone(), p.gone.clone(), p.conn.is_some())).collect()
}

pub fn progress(core: &Core, set: &Set, mine: &HashMap<String, u64>, others: &[&HashMap<String, u64>]) -> (u64, u64, bool) {
    let (mut have, mut total, mut stuck_all, mut lacking) = (0u64, 0u64, true, false);
    for f in &set.files {
        let (h, t) = data::held(core, &set.room, f);
        have += h;
        total += t;
        if !full(mine, set, f) {
            lacking = true;
            stuck_all &= stuck(mine, others, set, f);
        }
    }
    (have, total, lacking && stuck_all)
}

pub fn sets_json(core: &Core, room: &str, logs: &[Log], events: &[Value]) -> Vec<Value> {
    let me = core.me();
    let peers = others_have(core);
    let names: Vec<String> = peers.iter().map(|(id, _, _, _)| display_name(core, id)).collect();
    let online: Vec<&HashMap<String, u64>> = peers.iter().filter(|p| p.3).map(|p| &p.1).collect();
    let all = sets(core, room, logs);
    let mine = data::have_counts(core, room, &all);
    let mut flows = party::records(logs);
    let mut out = Vec::new();
    for set in all {
        let mut holders = vec![Holder { id: &me, name: "You".into(), have: &mine, gone: false }];
        holders.extend(peers.iter().zip(&names).map(|((id, have, gone, _), name)| Holder { id, name: name.clone(), have, gone: gone.contains(&set.id) }));
        let mut players = Vec::new();
        for f in &set.files {
            let (have, total) = data::held(core, room, f);
            let by: Vec<&str> = holders.iter().filter(|h| full(h.have, &set, f)).map(|h| h.id).collect();
            players.push(json!({
                "owner": f.owner, "start_us": f.start_us, "end_us": f.end_us, "offset_us": f.offset_us, "have": have, "total": total,
                "recording": f.end_us.is_none(), "verified_by": by, "stuck": stuck(&mine, &online, &set, f), "parts": parts_json(core, room, &set, f),
            }));
        }
        let (have, total, stuck_all) = progress(core, &set, &mine, &online);
        let finished: Vec<&File> = set.files.iter().filter(|f| f.end_us.is_some()).collect();
        let lacking = |h: &Holder| finished.iter().filter(|f| !full(h.have, &set, f)).count();
        let missing: Vec<(&str, usize)> = holders.iter().filter(|h| !h.gone).map(|h| (h.name.as_str(), lacking(h))).filter(|(_, n)| *n > 0).collect();
        let lacking_by: Vec<Value> = holders.iter().filter(|h| !h.gone).map(|h| json!({"id": h.id, "n": lacking(h)})).filter(|v| v["n"] != 0).collect();
        let live = set.end().is_none();
        let all_verified = missing.is_empty() && finished.len() == set.files.len();
        let covered: Vec<&Value> = events.iter().filter(|e| covers(&set, e)).map(|e| &e["key"]).collect();
        out.push(json!({
            "id": set.id, "room": room, "start_us": set.start(), "end_us": set.end(), "live": live,
            "players": players, "events": covered, "progress": {"have": have, "total": total, "stuck": stuck_all}, "missing": missing.len(), "lacking": lacking_by, "all_verified": all_verified,
            "status": set_status(live, all_verified, &missing), "layout": set_layout(logs, &set.id), "party": flows.remove(&set.id).unwrap_or_default(),
        }));
    }
    out
}

fn path_kind(conn: &Connection) -> (&'static str, u64, Option<String>) {
    for path in conn.paths().iter() {
        if path.is_selected() {
            let relay = (!path.is_ip()).then(|| relay_host(&format!("{:?}", path.remote_addr()))).flatten();
            return (if path.is_ip() { "direct" } else { "relay" }, path.rtt().as_millis() as u64, relay);
        }
    }
    ("none", 0, None)
}

pub fn relay_host(text: &str) -> Option<String> {
    let rest = &text[text.find("://")? + 3..];
    let host: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-').collect();
    (!host.is_empty()).then_some(host)
}

fn peer_json(core: &Core, room: &str, p: &Peer) -> Value {
    let alive = p.alive();
    let kind = alive.map(path_kind);
    let (path, rtt, relay) = (kind.as_ref().map(|k| k.0), kind.as_ref().map(|k| k.1), kind.and_then(|k| k.2));
    let stats = alive.map(|c| c.stats());
    let other_version = p.info.as_ref().is_some_and(|(_, proto)| *proto != PROTOCOL);
    let sharing = p.sharing && alive.is_some();
    json!({
        "id": p.id.to_string(),
        "name": p.name,
        "connected": alive.is_some(),
        "path": path,
        "rtt_ms": rtt,
        "relay": relay,
        "sent": stats.as_ref().map(|s| s.udp_tx.bytes),
        "received": stats.as_ref().map(|s| s.udp_rx.bytes),
        "failures": p.failures,
        "offset_us": p.offset_us,
        "version": p.info.as_ref().map(|(version, _)| version),
        "other_version": other_version,
        "sharing": sharing,
        "live": if sharing { stream::live_json(core, room, &p.id.to_string()) } else { None },
    })
}

fn last_used(core: &Core, room: &str) -> Option<i64> {
    std::fs::metadata(core.log_path(room, &core.me())).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_micros() as i64)
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
    if let Some(name) = latest_name(&logs) {
        keep_name(core, &room_id, &name);
    }
    let active = core.active_room();
    let caps = core.party.lock().unwrap().caps();
    let mut events = events(&logs, caps.as_ref());
    let sets = sets_json(core, &room_id, &logs, &events);
    let recorded: Vec<(u64, i64)> = sets.iter().filter_map(|s| Some((s["progress"]["total"].as_u64()?, s["end_us"].as_i64()? - s["start_us"].as_i64()?))).collect();
    let disk = data::disk_json(core.free_bytes.load(Ordering::Relaxed), core.total_bytes.load(Ordering::Relaxed), data::bytes_per_hour(&recorded), *core.warn_hours.lock().unwrap(), core.disk_full());
    let seekable: HashSet<&str> = sets.iter().filter(|s| s["progress"]["have"].as_u64() > Some(0)).flat_map(|s| s["events"].as_array().into_iter().flatten().filter_map(|k| k.as_str())).collect();
    for e in events.iter_mut() {
        let (can_seek, mine) = (seekable.contains(e["key"].as_str().unwrap_or_default()) && e["local_us"].is_i64(), e["author"] == my_id.as_str() && e["plugin"].is_null());
        e["seekable"] = json!(can_seek);
        e["mine"] = json!(mine);
    }
    let mut asks: Vec<Value> = core.asks.lock().unwrap().iter().filter(|a| a.answer.is_none()).map(|a| json!({"id": a.id, "name": a.name})).collect();
    for p in core.peers.lock().unwrap().iter().filter(|p| p.alive().is_some()) {
        for a in &p.asks {
            if !asks.iter().any(|b| b["id"] == a["id"]) {
                asks.push(a.clone());
            }
        }
    }
    let people = active.as_ref().map(Room::people).unwrap_or_default();
    let rooms = core.rooms.lock().unwrap();
    let room_list: Vec<Value> = rooms.list.iter().map(|r| json!({"id": r.id, "name": r.name, "active": rooms.active.as_deref() == Some(&r.id), "members": r.members, "names": r.names, "colors": colors(&r.people()), "used_us": last_used(core, &r.id)})).collect();
    drop(rooms);
    json!({
        "version": VERSION,
        "ffmpeg": core.ffmpeg_status.lock().unwrap().clone(),
        "id": my_id,
        "name": core.name.lock().unwrap().clone(),
        "lang": crate::language(core),
        "language": *core.language.lock().unwrap(),
        "colors": colors(&people),
        "palette": PALETTE,
        "audio_process": core.audio_process.lock().unwrap().clone(),
        "invites": core.invites.lock().unwrap().len(),
        "limit_mbps": *core.limit_mbps.lock().unwrap(),
        "recovery": core.recovery.lock().unwrap().clone(),
        "notice": core.notice.lock().unwrap().clone(),
        "disk": disk,
        "warn_hours": *core.warn_hours.lock().unwrap(),
        "channel": *core.channel.lock().unwrap(),
        "previous_version": core.previous_version,
        "codec": ffmpeg::pick(&core.codecs.lock().unwrap()).1,
        "recording": core.active_set.lock().unwrap().is_some(),
        "capture": record::capture_json(core, stream::live_json(core, &room_id, &my_id)),
        "active_set": core.active_set_json(),
        "room": active.map(|r| json!({"id": r.id, "name": r.name, "pending": r.invite.is_some(), "waiting": r.waiting})),
        "rooms": room_list,
        "layout": core.layout.lock().unwrap().clone(),
        "hotkey": core.hotkey.lock().unwrap().clone(),
        "api": {"on": *core.api_on.lock().unwrap(), "address": crate::api::address(), "error": core.api_error.lock().unwrap().clone()},
        "sets": sets,
        "gone": gone_json(&room_id, &logs, &core.deleted.lock().unwrap(), &events),
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
    fn sets_come_from_rec_and_rec_end_records() {
        let a = log("a", 0, vec![
            json!({"k": "rec", "set": "s1", "start_us": 100}),
            json!({"k": "rec_end", "set": "s1", "t_us": 900}),
            json!({"k": "rec_end", "set": "s1", "t_us": 950}),
            json!({"k": "rec", "set": "s2", "start_us": 1000}),
            json!({"k": "rec", "set": "../bad", "start_us": 1}),
        ]);
        let b = log("b", 50, vec![json!({"k": "rec", "set": "s1", "start_us": 200}), json!({"k": "rec", "set": "s1", "start_us": 300})]);
        let sets = derive_sets("r", &[a, b], &HashSet::new());
        assert_eq!(sets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s2", "s1"]);
        let s1 = &sets[1];
        assert_eq!(s1.files.len(), 2);
        assert_eq!(s1.files[0].end_us, Some(900));
        assert_eq!(s1.start(), 100);
        assert_eq!(s1.end(), None);
        assert!(sets[0].files[0].end_us.is_none());
        let rejoined = derive_sets("r", &[log("a", 0, vec![
            json!({"k": "rec", "set": "s1", "start_us": 100}),
            json!({"k": "rec_end", "set": "s1", "t_us": 400}),
            json!({"k": "rec", "set": "s1", "start_us": 600}),
        ])], &HashSet::new());
        assert_eq!((rejoined[0].files.len(), rejoined[0].files[0].start_us, rejoined[0].files[0].end_us), (1, 100, None));
        let deleted: HashSet<String> = ["r/s2".to_string()].into();
        assert_eq!(derive_sets("r", &[log("a", 0, vec![json!({"k": "rec", "set": "s2"})])], &deleted).len(), 0);
    }

    fn seg(capture: &str, series: &str, n: u32, set: &str) -> Value {
        json!({"k": "seg", "capture": capture, "series": series, "n": n, "set": set, "t_us": n as i64 * 4_000_000, "d_us": 4_000_000, "size": 100, "sha256": "x"})
    }

    #[test]
    fn set_contents_come_from_seg_records() {
        let a = log("a", 0, vec![
            json!({"k": "cap", "capture": "111", "start_us": 0, "init": {"v": {"size": 7, "sha256": "i"}, "m": {"size": 3, "sha256": "j"}}}),
            seg("111", "v", 1, "s0"),
            json!({"k": "rec", "set": "s1", "start_us": 8_000_000}),
            seg("111", "v", 2, "s1"),
            seg("111", "m", 2, "s1"),
            seg("111", "v", 3, "s9"),
            seg("../x", "v", 4, "s1"),
            seg("111", "q", 4, "s1"),
            json!({"k": "cap", "capture": "222", "start_us": 9}),
            seg("222", "v", 1, "s1"),
            seg("111", "v", 2, "s1"),
            json!({"k": "rec_end", "set": "s1", "t_us": 20_000_000}),
        ]);
        let sets = derive_sets("r", &[a], &HashSet::new());
        let f = &sets[0].files[0];
        assert_eq!(f.segs.iter().map(|s| (s.capture.as_str(), s.series.as_str(), s.n)).collect::<Vec<_>>(), [("111", "v", 2), ("111", "m", 2), ("222", "v", 1)]);
        assert_eq!(f.captures(), ["111", "222"]);
        assert_eq!(f.init("111", "m"), Some((3, "j".into())));
        assert_eq!(f.init("222", "v"), None);
        assert_eq!(f.total(), 300 + 7 + 3);
        assert_eq!(f.series("v").len(), 2);
        let key = |s: &str| have_key("s1", "a", s);
        let all: HashMap<String, u64> = [(key("v"), 2), (key("m"), 1)].into();
        let some: HashMap<String, u64> = [(key("v"), 1)].into();
        assert!(full(&all, &sets[0], f));
        assert!(!full(&some, &sets[0], f));
        assert!(!stuck(&some, &[&all], &sets[0], f));
        assert!(stuck(&some, &[&some], &sets[0], f));
        assert!(!stuck(&all, &[], &sets[0], f));
    }

    #[test]
    fn deleted_sessions_are_listed_apart_with_what_they_held() {
        let a = log("a", 0, vec![
            json!({"k": "rec", "set": "s1", "start_us": 100}),
            seg("111", "m", 1, "s1"),
            json!({"k": "rec_end", "set": "s1", "t_us": 900}),
            json!({"k": "rec", "set": "s2", "start_us": 1000}),
        ]);
        let deleted: HashSet<String> = ["r/s1".to_string()].into();
        let gone = gone_json("r", &[a], &deleted, &[json!({"key": "a/1", "local_us": 500, "set": null})]);
        assert_eq!(gone.len(), 1);
        assert_eq!((gone[0]["id"].as_str(), gone[0]["end_us"].as_i64(), gone[0]["players"][0]["total"].as_u64(), gone[0]["players"][0]["voice"].as_bool()), (Some("s1"), Some(900), Some(100), Some(true)));
        assert_eq!(gone[0]["events"], json!(["a/1"]));
    }

    #[test]
    fn the_relay_shows_by_its_host_name() {
        assert_eq!(relay_host("Relay(RelayUrl(\"https://use1-1.relay.n0.example/\"))").as_deref(), Some("use1-1.relay.n0.example"));
        assert_eq!(relay_host("Ip(192.0.2.1:5000)"), None);
    }

    #[test]
    fn log_lines_carry_the_local_wall_clock() {
        assert_eq!(wall_clock(1_760_801_895_123_456, 9 * 3600), "2025-10-19T00:38:15.123");
        assert_eq!(wall_clock(0, -3600), "1969-12-31T23:00:00.000");
    }

    #[test]
    fn tags_are_read_from_text() {
        assert_eq!(tags("push #Rotate then #rotate and #ジャンプ, #"), vec!["Rotate", "ジャンプ"]);
        assert!(tags("no tags").is_empty());
    }

    #[test]
    fn events_merge_with_their_edits() {
        let a = log("a", 10, vec![
            json!({"k": "event", "id": 1, "t_us": 1, "kind": "note", "target": {"t_us": 500}, "text": "first #one"}),
            json!({"k": "event_edit", "id": 1, "text": "edited #two"}),
            json!({"k": "event", "id": 2, "kind": "note", "target": {"t_us": 700}, "text": "new", "tags": ["x"]}),
            json!({"k": "event_del", "id": 2}),
            json!({"k": "event", "id": 3, "kind": "marker", "target": {"from_us": 100, "to_us": 200}, "text": ""}),
            json!({"k": "plugin", "plugin": "apex", "on": true, "version": "1", "settings": {}, "t_us": 1}),
            json!({"k": "event", "id": 4, "kind": "round", "plugin": {"name": "apex", "version": "1"}, "target": {"set": "s1"}, "values": {"n": 2}, "tags": ["p"], "run": 5}),
            json!({"k": "job", "state": "done", "job": "apex@1/44136fa355b3/s1/a", "run": 5}),
            json!({"k": "event_edit", "id": 4, "text": "hijack"}),
            json!({"k": "event_del", "id": 9}),
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
    fn peer_events_are_cleaned() {
        let a = log("a", 0, vec![json!({"k": "event", "id": 1, "kind": "note", "target": {"t_us": 5, "file": 7, "set": [1]}, "text": "x".repeat(crate::BOARD_MAX + 9), "tags": ["ok", 5, null]})]);
        let e = &events(&[a], None)[0];
        assert_eq!((e["tags"].clone(), e["file"].clone(), e["set"].clone()), (json!(["ok"]), Value::Null, Value::Null));
        assert_eq!(e["text"].as_str().unwrap().len(), crate::BOARD_MAX);
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
        let set = Set { id: "s1".into(), room: "r".into(), files: vec![File { owner: "a".into(), start_us: 100, end_us: Some(200), offset_us: 0, segs: Vec::new(), caps: HashMap::new() }] };
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
    fn the_latest_room_name_wins_on_every_clock() {
        assert_eq!(latest_name(&[log("a", 0, vec![json!({"k": "rec"})])]), None);
        let a = log("a", 0, vec![json!({"k": "room_name", "name": "Old", "t_us": 100}), json!({"k": "room_name", "name": "Newer", "t_us": 300})]);
        let b = log("b", 1000, vec![json!({"k": "room_name", "name": "  Late  ", "t_us": 1200})]);
        assert_eq!(latest_name(&[a, b]), Some("Newer".to_string()));
        let c = log("c", -500, vec![json!({"k": "room_name", "name": "Last", "t_us": -150})]);
        assert_eq!(latest_name(&[c, log("a", 0, vec![json!({"k": "room_name", "name": "First", "t_us": 340})])]), Some("Last".to_string()));
    }

    #[test]
    fn a_rename_made_here_beats_a_name_from_a_fast_clock() {
        let ahead = log("b", 0, vec![json!({"k": "room_name", "name": "Theirs", "t_us": 5000})]);
        let t = named(std::slice::from_ref(&ahead)).map_or(0, |(t, ..)| 1000i64.max(t + 1));
        let mine = log("a", 0, vec![json!({"k": "room_name", "name": "Mine", "t_us": t})]);
        assert_eq!(latest_name(&[ahead, mine]), Some("Mine".to_string()));
    }

    #[test]
    fn invite_tokens_and_room_ids_cannot_leave_the_rooms_folder() {
        assert!(valid_token(&random_hex(TOKEN_BYTES)));
        assert!(!valid_token("0123456789abcdef/../x"));
        assert!(!valid_token("../../../../evil00"));
        assert!(!valid_id("../outside"));
        assert!(!valid_id("a\\b"));
        assert!(valid_id("1a2b3c4d"));
    }

    #[test]
    fn a_stop_in_any_log_ends_the_set_and_a_crash_lets_its_member_rejoin() {
        let a = log("a", 0, vec![json!({"k": "rec", "set": "s1"}), json!({"k": "rec_end", "set": "s1", "stop": false})]);
        let b = log("b", 0, vec![json!({"k": "rec_end", "set": "s2", "stop": true})]);
        assert!(!joined(&a.records, "s1"));
        assert!(joined(&[json!({"k": "rec_end", "set": "s1"}), json!({"k": "rec", "set": "s1"})], "s1"));
        assert!(!joined(&[json!({"k": "rec", "set": "s2"})], "s1"));
        let crashed = [json!({"k": "rec", "set": "s1"}), json!({"k": "rec_end", "set": "s1"})];
        assert!(may_join(&crashed, "s1"));
        assert!(may_join(&crashed, "s2"));
        let moved = [crashed[0].clone(), crashed[1].clone(), json!({"k": "rec", "set": "s2"}), json!({"k": "rec_end", "set": "s2"})];
        assert!(!may_join(&moved, "s1"));
        assert!(may_join(&moved, "s2"));
        assert!(!may_join(&moved[..3], "s2"));
        let logs = [a, b];
        assert!(!stopped(&logs, "s1"));
        assert!(stopped(&logs, "s2"));
    }

    #[test]
    fn colors_are_the_same_everywhere() {
        let ids: Vec<String> = (0..5).map(|i| format!("{i:064x}")).collect();
        let a = colors(&ids);
        let mut reversed = ids.clone();
        reversed.reverse();
        assert_eq!(a, colors(&reversed));
        let mut seen: Vec<usize> = a.values().copied().collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), ids.len());
    }

    #[test]
    fn colors_stay_when_someone_joins_without_a_clash() {
        let first = "a".repeat(64);
        let alone = colors(std::slice::from_ref(&first))[&first];
        assert_eq!(alone, preferred_color(&first));
        let newcomer = (0..).map(|i| format!("{i:064x}")).find(|id| preferred_color(id) != alone).unwrap();
        assert_eq!(colors(&[first.clone(), newcomer])[&first], alone);
        let clash = (0..).map(|i| format!("{i:064x}")).find(|id| preferred_color(id) == alone && *id != first).unwrap();
        let both = colors(&[first.clone(), clash.clone()]);
        assert_ne!(both[&first], both[&clash]);
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

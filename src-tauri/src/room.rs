use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use tauri::Emitter;

use crate::{clean_text, data, ffmpeg, hex, hw, net, now_us, party, record, short, stream, Core, Peer, PROTOCOL, ROOM_NAME_MAX, VERSION};

pub const SOLO: &str = "solo";
pub const SERIES: [&str; 2] = ["v", "m"];
pub const INIT: &str = "init.mp4";
pub const RECORDINGS: &str = "recordings";
pub const STANDBY: &str = "standby";
pub const LOGS: &str = "logs";
pub const PENDING: &str = "pending.bin";
pub const TOKEN_BYTES: usize = 8;
pub const UNREACHABLE_AFTER: Duration = Duration::from_secs(60);
const ROOM_ID_BYTES: usize = 4;
const TITLE_MAX: usize = 80;

pub fn seg_name(n: u32) -> String {
    format!("{n:06}.m4s")
}

pub fn seg_number(name: &str) -> Option<u32> {
    name.strip_suffix(".part").unwrap_or(name).strip_suffix(".m4s")?.parse().ok()
}

pub fn recording_dir(room_dir: &Path, set: &str) -> PathBuf {
    room_dir.join(RECORDINGS).join(set)
}

pub fn log_file(room_dir: &Path, id: &str) -> PathBuf {
    room_dir.join(LOGS).join(format!("{id}.bin"))
}

pub fn series_dir(room_dir: &Path, set: Option<&str>, owner: &str, capture: &str, series: &str) -> PathBuf {
    set.map_or_else(|| room_dir.join(STANDBY), |set| recording_dir(room_dir, set)).join(owner).join(capture).join(series)
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
    pub of: Option<String>,
}

pub fn random_hex(bytes: usize) -> String {
    hex(&iroh::SecretKey::generate().to_bytes()[..bytes])
}

pub fn capture_id(text: &str) -> bool {
    !text.is_empty() && text.len() <= 20 && text.chars().all(|c| c.is_ascii_digit())
}

fn hex_of(text: &str, bytes: usize) -> bool {
    text.len() == 2 * bytes && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn valid_token(text: &str) -> bool {
    hex_of(text, TOKEN_BYTES)
}

pub fn member_id(text: &str) -> bool {
    text.parse::<EndpointId>().is_ok()
}

pub fn valid_id(text: &str) -> bool {
    !text.is_empty() && text.len() <= 40 && text.trim() == text && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == ' ')
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

    pub fn room_dir(&self, room: &str) -> PathBuf {
        self.dir.join("rooms").join(room)
    }

    pub fn recording_dir(&self, room: &str, set: &str) -> PathBuf {
        recording_dir(&self.room_dir(room), set)
    }

    pub fn series_dir(&self, room: &str, set: Option<&str>, owner: &str, capture: &str, series: &str) -> PathBuf {
        series_dir(&self.room_dir(room), set, owner, capture, series)
    }

    pub fn save_rooms(&self) {
        let text = serde_json::to_string(&*self.rooms.lock().unwrap()).unwrap_or_default();
        let _ = crate::write_whole(&self.dir.join("rooms.json"), text.as_bytes());
    }

    pub fn active_set_json(&self) -> Value {
        match self.active_set.lock().unwrap().as_ref() {
            Some(s) => json!({"id": s.id, "room": s.room, "by": s.by, "start_us": s.start_us, "voice": s.voice, "of": s.of}),
            None => Value::Null,
        }
    }

    pub fn is_recording(&self, room: &str, set: &str) -> bool {
        self.active_set.lock().unwrap().as_ref().is_some_and(|a| a.id == set && a.room == room)
    }

    pub fn in_use(&self, room: &str, set: &str) -> bool {
        self.is_recording(room, set) || self.export.lock().unwrap().as_ref().is_some_and(|e| e.set.0 == room && e.set.1 == set)
    }

    pub fn present(&self, owner: &str) -> bool {
        if owner == self.me() {
            return true;
        }
        let peers = self.peers.lock().unwrap();
        let peer = peers.iter().find(|p| p.id.to_string() == owner);
        reachable(peer.is_some_and(|p| p.alive().is_some()), peer.and_then(|p| p.seen_us), now_us(), self.started.elapsed())
    }
}

pub fn reachable(connected: bool, seen_us: Option<i64>, now_us: i64, up: Duration) -> bool {
    connected || seen_us.map_or(up < UNREACHABLE_AFTER, |seen| now_us - seen < UNREACHABLE_AFTER.as_micros() as i64)
}

pub fn display_name(core: &Core, id: &str) -> String {
    if id == core.ep.id().to_string() {
        return core.name.lock().unwrap().clone();
    }
    let peer = core.peers.lock().unwrap().iter().find(|p| p.id.to_string() == id && !p.name.is_empty()).map(|p| p.name.clone());
    peer.or_else(|| core.active_room().and_then(|r| r.names.get(id).cloned()))
        .or_else(|| core.active_room().and_then(|r| view_info(&core.room_log(&r.id, id)).map(|v| v.name)))
        .unwrap_or_else(|| short(id))
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
    let mut peers = core.peers.lock().unwrap();
    peers.retain(|p| {
        let keep = wanted.contains(&p.id);
        if !keep {
            p.task.abort();
            for conn in p.conn.iter() {
                conn.close(2u32.into(), b"left");
            }
        }
        keep
    });
    for id in &wanted {
        if !peers.iter().any(|p| p.id == *id) {
            let task = tauri::async_runtime::spawn(net::peer_loop(core.clone(), *id));
            peers.push(Peer::new(*id, room.names.get(&id.to_string()).cloned().unwrap_or_default(), task));
        }
    }
}

pub fn leave(core: &Arc<Core>) {
    core.rooms.lock().unwrap().active = None;
    core.save_rooms();
    end_set(core, false, None);
    apply_active(core);
}

fn open(core: &Arc<Core>, id: &str) {
    if core.room_id().is_some() {
        leave(core);
    }
    core.rooms.lock().unwrap().active = Some(id.to_string());
    core.save_rooms();
    apply_active(core);
}

fn activate(core: &Arc<Core>, room: Room) {
    let id = room.id.clone();
    {
        let mut rooms = core.rooms.lock().unwrap();
        rooms.list.retain(|r| r.id != id);
        rooms.list.push(room);
    }
    open(core, &id);
}

pub fn create_room(core: &Arc<Core>, name: &str) -> Result<String, String> {
    let id = random_hex(ROOM_ID_BYTES);
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
    id.parse::<EndpointId>().ok().filter(|id| *id != core.ep.id()).ok_or_else(bad)?;
    let members = vec![core.ep.id().to_string(), id.to_string()];
    let invite = Some((id.to_string(), token.to_string()));
    activate(core, Room { id: format!("pending-{token}"), name: "Joining...".into(), members, invite, ..Default::default() });
    Ok(())
}

pub fn enter_room(core: &Arc<Core>, id: &str) -> Result<(), String> {
    if core.room_id().as_deref() == Some(id) {
        return Ok(());
    }
    if !core.rooms.lock().unwrap().list.iter().any(|r| r.id == id) {
        return Err("That room is not in your list.".into());
    }
    open(core, id);
    Ok(())
}

pub fn leave_room(core: &Arc<Core>, id: &str) {
    if core.rooms.lock().unwrap().active.as_deref() == Some(id) {
        leave(core);
    }
}

pub fn forget_room(core: &Arc<Core>, id: &str) {
    leave_room(core, id);
    core.rooms.lock().unwrap().list.retain(|r| r.id != id);
    core.save_rooms();
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
    valid_id(room) && member_id(of) && core.rooms.lock().unwrap().list.iter().find(|r| r.id == room).is_some_and(|r| shares(r, asker, of))
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
    if !hex_of(id, ROOM_ID_BYTES) {
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

pub fn begin_set(core: &Arc<Core>, room: &str, set: &str, by: &str, start_us: i64, voice: bool, of: Option<String>) -> Result<bool, String> {
    ffmpeg::ready()?;
    if core.disk_full() {
        return Err("The disk is almost full.".into());
    }
    {
        let mut slot = core.active_set.lock().unwrap();
        if slot.is_some() || core.room_id().as_deref() != Some(room) {
            return Ok(false);
        }
        *slot = Some(ActiveSet { id: set.to_string(), room: room.to_string(), by: by.to_string(), start_us, voice, of });
    }
    tracing::info!("{} {set} started by {by}", if voice { "review session" } else { "session" });
    Ok(true)
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

pub fn apply_stops(core: &Arc<Core>) {
    let Some(active) = core.active_set.lock().unwrap().clone() else { return };
    if sets(core, &active.room, &logs(core, &active.room)).iter().any(|s| s.id == active.id && s.stopped) {
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

pub fn unused_set(logs: &[Log], room_dir: &Path, start_us: i64, tz_offset_s: i64) -> String {
    let taken: HashSet<&str> = logs.iter().flat_map(|l| l.records.iter()).filter(|r| r["k"] == "rec" || r["k"] == "set_del").filter_map(|r| r["set"].as_str()).collect();
    (0..).map(|i| set_id_for(start_us + i * 1_000_000, tz_offset_s)).find(|id| !taken.contains(id.as_str()) && !recording_dir(room_dir, id).exists()).unwrap_or_default()
}

pub fn start_set(core: &Arc<Core>, debrief: Option<bool>) -> Result<(), String> {
    let start_us = now_us();
    let room = core.active_room().filter(|r| r.invite.is_none()).map(|r| r.id).ok_or("Join a room first.")?;
    let set = unused_set(&logs(core, &room), &core.room_dir(&room), start_us, core.tz_offset_s);
    let by = core.name.lock().unwrap().clone();
    let open = core.party.lock().unwrap().open_set();
    let voice = debrief.unwrap_or(open.is_some());
    let of = open.filter(|_| voice);
    if !begin_set(core, &room, &set, &by, start_us, voice, of.clone())? {
        return Ok(());
    }
    log_set_layout(core);
    party::log(core);
    net::broadcast(core, json!({"t": "rec", "action": "start", "set": set, "room": room, "by": by, "start_us": start_us, "voice": voice, "of": of}));
    announce(core, "start", &by, voice);
    Ok(())
}

pub fn announce(core: &Core, action: &str, by: &str, voice: bool) {
    let _ = core.app.emit("session", json!({"action": action, "by": by, "voice": voice}));
}

pub fn stop_set(core: &Arc<Core>) {
    let set = core.active_set.lock().unwrap().as_ref().map(|s| s.id.clone());
    end_set(core, true, None);
    if let Some(set) = set {
        let by = core.name.lock().unwrap().clone();
        net::broadcast(core, json!({"t": "rec", "action": "stop", "set": set, "room": core.active_room_id(), "by": by}));
        announce(core, "end", &by, false);
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
    pub set: String,
    pub owner: String,
    pub start_us: i64,
    pub end_us: Option<i64>,
    pub offset_us: i64,
    pub segs: Vec<Seg>,
    pub caps: HashMap<String, Value>,
    pub view: Option<ViewInfo>,
    pub present: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewInfo {
    pub name: String,
    pub by: String,
    pub title: Option<String>,
}

pub fn view_info(records: &[Value]) -> Option<ViewInfo> {
    let r = records.iter().find(|r| r["k"] == "rec")?;
    let by = r["imported"].as_str().filter(|b| member_id(b))?;
    let name = crate::clean_name(r["view"].as_str()?);
    (!name.is_empty()).then(|| ViewInfo { name, by: by.to_string(), title: r["title"].as_str().map(|t| crate::clean_text(t, TITLE_MAX)).filter(|t| !t.is_empty()) })
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

    pub fn last_us(&self) -> i64 {
        self.end_us.unwrap_or_else(|| self.segs.iter().map(|s| s.t_us + s.d_us).max().unwrap_or(self.start_us))
    }

    pub fn recording(&self) -> bool {
        self.end_us.is_none() && self.present
    }
}

pub struct Set {
    pub id: String,
    pub room: String,
    pub files: Vec<File>,
    pub stopped: bool,
}

impl Set {
    pub fn start(&self) -> i64 {
        self.files.iter().map(|f| f.start_us - f.offset_us).min().unwrap_or(0)
    }

    pub fn recording(&self) -> bool {
        !self.stopped && self.files.iter().any(File::recording)
    }

    pub fn end_shown(&self) -> Option<i64> {
        if self.recording() {
            return None;
        }
        self.files.iter().map(|f| f.last_us() - f.offset_us).max()
    }

    pub fn covers(&self, t: i64) -> bool {
        self.start() <= t && self.end_shown().is_none_or(|end| t <= end)
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
    for id in people.iter().filter(|id| member_id(id) && **id != me) {
        let records = core.room_log(room, id);
        let offset_us = match view_info(&records) {
            Some(view) if view.by == me => 0,
            Some(view) => offsets.get(&short(&view.by)).copied().unwrap_or(0),
            None => offsets.get(&short(id)).copied().unwrap_or(0),
        };
        out.push(Log { owner: id.clone(), offset_us, records });
    }
    out
}

pub fn imported_views(core: &Core, room: &str) -> HashMap<String, ViewInfo> {
    let people = core.rooms.lock().unwrap().list.iter().find(|r| r.id == room).map(|r| r.removed.clone()).unwrap_or_default();
    people.into_iter().filter(|id| member_id(id)).filter_map(|id| Some((id.clone(), view_info(&core.room_log(room, &id))?))).collect()
}

pub fn sets(core: &Core, room: &str, logs: &[Log]) -> Vec<Set> {
    with_presence(derive_sets(room, logs), |owner| core.present(owner))
}

pub fn with_presence(mut sets: Vec<Set>, present: impl Fn(&str) -> bool) -> Vec<Set> {
    for f in sets.iter_mut().flat_map(|s| s.files.iter_mut()) {
        f.present = present(&f.owner);
    }
    sets
}

pub fn dropped(logs: &[Log]) -> HashSet<String> {
    logs.iter().flat_map(|l| l.records.iter()).filter(|r| r["k"] == "set_del").filter_map(|r| r["set"].as_str().map(String::from)).collect()
}

fn seg_from(r: &Value) -> Option<Seg> {
    let capture = r["capture"].as_str().filter(|c| capture_id(c))?;
    let series = r["series"].as_str().filter(|s| SERIES.contains(s))?;
    let n = r["n"].as_u64().filter(|n| *n > 0 && *n <= u32::MAX as u64)? as u32;
    Some(Seg { capture: capture.into(), series: series.into(), n, t_us: r["t_us"].as_i64()?, d_us: r["d_us"].as_i64().unwrap_or(0), size: r["size"].as_u64()?, sha256: r["sha256"].as_str()?.into() })
}

pub fn derive_sets(room: &str, logs: &[Log]) -> Vec<Set> {
    let dropped = dropped(logs);
    let stopped: HashSet<&str> = logs.iter().flat_map(|l| l.records.iter()).filter(|r| r["k"] == "rec_end" && r["stop"] == true).filter_map(|r| r["set"].as_str()).collect();
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
                    if dropped.contains(id) || sets.get(id).is_some_and(|s| s.files.iter().any(|f| f.owner == log.owner)) {
                        continue;
                    }
                    let set = sets.entry(id.to_string()).or_insert_with(|| Set { id: id.to_string(), room: room.to_string(), files: Vec::new(), stopped: stopped.contains(id) });
                    open.insert(id, (set.files.len(), false));
                    set.files.push(File { set: id.to_string(), owner: log.owner.clone(), start_us: r["start_us"].as_i64().unwrap_or(0), end_us: None, offset_us: log.offset_us, segs: Vec::new(), caps: HashMap::new(), view: view_info(&log.records), present: false });
                }
                Some("rec_end") => {
                    if let Some((i, ended)) = open.get_mut(id).filter(|(_, ended)| !ended) {
                        *ended = true;
                        sets.get_mut(id).unwrap().files[*i].end_us = r["t_us"].as_i64();
                    }
                }
                Some("seg") => {
                    if let (Some((i, _)), Some(seg)) = (open.get(id), seg_from(r).filter(|s| seen.insert((id, s.capture.clone(), s.series.clone(), s.n)))) {
                        sets.get_mut(id).unwrap().files[*i].segs.push(seg);
                    }
                }
                _ => {}
            }
        }
        for (id, (i, _)) in &open {
            let f = &mut sets.get_mut(*id).unwrap().files[*i];
            f.caps = f.captures().into_iter().filter_map(|c| Some((c.to_string(), caps.get(c)?.clone()))).collect();
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
        "source": if r["source"] == "ai" { json!("ai") } else { Value::Null }, "at_us": id - offset_us,
    })
}

fn ai_cutoffs(logs: &[Log]) -> HashMap<String, i64> {
    let mut out: HashMap<String, i64> = HashMap::new();
    for log in logs {
        for r in log.records.iter().filter(|r| r["k"] == "ai_del") {
            if let (Some(set), Some(t)) = (r["set"].as_str(), r["t_us"].as_i64()) {
                let at = out.entry(set.to_string()).or_insert(i64::MIN);
                *at = (*at).max(t - log.offset_us);
            }
        }
    }
    out
}

fn ai_hidden(event: &Value, cutoffs: &HashMap<String, i64>) -> bool {
    event["source"] == "ai" && event["set"].as_str().and_then(|set| cutoffs.get(set)).is_some_and(|cut| event["at_us"].as_i64().unwrap_or(i64::MAX) < *cut)
}

pub fn events(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> Vec<Value> {
    let mut out = Vec::new();
    let cutoffs = ai_cutoffs(logs);
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
        out.extend(list.into_iter().filter(|n| n["deleted"] == false && !ai_hidden(n, &cutoffs)));
    }
    crate::jobs::keep_shown(&mut out, logs, caps);
    let gone = dropped(logs);
    out.retain(|e| e["set"].as_str().is_none_or(|set| !gone.contains(set)));
    out.sort_by_key(|n| n["local_us"].as_i64());
    out
}

pub fn others_hold(set: &str, others: &[&HashMap<String, u64>]) -> bool {
    let prefix = format!("{set}/");
    others.iter().any(|have| have.iter().any(|(key, n)| *n > 0 && key.starts_with(&prefix)))
}

pub fn auto_receive(live: bool, here: bool, choice: Option<bool>) -> bool {
    choice.unwrap_or(live || here)
}

fn want_key(room: &str, set: &str) -> String {
    format!("{room}/{set}")
}

pub fn wants(core: &Core, room: &str, set: &str) -> Option<bool> {
    core.want.lock().unwrap().get(&want_key(room, set)).copied()
}

pub fn download_set(core: &Core, room: &str, set: &str) -> Result<(), String> {
    if !valid_id(room) || !valid_id(set) {
        return Err("That recording does not exist.".into());
    }
    core.want.lock().unwrap().insert(want_key(room, set), true);
    Ok(())
}

fn remove_set(core: &Core, room: &str, set: &str, complete: bool) -> Result<bool, String> {
    if !valid_id(room) || !valid_id(set) {
        return Err("That recording does not exist.".into());
    }
    if core.is_recording(room, set) {
        return Err("This recording is still in progress.".into());
    }
    if core.in_use(room, set) {
        return Err("This recording is being exported. Cancel the export first.".into());
    }
    core.want.lock().unwrap().insert(want_key(room, set), false);
    crate::jobs::cancel_set(room, set);
    if complete {
        core.append_log(room, &json!({"k": "set_del", "room": room, "set": set, "t_us": now_us()}));
    }
    remove_recording(&core.room_dir(room), set);
    data::forget(&core.files, &core.recording_dir(room, set));
    Ok(complete)
}

pub fn delete_set(core: &Core, room: &str, set: &str) -> Result<bool, String> {
    let debriefs = named_debriefs(&logs(core, room), set);
    let complete = !others_hold(set, &core.peers.lock().unwrap().iter().map(|p| &p.have).collect::<Vec<_>>());
    let deleted = remove_set(core, room, set, complete)?;
    for debrief in debriefs {
        let _ = delete_set(core, room, &debrief);
    }
    Ok(deleted)
}

fn started<'a>(logs: &'a [Log], set: &str) -> impl Iterator<Item = &'a Value> + 'a {
    let set = set.to_string();
    logs.iter().flat_map(|l| l.records.iter()).filter(move |r| r["k"] == "rec" && r["set"] == set.as_str())
}

pub fn debrief_of(set: &str, logs: &[Log]) -> Option<String> {
    let named = started(logs, set).find_map(|r| r["of"].as_str().filter(|of| valid_id(of)));
    let parent = match named {
        Some(parent) => parent.to_string(),
        None if started(logs, set).any(|r| r["voice"] == true) => party::records(logs).remove(set)?.into_iter().next()?["shared"]["set"].as_str()?.to_string(),
        None => return None,
    };
    (parent != set && started(logs, &parent).next().is_some() && !dropped(logs).contains(&parent)).then_some(parent)
}

pub fn named_debriefs(logs: &[Log], parent: &str) -> Vec<String> {
    let gone = dropped(logs);
    let mut out: Vec<String> = Vec::new();
    for r in logs.iter().flat_map(|l| l.records.iter()).filter(|r| r["k"] == "rec" && r["of"] == parent) {
        if let Some(set) = r["set"].as_str().filter(|s| valid_id(s) && *s != parent && !gone.contains(*s)) {
            if !out.iter().any(|o| o == set) {
                out.push(set.to_string());
            }
        }
    }
    out
}

pub fn board_card_valid(card: &str) -> bool {
    card.len() <= 60 && (card == "team" || card.strip_prefix("m:").or_else(|| card.strip_prefix("c:")).is_some_and(|id| !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')))
}

pub fn board_cards(events: &[Value], members: &[String]) -> Vec<Value> {
    let mut cards: BTreeMap<String, ((i64, String), Value)> = BTreeMap::new();
    let mut first: HashMap<String, f64> = members.iter().enumerate().map(|(i, member)| (format!("m:{member}"), i as f64)).collect();
    first.insert("team".into(), -1.0);
    for (card, pos) in &first {
        cards.insert(card.clone(), ((i64::MIN, String::new()), json!({"title": null, "text": "", "pos": pos, "gone": false})));
    }
    for e in events.iter().filter(|e| e["kind"] == "board") {
        let card = e["values"]["card"].as_str().filter(|c| !c.is_empty()).unwrap_or("team");
        let at = (e["at_us"].as_i64().unwrap_or(0), e["author"].as_str().unwrap_or_default().to_string());
        if cards.get(card).is_some_and(|(old, _)| *old >= at) {
            continue;
        }
        let pos = if card == "team" { -1.0 } else { e["values"]["pos"].as_f64().or_else(|| first.get(card).copied()).unwrap_or(f64::MAX) };
        let state = json!({"title": e["values"]["title"].as_str().filter(|t| !t.is_empty()), "text": e["text"], "pos": pos, "gone": e["values"]["gone"] == true, "by": e["author"], "at_us": at.0});
        cards.insert(card.to_string(), (at, state));
    }
    let mut out: Vec<Value> = cards.into_iter().map(|(card, (_, mut state))| { state["card"] = json!(card); state }).collect();
    out.sort_by(|a, b| a["pos"].as_f64().unwrap_or(f64::MAX).total_cmp(&b["pos"].as_f64().unwrap_or(f64::MAX)).then_with(|| a["card"].as_str().cmp(&b["card"].as_str())));
    out
}

fn remove_recording(room_dir: &Path, set: &str) {
    let _ = std::fs::remove_dir_all(recording_dir(room_dir, set));
    let _ = std::fs::remove_file(crate::thumbs::path(room_dir, set));
}

pub fn sweep(room_dir: &Path, gone: &HashSet<String>, recording: Option<&str>) -> Vec<String> {
    let mut out = Vec::new();
    for set in gone.iter().filter(|set| valid_id(set) && Some(set.as_str()) != recording) {
        if recording_dir(room_dir, set).exists() || crate::thumbs::path(room_dir, set).exists() {
            remove_recording(room_dir, set);
            out.push(set.clone());
        }
    }
    out
}

pub fn sweep_dropped(core: &Core, room: &str, logs: &[Log]) {
    let recording = core.active_set.lock().unwrap().as_ref().filter(|a| a.room == room).map(|a| a.id.clone());
    for set in sweep(&core.room_dir(room), &dropped(logs), recording.as_deref()) {
        data::forget(&core.files, &core.recording_dir(room, &set));
    }
}

pub fn sweep_removed(core: &Core) {
    let removed = core.removed_rooms.lock().unwrap().clone();
    for id in removed.iter().filter(|id| valid_id(id) && !core.rooms.lock().unwrap().list.iter().any(|r| &r.id == *id)) {
        if core.room_dir(id).exists() {
            data::remove_room(core, id);
        }
    }
}

struct Holder<'a> {
    id: &'a str,
    have: &'a HashMap<String, u64>,
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

fn last_end(f: &File, series: &str) -> Option<i64> {
    f.segs.iter().filter(|s| s.series == series).map(|s| s.t_us + s.d_us - f.offset_us).max()
}

fn parts_json(room: &str, set: &Set, f: &File, size_of: &dyn Fn(&str, &str) -> (u64, bool)) -> Vec<Value> {
    let mut out = Vec::new();
    for capture in f.captures() {
        let Some(start_us) = f.caps.get(capture).and_then(|cap| cap["start_us"].as_i64()) else { continue };
        let mut part = json!({"capture": capture});
        for series in SERIES.iter().filter(|series| f.segs.iter().any(|s| s.capture == capture && s.series == **series)) {
            let (size, open) = size_of(capture, series);
            part[series] = json!({"path": format!("set/{room}/{}/{}/{series}/{capture}", set.id, f.owner), "start_us": start_us, "size": size, "open": open});
        }
        out.push(part);
    }
    out
}

pub fn others_have(core: &Core) -> Vec<(String, HashMap<String, u64>, bool)> {
    core.peers.lock().unwrap().iter().map(|p| (p.id.to_string(), p.have.clone(), p.conn.is_some())).collect()
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
    let online: Vec<&HashMap<String, u64>> = peers.iter().filter(|p| p.2).map(|p| &p.1).collect();
    let reports: Vec<&HashMap<String, u64>> = peers.iter().map(|p| &p.1).collect();
    let all = sets(core, room, logs);
    let mine = data::have_counts(core, room, &all);
    let mut analysis = crate::jobs::analysis_by_set(core, room, logs, &all, &mine);
    let mut out = Vec::new();
    for set in all {
        let mut holders = vec![Holder { id: &me, have: &mine }];
        holders.extend(peers.iter().map(|(id, have, _)| Holder { id, have }));
        let mut players = Vec::new();
        for f in &set.files {
            let (have, total) = data::held(core, room, f);
            let by: Vec<&str> = holders.iter().filter(|h| full(h.have, &set, f)).map(|h| h.id).collect();
            players.push(json!({
                "owner": f.owner, "imported": f.view.is_some(), "start_us": f.start_us, "end_us": f.end_us, "offset_us": f.offset_us, "have": have, "total": total,
                "recording": f.recording(), "verified_by": by, "stuck": stuck(&mine, &online, &set, f), "parts": parts_json(room, &set, f, &|capture, series| { let pieces = stream::set_pieces(core, room, f, capture, series); (stream::size(&pieces), stream::open(&pieces)) }),
                "last_us": {"v": last_end(f, SERIES[0]), "m": last_end(f, SERIES[1])},
            }));
        }
        let (have, total, stuck_all) = progress(core, &set, &mine, &online);
        let covered: Vec<&Value> = events.iter().filter(|e| covers(&set, e)).map(|e| &e["key"]).collect();
        let imported = set.files.iter().find_map(|f| f.view.as_ref());
        out.push(json!({
            "id": set.id, "room": room, "start_us": set.start(), "end_us": set.end_shown(), "live": set.recording(),
            "thumb": crate::thumbs::path(&core.room_dir(room), &set.id).exists(), "title": imported.and_then(|v| v.title.clone()), "imported_by": imported.map(|v| display_name(core, &v.by)),
            "players": players, "events": covered, "progress": {"have": have, "total": total, "stuck": stuck_all}, "wanted": wants(core, room, &set.id) == Some(true), "shared": others_hold(&set.id, &reports),
            "layout": set_layout(logs, &set.id), "of": debrief_of(&set.id, logs), "debriefs": named_debriefs(logs, &set.id), "analysis": analysis.remove(&set.id).unwrap_or_default(),
        }));
    }
    out
}

pub fn path_kind(conn: &Connection) -> (&'static str, u64, Option<String>) {
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

fn peer_json(p: &Peer) -> Value {
    let alive = p.alive();
    let kind = alive.map(path_kind);
    let (path, rtt, relay) = (kind.as_ref().map(|k| k.0), kind.as_ref().map(|k| k.1), kind.and_then(|k| k.2));
    let stats = alive.map(|c| c.stats());
    let other_version = p.info.as_ref().is_some_and(|(_, proto)| *proto != PROTOCOL);
    let sharing = p.live.is_some() && alive.is_some();
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
        "seen_us": p.seen_us,
        "offset_us": p.offset_us,
        "hw": p.hw,
        "version": p.info.as_ref().map(|(version, _)| version),
        "other_version": other_version,
        "sharing": sharing,
        "speaking": p.speaking && alive.is_some(),
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
    let mut peers: Vec<Value> = core
        .peers
        .lock()
        .unwrap()
        .iter()
        .map(|p| {
            if !p.name.is_empty() {
                names.insert(p.id.to_string(), p.name.clone());
            }
            peer_json(p)
        })
        .collect();
    for p in peers.iter_mut().filter(|p| p["sharing"] == true) {
        p["live"] = json!(stream::live_json(core, &room_id, p["id"].as_str().unwrap_or_default()));
    }
    names.insert(my_id.clone(), "You".to_string());
    names.extend(imported_views(core, &room_id).into_iter().map(|(id, v)| (id, v.name)));
    let logs = logs(core, &room_id);
    if let Some(name) = latest_name(&logs) {
        keep_name(core, &room_id, &name);
    }
    let active = core.active_room();
    let caps = core.party.lock().unwrap().caps();
    let mut events = events(&logs, caps.as_ref());
    let sets = sets_json(core, &room_id, &logs, &events);
    let recorded: Vec<(u64, i64)> = sets.iter().filter_map(|s| Some((s["progress"]["total"].as_u64()?, s["end_us"].as_i64()? - s["start_us"].as_i64()?))).collect();
    let disk = data::disk_json(core.free_bytes.load(Ordering::Relaxed), core.total_bytes.load(Ordering::Relaxed), data::bytes_per_hour(&recorded), core.disk_full());
    let members: Vec<String> = active.as_ref().map(|r| r.members.clone()).unwrap_or_default();
    let board: Vec<Value> = board_cards(&events, &members).into_iter().filter(|c| c["gone"] != true).collect();
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
        "app": crate::shell::app_name(&core.app, &crate::language(core)),
        "version": VERSION,
        "hw": hw::share(core),
        "now_us": now_us(),
        "clock": crate::clock::status(),
        "ffmpeg": core.ffmpeg_status.lock().unwrap().clone(),
        "ffmpeg_progress": crate::download::progress(ffmpeg::DOWNLOAD_KEY),
        "id": my_id,
        "name": core.name.lock().unwrap().clone(),
        "lang": crate::language(core),
        "language": *core.language.lock().unwrap(),
        "colors": colors(&people),
        "palette": PALETTE,
        "invites": core.invites.lock().unwrap().len(),
        "limit_mbps": *core.limit_mbps.lock().unwrap(),
        "recovery": core.recovery.lock().unwrap().clone(),
        "notice": core.notice.lock().unwrap().clone(),
        "disk": disk,
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
        "api": {"on": *core.api_on.lock().unwrap(), "lan": *core.api_lan.lock().unwrap(), "address": crate::api::address(), "error": core.api_error.lock().unwrap().clone()},
        "sets": sets,
        "peers": peers,
        "events": events,
        "tag_names": crate::jobs::tag_names(&logs, caps.as_ref()),
        "board": board,
        "names": names,
        "asks": asks,
        "party": party::state_json(core, &holdings(&logs)),
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::log::tests::{present, seed, temp};

    fn log(owner: &str, offset_us: i64, records: Vec<Value>) -> Log {
        Log { owner: owner.into(), offset_us, records: Arc::new(records) }
    }

    pub fn small_file(segs: i64, ended: bool) -> File {
        let mut records = vec![json!({"k": "cap", "capture": "1000", "start_us": 0, "init": {"v": {"size": 4, "sha256": "i"}}}), json!({"k": "rec", "set": "s1", "start_us": 0})];
        records.extend((1..=segs).map(|n| json!({"k": "seg", "capture": "1000", "series": "v", "n": n, "set": "s1", "t_us": n * 4, "d_us": 4, "size": 4, "sha256": format!("s{n}")})));
        records.extend(ended.then(|| json!({"k": "rec_end", "set": "s1", "t_us": 100})));
        derive_sets("r", &[log("a", 0, records)]).remove(0).files.remove(0)
    }

    #[test]
    fn a_part_starts_when_its_capture_began_and_a_capture_without_a_start_has_none() {
        let cap = |capture: &str, start: Option<i64>| json!({"k": "cap", "capture": capture, "start_us": start, "init": {"v": {"size": 4, "sha256": "i"}}});
        let seg = |capture: &str, t_us: i64| json!({"k": "seg", "capture": capture, "series": "v", "n": 1, "set": "s1", "t_us": t_us, "d_us": 4, "size": 4, "sha256": "s"});
        let records = vec![json!({"k": "rec", "set": "s1", "start_us": 0}), cap("1000", Some(700)), seg("1000", 800), cap("2000", None), seg("2000", 900)];
        let set = derive_sets("r", &[log("a", 0, records)]).remove(0);
        let parts = parts_json("r", &set, &set.files[0], &|capture, series| (capture.parse::<u64>().unwrap() + series.len() as u64, true));
        assert_eq!(parts, [json!({"capture": "1000", "v": {"path": "set/r/s1/a/v/1000", "start_us": 700, "size": 1001, "open": true}})]);
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
        let sets = derive_sets("r", &[a, b]);
        assert_eq!(sets.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s2", "s1"]);
        let s1 = &sets[1];
        assert_eq!(s1.files.len(), 2);
        assert_eq!(s1.files[0].end_us, Some(900));
        assert_eq!(s1.start(), 100);
        assert!(sets[0].files[0].end_us.is_none());
        let rejoined = derive_sets("r", &[log("a", 0, vec![
            json!({"k": "rec", "set": "s1", "start_us": 100}),
            json!({"k": "rec_end", "set": "s1", "t_us": 400}),
            json!({"k": "rec", "set": "s1", "start_us": 600}),
        ])]);
        assert_eq!((rejoined[0].files.len(), rejoined[0].files[0].start_us, rejoined[0].files[0].end_us), (1, 100, None));
        let dropped_by_b = [log("a", 0, vec![json!({"k": "rec", "set": "s2"})]), log("b", 0, vec![json!({"k": "set_del", "set": "s2"})])];
        assert_eq!(derive_sets("r", &dropped_by_b).len(), 0);
        assert_eq!(dropped(&dropped_by_b), HashSet::from(["s2".to_string()]));
    }

    #[test]
    fn records_from_other_versions_never_break_reading() {
        let valid = vec![
            json!({"k": "rec", "set": "s1", "start_us": 5, "room": "r", "by": "a", "voice": false}),
            json!({"k": "rec", "set": "d1", "start_us": 6, "room": "r", "by": "a", "voice": true, "of": "s1"}),
            json!({"k": "cap", "capture": "1000", "start_us": 0, "width": 1280, "init": {"v": {"size": 4, "sha256": "i"}}}),
            seg("1000", "v", 1, "s1"),
            json!({"k": "rec_end", "set": "s1", "t_us": 9, "stop": true}),
            user_event(1, "note", "hi #a", json!({"set": "s1", "t_us": 6}), json!({})),
            json!({"k": "event_edit", "id": 1, "t_us": 7, "text": "edited"}),
            json!({"k": "event_del", "id": 2, "t_us": 7}),
            json!({"k": "event_restore", "id": 2, "t_us": 8}),
            json!({"k": "set_del", "set": "gone"}),
            json!({"k": "remove", "member": "m1", "t_us": 9}),
            json!({"k": "admit", "member": "m2", "t_us": 9}),
            json!({"k": "room_name", "name": "Squad", "t_us": 9}),
            json!({"k": "offset", "peer": "p", "t_us": 9, "rtt_us": 1, "offset_us": 2}),
            json!({"k": "layout", "set": "s1", "t_us": 9, "layout": {"order": ["a"]}}),
            json!({"k": "plugin", "plugin": "p1", "on": true, "version": "1", "settings": {"model": "base"}, "t_us": 9}),
            json!({"k": "job", "job": "p1@1/ab/s1/a", "state": "done", "run": 1, "plugin": "p1", "version": "1", "file": "s1/a", "tags": {}, "t_us": 9}),
            json!({"k": "ai_del", "set": "s1", "t_us": 9}),
            json!({"k": "party", "set": "s1", "t_us": 9, "shared": {"set": "s1", "playing": false}}),
            json!({"k": "file_bad", "room": "r", "set": "s1", "file": "a/1000/000001.m4s", "size": 4, "t_us": 9}),
        ];
        let wrong = [json!("x"), Value::Null, json!([1]), json!(-1), json!(5_000_000_000i64), json!({})];
        let mut records = valid.clone();
        for r in &valid {
            for field in r.as_object().unwrap().keys().filter(|f| *f != "k") {
                for bad in &wrong {
                    let mut changed = r.clone();
                    changed[field.as_str()] = bad.clone();
                    records.push(changed);
                }
                let mut missing = r.clone();
                missing.as_object_mut().unwrap().remove(field);
                records.push(missing);
            }
            let mut extra = r.clone();
            extra["from_the_future"] = json!({"new": [1, 2]});
            records.push(extra);
        }
        records.extend([json!({"k": "from_the_future", "n": 1}), json!({"k": "file", "set": "s1", "size": 15}), json!({"k": "note", "text": "hi"}), json!({"k": 7}), json!("text"), Value::Null, json!([1])]);
        let logs = [log("a", 0, valid), log("b", 3, records)];
        assert_eq!(derive_sets("r", &logs).iter().find(|s| s.id == "s1").map(|s| s.files.len()), Some(2));
        assert!(events(&logs, None).iter().any(|e| e["text"] == "edited"));
        assert!(removals(&logs).contains_key("m1"));
        let _ = set_layout(&logs, "s1");
        assert!(latest_name(&logs).is_some());
        assert!(crate::party::records(&logs).contains_key("s1"));
        assert!(crate::jobs::configs(&logs, None).contains_key("p1") && crate::jobs::tracks(&logs, None).contains_key(&("p1".to_string(), "s1/a".to_string())));
        assert!(crate::jobs::chosen(&logs, None).contains_key("s1"));
        assert!(dropped(&logs).contains("gone"));
    }

    #[test]
    fn an_imported_view_carries_its_name_importer_and_title_in_its_first_rec() {
        let (importer, view) = (iroh::SecretKey::generate().public().to_string(), iroh::SecretKey::generate().public().to_string());
        let imported = |by: &str, name: &str, title: Value| vec![json!({"k": "rec", "set": "s1", "start_us": 5, "imported": by, "view": name, "title": title}), json!({"k": "rec_end", "set": "s1", "t_us": 9})];
        let sets = derive_sets("r", &[log("a", 0, vec![json!({"k": "rec", "set": "s2", "start_us": 1})]), log(&view, 0, imported(&importer, "  Walk\u{7}through ", json!("What went wrong")))]);
        let found = sets.iter().find(|s| s.id == "s1").unwrap().files[0].view.clone();
        assert_eq!(found, Some(ViewInfo { name: "Walkthrough".into(), by: importer.clone(), title: Some("What went wrong".into()) }));
        assert_eq!(sets.iter().find(|s| s.id == "s2").unwrap().files[0].view, None);
        assert_eq!(view_info(&imported(&importer, "Coach", Value::Null)).map(|v| v.title), Some(None));
        assert_eq!(view_info(&imported("not an id", "Coach", Value::Null)), None);
        assert_eq!(view_info(&imported(&importer, "  ", Value::Null)), None);
        assert_eq!(view_info(&[json!({"k": "seg"}), json!({"k": "rec", "set": "s1", "view": "Coach"})]), None);
        assert_eq!(view_info(&[]), None);
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
            json!({"k": "cap", "capture": "222", "init": {"v": {"size": 5, "sha256": "k"}}}),
            json!({"k": "rec_end", "set": "s1", "t_us": 20_000_000}),
        ]);
        let sets = derive_sets("r", &[a]);
        let f = &sets[0].files[0];
        assert_eq!(f.segs.iter().map(|s| (s.capture.as_str(), s.series.as_str(), s.n)).collect::<Vec<_>>(), [("111", "v", 2), ("111", "m", 2), ("222", "v", 1)]);
        assert_eq!(f.captures(), ["111", "222"]);
        assert_eq!(f.init("111", "m"), Some((3, "j".into())));
        assert_eq!((f.init("222", "v"), f.init("222", "m")), (Some((5, "k".into())), None));
        assert_eq!(f.total(), 300 + 7 + 3 + 5);
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
    fn a_recording_is_removed_completely_only_when_no_other_member_holds_a_part() {
        let report = |keys: &[(&str, u64)]| -> HashMap<String, u64> { keys.iter().map(|(k, n)| (k.to_string(), *n)).collect() };
        let nothing = report(&[]);
        let other_set = report(&[("s10/ab/v", 3)]);
        let zero = report(&[("s1/ab/v", 0)]);
        let holds = report(&[("s1/ab/m", 2)]);
        assert!(!others_hold("s1", &[]));
        assert!(!others_hold("s1", &[&nothing, &other_set, &zero]));
        assert!(others_hold("s1", &[&nothing, &holds]));
        assert!(others_hold("s10", &[&other_set]));
    }

    #[test]
    fn deleting_a_recording_removes_only_its_folder_and_thumbnail() {
        let dir = temp("sweep");
        let names = ["recordings/s1/a/1/v/000001.m4s", "recordings/s1/b/2/m/init.mp4", "thumbs/s1.jpg", "recordings/s2/a/1/v/000001.m4s", "thumbs/s2.jpg", "recordings/s3/a/1/v/000001.m4s", "standby/a/1/v/000002.m4s", "a/1/v/000001.m4s", "sets/s1/a.mp4", "logs/a.bin"];
        seed(&dir, names);
        remove_recording(&dir, "s1");
        assert_eq!(present(&dir, &names), &names[3..]);
        let gone: HashSet<String> = ["s2", "s9", "../x", " s3"].map(String::from).into();
        assert!(sweep(&dir, &gone, Some("s2")).is_empty());
        assert_eq!(present(&dir, &names), &names[3..]);
        assert_eq!(sweep(&dir, &gone, None), ["s2"]);
        assert!(sweep(&dir, &gone, None).is_empty());
        seed(&dir, ["recordings/s2/b/2/v/000001.m4s"]);
        assert_eq!(sweep(&dir, &gone, None), ["s2"]);
        assert_eq!(present(&dir, &names), &names[5..]);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_new_recording_never_takes_an_id_in_use() {
        let dir = temp("unused");
        let start = 1_760_000_000_000_000;
        let id = |i: i64| set_id_for(start + i * 1_000_000, 0);
        let logs = [log("a", 0, vec![json!({"k": "rec", "set": id(0)})]), log("b", 0, vec![json!({"k": "set_del", "set": id(1)})])];
        std::fs::create_dir_all(recording_dir(&dir, &id(2))).unwrap();
        assert_eq!((unused_set(&logs, &dir, start, 0), unused_set(&[], &dir, start, 0), unused_set(&[], &dir, start + 2_000_000, 0)), (id(3), id(0), id(3)));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn only_a_recording_in_progress_or_a_requested_one_is_received() {
        assert!(auto_receive(true, false, None));
        assert!(auto_receive(false, true, None));
        assert!(!auto_receive(false, false, None));
        assert!(auto_receive(false, false, Some(true)));
        assert!(!auto_receive(true, true, Some(false)));
        assert!(!auto_receive(false, true, Some(false)));
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
    fn a_recording_removed_completely_takes_its_results_and_jobs_with_it() {
        let rec = |set: &str| json!({"k": "rec", "set": set, "start_us": 1});
        let owner = log("a", 0, vec![
            rec("s1"),
            rec("s2"),
            json!({"k": "event", "id": 1, "kind": "note", "target": {"set": "s1", "t_us": 5}, "text": "in s1"}),
            json!({"k": "event", "id": 2, "kind": "note", "target": {"set": "s2", "t_us": 6}, "text": "in s2"}),
            json!({"k": "event", "id": 3, "kind": "note", "target": {"t_us": 7}, "text": "in the room"}),
        ]);
        let texts = |logs: &[Log]| -> Vec<String> { events(logs, None).iter().map(|e| e["text"].as_str().unwrap().to_string()).collect() };
        assert_eq!(texts(std::slice::from_ref(&owner)), ["in s1", "in s2", "in the room"]);
        let remover = log("b", 0, vec![json!({"k": "set_del", "set": "s1"})]);
        let both = [owner, remover];
        assert_eq!(texts(&both), ["in s2", "in the room"]);
        assert_eq!(derive_sets("r", &both).iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["s2"]);
    }

    #[test]
    fn coverage_by_moment_or_set() {
        let set = Set { id: "s1".into(), room: "r".into(), files: vec![File { set: "s1".into(), owner: "a".into(), start_us: 100, end_us: Some(200), offset_us: 0, segs: Vec::new(), caps: HashMap::new(), view: None, present: false }], stopped: false };
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
        assert!(!valid_id(" ") && !valid_id("s1 ") && !valid_id(" s1"));
        assert!(valid_id("1a2b3c4d"));
        assert!(valid_id(&set_id_for(1_760_000_000_000_000, 0)));
    }

    #[test]
    fn a_peer_can_only_name_a_room_with_an_id_this_app_makes() {
        assert!(hex_of(&random_hex(ROOM_ID_BYTES), ROOM_ID_BYTES));
        for id in ["A1B2C3D4", "a1b2c3d ", "pending-0123456789abcdef", "a1b2c3d4e5", "a1b2"] {
            assert!(!hex_of(id, ROOM_ID_BYTES), "{id}");
        }
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
        let b = log("b", 0, vec![json!({"k": "rec", "set": "s2"}), b.records[0].clone()]);
        assert_eq!(derive_sets("r", &[a, b]).iter().map(|s| (s.id.as_str(), s.stopped)).collect::<Vec<_>>(), [("s2", true), ("s1", false)]);
    }

    #[test]
    fn a_recording_ends_for_the_list_once_nobody_present_records_it() {
        let seg = |set: &str, n: i64| json!({"k": "seg", "capture": "1000", "series": "m", "n": n, "set": set, "t_us": n * 4_000_000, "d_us": 4_000_000, "size": 4, "sha256": "x"});
        let rec = |set: &str| json!({"k": "rec", "set": set, "start_us": 0});
        let end = |set: &str, t_us: i64, stop: bool| json!({"k": "rec_end", "set": set, "t_us": t_us, "stop": stop});
        let a = log("a", 0, vec![rec("s1"), seg("s1", 1), end("s1", 20_000_000, true), rec("s2"), seg("s2", 1), end("s2", 9_000_000, false), rec("s3"), end("s3", 5_000_000, false)]);
        let c = log("c", 1_000_000, vec![rec("s1"), seg("s1", 6), rec("s2"), seg("s2", 3), rec("s3"), end("s3", 7_000_000, true)]);
        let shown = |present: &'static [&str]| {
            with_presence(derive_sets("r", &[log("a", 0, a.records.to_vec()), log("c", 1_000_000, c.records.to_vec())]), |o| present.contains(&o))
                .into_iter()
                .map(|s| (s.id.clone(), s.stopped, s.recording(), s.end_shown()))
                .collect::<Vec<_>>()
        };
        let s = |id: &str, stopped: bool, recording: bool, end: Option<i64>| (id.to_string(), stopped, recording, end);
        assert_eq!(shown(&["a", "c"]), [s("s3", true, false, Some(6_000_000)), s("s2", false, true, None), s("s1", true, false, Some(27_000_000))]);
        assert_eq!(shown(&["a"])[1], s("s2", false, false, Some(15_000_000)));
        let left = with_presence(derive_sets("r", &[a, c]), |o| o == "a").remove(1);
        assert!(!left.files[1].recording() && left.files[0].end_us.is_some());
        assert!(covers(&left, &json!({"local_us": 15_000_000, "set": null})));
        assert!(!covers(&left, &json!({"local_us": 16_000_000, "set": null})));
    }

    #[test]
    fn a_member_is_present_while_connected_or_seen_within_a_minute() {
        let (now, minute) = (1_000_000_000, UNREACHABLE_AFTER.as_micros() as i64);
        assert!(reachable(true, None, now, UNREACHABLE_AFTER * 2));
        assert!(reachable(false, Some(now - minute + 1), now, UNREACHABLE_AFTER * 2));
        assert!(!reachable(false, Some(now - minute), now, Duration::ZERO));
        assert!(reachable(false, None, now, UNREACHABLE_AFTER / 2));
        assert!(!reachable(false, None, now, UNREACHABLE_AFTER));
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

    fn started_by(set: &str, extra: Value) -> Value {
        let mut r = json!({"k": "rec", "set": set, "start_us": 1});
        crate::merge(&mut r, &extra);
        r
    }

    #[test]
    fn an_old_debrief_links_through_its_first_party_record() {
        let party = |t: i64, set: &str| json!({"k": "party", "set": "d1", "t_us": t, "shared": {"set": set, "at_us": 1}});
        let logs = vec![log("a", 0, vec![started_by("s1", json!({})), started_by("d1", json!({"voice": true})), party(20, "s9"), party(10, "s1")]), log("b", 0, vec![started_by("s9", json!({}))])];
        assert_eq!(debrief_of("d1", &logs), Some("s1".to_string()));
        assert_eq!(debrief_of("s1", &logs), None);
        let plain = vec![log("a", 0, vec![started_by("s1", json!({})), started_by("d1", json!({})), party(10, "s1")])];
        assert_eq!(debrief_of("d1", &plain), None);
        let named = vec![log("a", 0, vec![started_by("s1", json!({})), started_by("d2", json!({"voice": true, "of": "s1"}))])];
        assert_eq!(debrief_of("d2", &named), Some("s1".to_string()));
        let orphan = vec![log("a", 0, vec![started_by("d2", json!({"voice": true, "of": "s1"}))])];
        assert_eq!(debrief_of("d2", &orphan), None);
        let gone = vec![log("a", 0, vec![started_by("s1", json!({})), started_by("d2", json!({"voice": true, "of": "s1"}))]), log("b", 0, vec![json!({"k": "set_del", "set": "s1"})])];
        assert_eq!(debrief_of("d2", &gone), None);
        assert_eq!(debrief_of("s1", &named), None);
    }

    #[test]
    fn deleting_a_recording_deletes_only_debriefs_that_name_it_and_keeps_peers_copies() {
        let party = json!({"k": "party", "set": "old", "t_us": 1, "shared": {"set": "s1"}});
        let logs = vec![
            log("a", 0, vec![started_by("s1", json!({})), started_by("d1", json!({"voice": true, "of": "s1"})), started_by("old", json!({"voice": true})), party, started_by("d2", json!({"voice": true, "of": "s2"}))]),
            log("b", 0, vec![started_by("d1", json!({"voice": true, "of": "s1"})), started_by("d3", json!({"voice": true, "of": "s1"})), json!({"k": "set_del", "set": "d3"})]),
        ];
        assert_eq!(named_debriefs(&logs, "s1"), vec!["d1".to_string()]);
        assert_eq!(debrief_of("old", &logs), Some("s1".to_string()));
        let peers_hold = HashMap::from([("d1/abc/m".to_string(), 3u64)]);
        assert!(others_hold("d1", &[&peers_hold]) && !others_hold("old", &[&peers_hold]));
    }

    #[test]
    fn ai_del_hides_ai_notes_from_every_log_but_not_human_notes() {
        let note = |id: i64, set: &str, ai: bool| {
            let mut r = user_event(id, "note", "n", json!({"set": set}), Value::Null);
            if ai {
                r["source"] = json!("ai");
            }
            r
        };
        let logs = vec![
            log("a", 0, vec![note(100, "s1", true), note(101, "s1", false), note(102, "s2", true), note(900, "s1", true)]),
            log("b", 10, vec![note(150, "s1", true), json!({"k": "ai_del", "set": "s1", "t_us": 500})]),
        ];
        let shown: Vec<(String, bool)> = events(&logs, None).iter().map(|e| (e["key"].as_str().unwrap().to_string(), e["source"] == "ai")).collect();
        assert_eq!(shown.len(), 3);
        assert!(shown.contains(&("a/101".to_string(), false)) && shown.contains(&("a/102".to_string(), true)) && shown.contains(&("a/900".to_string(), true)));
        let later = vec![log("a", 0, vec![note(100, "s1", true)]), log("b", 500, vec![json!({"k": "ai_del", "set": "s1", "t_us": 650})])];
        assert!(events(&later, None).is_empty());
        let after = vec![log("a", 0, vec![note(100, "s1", true)]), log("b", 0, vec![json!({"k": "ai_del", "set": "s1", "t_us": 50})])];
        assert_eq!(events(&after, None).len(), 1);
    }

    #[test]
    fn board_cards_newest_event_wins_and_old_team_card_reads() {
        let card = |id: i64, author: &str, text: &str, values: Value| {
            let mut e = events(&[log(author, 0, vec![user_event(id, "board", text, json!({}), values)])], None).remove(0);
            e["at_us"] = json!(id);
            e
        };
        let members = vec!["a".to_string(), "b".to_string()];
        let none = board_cards(&[], &members);
        assert_eq!(none.iter().map(|c| c["card"].as_str().unwrap()).collect::<Vec<_>>(), ["team", "m:a", "m:b"]);
        let old = card(5, "a", "# Plan", Value::Null);
        let list = board_cards(&[old, card(7, "b", "Newer plan", json!({"card": "team"})), card(6, "a", "Older plan", json!({"card": ""}))], &members);
        assert_eq!((list[0]["card"].as_str(), list[0]["text"].as_str(), list[0]["by"].as_str()), (Some("team"), Some("Newer plan"), Some("b")));
        let more = board_cards(&[card(8, "a", "New", json!({"card": "c:1", "title": "Drops", "pos": 2.5})), card(9, "b", "Mine", json!({"card": "m:a", "pos": 3.0})), card(10, "b", "", json!({"card": "m:b", "gone": true})), card(11, "a", "Moved", json!({"card": "c:1", "title": "Drops", "pos": 0.5}))], &members);
        assert_eq!(more.iter().map(|c| c["card"].as_str().unwrap()).collect::<Vec<_>>(), ["team", "c:1", "m:b", "m:a"]);
        assert_eq!((more[1]["text"].as_str(), more[1]["title"].as_str(), more[1]["pos"].as_f64()), (Some("Moved"), Some("Drops"), Some(0.5)));
        assert_eq!(more[2]["gone"], true);
        assert!(board_card_valid("team") && board_card_valid("m:ab12") && board_card_valid("c:0f-9") && !board_card_valid("m:") && !board_card_valid("x:1") && !board_card_valid("c:a b"));
    }
}

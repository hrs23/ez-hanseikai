use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::EndpointId;
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::time::{sleep, timeout};

use crate::room::{self, part_of, seg_name, valid_id, INIT, LEGACY, SERIES};
use crate::{data, debug, default_codecs, file_len, log, merge, now_us, party, short, strings, Core, ALPN, PROTOCOL, VERSION};

const CHUNK: usize = 64 << 10;
const LOG_SYNC: Duration = Duration::from_secs(2);
const FILE_WATCH: Duration = Duration::from_millis(250);
const RETRY: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_MAX: usize = 1 << 20;
const RESPONSE_MAX: usize = 32 << 20;
const PULL_MAX: u64 = 8 << 20;
const JOIN_MAX: usize = 4096;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const MISMATCH_WAIT: Duration = Duration::from_secs(10);
const OFFSET_SAMPLES: usize = 8;
const HELLO_EVERY: u32 = 3;
const MEASURE_FIRST: u32 = 5;
const MEASURE_EVERY: u32 = 15;
const BYTES_PER_MBIT: f64 = 125_000.0;
const LOG_PRIORITY: i32 = 3;
const LIVE_VOICE_PRIORITY: i32 = 2;
const LIVE_PICTURE_PRIORITY: i32 = 1;
const HISTORY_VOICE_PRIORITY: i32 = -1;
const HISTORY_PICTURE_PRIORITY: i32 = -2;
const HISTORY_WAIT: Duration = Duration::from_millis(50);
const HISTORY_SHARE: f64 = 0.25;
const FRAME_HEADER: usize = 8;

async fn request(conn: &Connection, req: &Value) -> Result<Vec<u8>> {
    timeout(REQUEST_TIMEOUT, async {
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(&serde_json::to_vec(req)?).await?;
        send.finish()?;
        Ok(recv.read_to_end(RESPONSE_MAX).await?)
    })
    .await?
}

pub fn broadcast(core: &Arc<Core>, message: Value) {
    for conn in core.connections() {
        let message = message.clone();
        tauri::async_runtime::spawn(async move {
            let _ = request(&conn, &message).await;
        });
    }
}

fn room_info(core: &Core) -> Value {
    match core.active_room() {
        Some(mut r) => {
            r.names.insert(core.ep.id().to_string(), core.name.lock().unwrap().clone());
            json!({"id": r.id, "name": r.name, "members": r.members, "names": r.names})
        }
        None => Value::Null,
    }
}

fn learn_members(core: &Arc<Core>, room: &Value, members: &Value) {
    let Some(id) = room["id"].as_str() else { return };
    if core.active_room_id() != id {
        return;
    }
    for member in members.as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
        room::add_member(core, id, member);
    }
    for (member, name) in room["names"].as_object().into_iter().flatten() {
        if member.parse::<EndpointId>().is_ok_and(|m| m != core.ep.id()) {
            room::remember_name(core, member, &crate::clean_name(name.as_str().unwrap_or_default()), false);
        }
    }
}

fn codec_list(value: &Value) -> Vec<String> {
    let list: Vec<String> = value.as_array().into_iter().flatten().filter_map(|v| v.as_str()).filter(|c| ["h264", "hevc", "av1"].contains(c)).map(String::from).collect();
    if list.is_empty() { default_codecs() } else { list }
}

fn learn_hello(core: &Arc<Core>, id: &EndpointId, message: &Value) -> u32 {
    let version = message["app"].as_str().unwrap_or("unknown").to_string();
    let proto = message["proto"].as_u64().unwrap_or(0) as u32;
    let name = crate::clean_name(message["name"].as_str().unwrap_or_default());
    if core.update_peer(id, |p| std::mem::take(&mut p.announce)).unwrap_or(false) {
        let _ = core.app.emit("joined", if name.is_empty() { short(id) } else { name.clone() });
    }
    let codecs = codec_list(&message["codecs"]);
    core.update_peer(id, |p| {
        p.info = Some((version, proto));
        p.codecs = codecs;
    });
    room::remember_name(core, &id.to_string(), &name, true);
    learn_members(core, &message["room"], &message["members"]);
    learn_holdings(core, id, message);
    react_to_set(core, &message["set"]);
    if message["room"]["id"].as_str() == Some(core.active_room_id().as_str()) {
        let asks = message["asks"].as_array().cloned().unwrap_or_default();
        core.update_peer(id, |p| p.asks = asks);
    }
    proto
}

fn bytes_per_sec(core: &Core) -> Option<f64> {
    core.limit_mbps.lock().unwrap().map(|mbps| mbps * BYTES_PER_MBIT)
}

pub struct Pacer {
    next: Instant,
    history: Instant,
}

impl Pacer {
    pub fn new() -> Self {
        Self { next: Instant::now(), history: Instant::now() }
    }

    fn reserve(&mut self, now: Instant, bytes: usize, history: bool, rate: f64) -> Option<Duration> {
        if history {
            if self.next > now && self.history > now {
                return None;
            }
            self.history = self.history.max(now) + Duration::from_secs_f64(bytes as f64 / (rate * HISTORY_SHARE));
        }
        self.next = self.next.max(now) + Duration::from_secs_f64(bytes as f64 / rate);
        Some(self.next - now)
    }
}

async fn pace(core: &Core, bytes: usize, history: bool) {
    let Some(rate) = bytes_per_sec(core) else { return };
    loop {
        let due = core.pacer.lock().unwrap().reserve(Instant::now(), bytes, history, rate);
        match due {
            Some(due) => return sleep(due).await,
            None => sleep(HISTORY_WAIT).await,
        }
    }
}

fn holdings(core: &Core) -> Value {
    let room = core.active_room_id();
    let sets = room::sets(core, &room, &room::logs(core, &room));
    json!({"have": data::have_counts(core, &room, &sets), "gone": room::gone_list(core, &room)})
}

fn learn_holdings(core: &Core, id: &EndpointId, message: &Value) {
    if message["room"]["id"].as_str() != Some(core.active_room_id().as_str()) {
        return;
    }
    core.update_peer(id, |p| {
        p.have = message["have"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_u64()?))).collect();
        p.gone = strings(&message["gone"]);
    });
}

fn hello_message(core: &Core) -> Value {
    let room = room_info(core);
    let mut message = json!({"app": VERSION, "proto": PROTOCOL, "name": *core.name.lock().unwrap(), "room": room, "members": room["members"], "set": core.active_set_json(), "codecs": *core.codecs.lock().unwrap(), "asks": pending_asks(core)});
    merge(&mut message, &holdings(core));
    message
}

fn in_active_room(core: &Core, room: &str) -> bool {
    valid_id(room) && room == core.active_room_id()
}

fn priority(live: bool, series: &str) -> i32 {
    match (live, series == SERIES[1]) {
        (true, true) => LIVE_VOICE_PRIORITY,
        (true, false) => LIVE_PICTURE_PRIORITY,
        (false, true) => HISTORY_VOICE_PRIORITY,
        (false, false) => HISTORY_PICTURE_PRIORITY,
    }
}

async fn write_frame(send: &mut SendStream, n: u32, data: &[u8]) -> Result<()> {
    let mut head = [0u8; FRAME_HEADER];
    head[..4].copy_from_slice(&n.to_le_bytes());
    head[4..].copy_from_slice(&(data.len() as u32).to_le_bytes());
    send.write_all(&head).await?;
    send.write_all(data).await?;
    Ok(())
}

async fn send_file(core: &Core, send: &mut SendStream, n: u32, path: &Path, at: u64, history: bool, tail: &impl Fn() -> bool) -> Result<bool> {
    let part = part_of(path);
    let mut file = loop {
        if let Ok(file) = tokio::fs::File::open(path).await {
            break file;
        }
        if let Ok(file) = tokio::fs::File::open(&part).await {
            break file;
        }
        if !tail() {
            return Ok(false);
        }
        sleep(FILE_WATCH).await;
    };
    file.seek(std::io::SeekFrom::Start(at)).await?;
    let mut sent = at;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let k = file.read(&mut buf).await?;
        if k > 0 {
            write_frame(send, n, &buf[..k]).await?;
            sent += k as u64;
            pace(core, k, history).await;
            continue;
        }
        if path.exists() && file_len(path) == sent {
            break;
        }
        if !tail() || !(path.exists() || part.exists()) {
            return Ok(false);
        }
        sleep(FILE_WATCH).await;
    }
    write_frame(send, n, &[]).await?;
    Ok(true)
}

fn capture_id(text: &str) -> bool {
    !text.is_empty() && text.len() <= 20 && text.chars().all(|c| c.is_ascii_digit())
}

async fn follow(core: &Core, send: &mut SendStream, req: &Value) -> Result<()> {
    let text = |k: &str| req[k].as_str().unwrap_or_default().to_string();
    let (room, owner, capture, series, set) = (text("room"), text("owner"), text("capture"), text("series"), text("set"));
    let legacy = capture == LEGACY;
    let own = owner == core.me();
    let live = req["live"] == true;
    let valid = in_active_room(core, &room) && owner.parse::<EndpointId>().is_ok() && SERIES.contains(&series.as_str()) && if legacy { valid_id(&set) } else { capture_id(&capture) };
    if !valid || (own && !core.share.load(Ordering::Relaxed)) || (live && !own) {
        return Ok(());
    }
    let _ = send.set_priority(priority(live, &series));
    let sets = if own { Vec::new() } else { room::sets(core, &room, &room::logs(core, &room)) };
    let files: Vec<(&room::Set, &room::File)> = sets.iter().flat_map(|s| s.files.iter().filter(|f| f.owner == owner).map(move |f| (s, f))).collect();
    let verified = |n: u32| files.iter().any(|(s, f)| (!legacy || s.id == set) && f.segs.iter().any(|g| g.capture == capture && g.series == series && g.n == n && data::seg_check(core, &room, &s.id, &f.owner, g) == Some(true)));
    let dir = core.capture_dir(&room, &owner, &capture).join(&series);
    let tail = || own && core.share.load(Ordering::Relaxed) && core.engine.live().is_some_and(|(r, c, _)| r == room && c == capture);
    let history = !live;
    if req["init"] == true && !legacy {
        let ok = own || files.iter().any(|(_, f)| f.segs.iter().any(|g| g.capture == capture) && data::init_check(core, &room, f, &capture, &series) == Some(true));
        if !ok || !send_file(core, send, 0, &dir.join(INIT), 0, history, &|| false).await? {
            return Ok(());
        }
    }
    let to = req["to"].as_u64().map(|t| t as u32);
    let (mut n, mut at) = (req["from"].as_u64().unwrap_or(1) as u32, req["at"].as_u64().unwrap_or(0));
    while to.is_none_or(|to| n <= to) && !(legacy && n > 1) {
        let path = if legacy { core.set_file(&room, &set, &owner) } else { dir.join(seg_name(n)) };
        if !(own || verified(n)) || !send_file(core, send, n, &path, at, history, &tail).await? {
            break;
        }
        n += 1;
        at = 0;
    }
    Ok(())
}

fn react_to_set(core: &Arc<Core>, set: &Value) {
    let (Some(id), Some(room)) = (set["id"].as_str(), set["room"].as_str()) else { return };
    if !valid_id(id) || room != core.active_room_id() || core.is_deleted(room, id) {
        return;
    }
    if core.own_log(room).iter().any(|r| (r["k"] == "rec" || r["k"] == "rec_end") && r["set"] == id) {
        return;
    }
    let (by, start_us) = (set["by"].as_str().unwrap_or_default().to_string(), set["start_us"].as_i64().unwrap_or(0));
    let newer = match core.active_set.lock().unwrap().as_ref() {
        None => false,
        Some(a) if (a.start_us, a.id.as_str()) < (start_us, id) => true,
        Some(_) => return,
    };
    if !newer {
        let _ = room::begin_set(core, id, &by, start_us);
        return;
    }
    let (core, id) = (core.clone(), id.to_string());
    std::thread::spawn(move || {
        room::end_set(&core);
        let _ = room::begin_set(&core, &id, &by, start_us);
    });
}

async fn handle(core: Arc<Core>, from: EndpointId, mut send: SendStream, mut recv: RecvStream) -> Result<()> {
    let req: Value = serde_json::from_slice(&recv.read_to_end(REQUEST_MAX).await?)?;
    match req["t"].as_str() {
        Some("pull") => {
            let (room, of) = (req["room"].as_str().unwrap_or_default(), req["of"].as_str().unwrap_or_default());
            if room::may_share(&core, room, &from.to_string(), of) {
                let _ = send.set_priority(LOG_PRIORITY);
                if let Ok(mut file) = tokio::fs::File::open(core.log_path(room, of)).await {
                    file.seek(std::io::SeekFrom::Start(req["from"].as_u64().unwrap_or(0))).await?;
                    let mut data = Vec::new();
                    file.take(PULL_MAX).read_to_end(&mut data).await?;
                    send.write_all(&data).await?;
                }
            }
        }
        Some("follow") => follow(&core, &mut send, &req).await?,
        Some("hello") => {
            learn_hello(&core, &from, &req);
            send.write_all(&serde_json::to_vec(&hello_message(&core))?).await?;
        }
        Some("rec") => {
            if req["action"] == "start" {
                react_to_set(&core, &json!({"id": req["set"], "room": req["room"], "by": req["by"], "start_us": req["start_us"]}));
            } else if req["action"] == "stop" && core.is_recording(req["room"].as_str().unwrap_or_default(), req["set"].as_str().unwrap_or_default()) {
                let core = core.clone();
                tokio::task::spawn_blocking(move || room::end_set(&core));
            }
            send.write_all(b"ok").await?;
        }
        Some("live") => {
            let sharing = core.sharing_live();
            let live = core.engine.live().filter(|(room, _, _)| sharing && *room == core.active_room_id());
            let reply = match live {
                Some((_, capture, n)) => json!({"sharing": true, "capture": capture, "n": n}),
                None => json!({"sharing": false}),
            };
            send.write_all(&serde_json::to_vec(&reply)?).await?;
        }
        Some("ping") => send.write_all(&now_us().to_le_bytes()).await?,
        Some("party") => {
            let offset = core.peers.lock().unwrap().iter().find(|p| p.id == from).and_then(|p| p.offset_us).unwrap_or(0);
            if party::receive(&core, &from.to_string(), &req, offset) {
                let _ = core.app.emit("party", from.to_string());
            }
            send.write_all(b"ok").await?;
        }
        Some("asks") if req["room"].as_str() == Some(core.active_room_id().as_str()) => {
            let asks = req["asks"].as_array().cloned().unwrap_or_default();
            core.update_peer(&from, |p| p.asks = asks);
            send.write_all(b"ok").await?;
        }
        Some("answer") => {
            if room::answer_ask(&mut core.asks.lock().unwrap(), req["id"].as_str().unwrap_or_default(), req["approve"] == true) {
                broadcast(&core, asks_message(&core));
            }
            send.write_all(b"ok").await?;
        }
        Some("remove") if req["room"].as_str() == Some(core.active_room_id().as_str()) => {
            core.wake.notify_waiters();
            send.write_all(b"ok").await?;
        }
        Some("join") => {
            let room = room_info(&core);
            send.write_all(&serde_json::to_vec(&json!({"ok": true, "room": room, "members": room["members"]}))?).await?;
        }
        _ => {}
    }
    send.finish()?;
    Ok(())
}

pub fn pending_asks(core: &Core) -> Vec<Value> {
    core.asks.lock().unwrap().iter().filter(|a| a.answer.is_none()).map(|a| json!({"id": a.id, "name": a.name})).collect()
}

pub fn asks_message(core: &Core) -> Value {
    json!({"t": "asks", "room": core.active_room_id(), "asks": pending_asks(core)})
}

pub fn answer(core: &Arc<Core>, id: &str, approve: bool) {
    let mine = room::answer_ask(&mut core.asks.lock().unwrap(), id, approve);
    if mine {
        broadcast(core, asks_message(core));
    } else {
        broadcast(core, json!({"t": "answer", "id": id, "approve": approve}));
    }
}

async fn join(core: &Arc<Core>, from: EndpointId, mut send: SendStream, mut recv: RecvStream) -> Result<()> {
    let req: Value = serde_json::from_slice(&recv.read_to_end(JOIN_MAX).await?)?;
    let room_id = core.active_room_id();
    let (token, id, name) = (req["token"].as_str().unwrap_or_default(), from.to_string(), crate::clean_name(req["name"].as_str().unwrap_or_default()));
    if req["t"] != "join" {
        send.finish()?;
        return Ok(());
    }
    let step = {
        let mut invites = core.invites.lock().unwrap();
        let mut asks = core.asks.lock().unwrap();
        room::ask_step(&mut invites, &mut asks, &room_id, token, &id, &name)
    };
    let reply = match step {
        room::Step::Refused => json!({"ok": false}),
        room::Step::Wait(changed) => {
            if changed {
                broadcast(core, asks_message(core));
            }
            json!({"wait": true})
        }
        room::Step::Declined => {
            broadcast(core, asks_message(core));
            json!({"declined": true})
        }
        room::Step::Admitted => {
            broadcast(core, asks_message(core));
            room::admit(core, &room_id, &id);
            let room = room_info(core);
            json!({"ok": true, "room": room, "members": room["members"]})
        }
    };
    send.write_all(&serde_json::to_vec(&reply)?).await?;
    send.finish()?;
    send.stopped().await.ok();
    Ok(())
}

fn watch_paths(core: &Arc<Core>, conn: &Connection, label: String) {
    let core = core.clone();
    let mut events = conn.path_events();
    let started = Instant::now();
    tokio::spawn(async move {
        use tokio_stream::StreamExt;
        let mut direct = false;
        while let Some(event) = events.next().await {
            if let iroh::endpoint::PathEvent::Selected { remote_addr, .. } = &event {
                if remote_addr.is_ip() && !direct {
                    direct = true;
                    debug::event(&core, started, format!("{label}: switched to a direct path"));
                }
            }
            debug::event(&core, started, format!("{label}: {event:?}"));
        }
    });
}

pub async fn accept_loop(core: Arc<Core>) {
    while let Some(incoming) = core.ep.accept().await {
        let core = core.clone();
        tokio::spawn(async move {
            let Ok(accepting) = incoming.accept() else { return };
            let Ok(conn) = accepting.await else { return };
            let from = conn.remote_id();
            if !core.has_peer(&from) {
                if let Ok((send, recv)) = conn.accept_bi().await {
                    let _ = join(&core, from, send, recv).await;
                }
                conn.close(1u32.into(), b"not a member");
                return;
            }
            core.update_peer(&from, |p| p.incoming = Some(conn.clone()));
            watch_paths(&core, &conn, format!("{} incoming", short(&from)));
            while let Ok((send, recv)) = conn.accept_bi().await {
                tokio::spawn(handle(core.clone(), from, send, recv));
            }
        });
    }
}

async fn pull_log(core: &Core, room: &str, of: &str, conn: &Connection) -> Result<()> {
    let path = core.log_path(room, of);
    let from = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
    let data = request(conn, &json!({"t": "pull", "room": room, "of": of, "from": from})).await?;
    if !data.is_empty() {
        let _guard = core.log_lock.lock().unwrap();
        log::extend(&path, from, &data)?;
    }
    if of == core.me() {
        if file_len(&path) == 0 {
            core.recovered.lock().unwrap().insert(room.to_string());
        }
        core.settle(room);
    }
    Ok(())
}

fn target(core: &Core, room: &str, set: &str, owner: &str, capture: &str, series: &str, n: u32) -> PathBuf {
    if capture == LEGACY {
        return core.set_file(room, set, owner);
    }
    let dir = core.capture_dir(room, owner, capture).join(series);
    if n == 0 { dir.join(INIT) } else { dir.join(seg_name(n)) }
}

struct Writing<'a> {
    core: &'a Core,
    path: PathBuf,
}

impl Drop for Writing<'_> {
    fn drop(&mut self) {
        self.core.writing.lock().unwrap().remove(&self.path);
    }
}

async fn receive(core: &Core, conn: &Connection, req: Value) -> Result<u64> {
    let text = |k: &str| req[k].as_str().unwrap_or_default().to_string();
    let (room, set, owner, capture, series) = (text("room"), text("set"), text("owner"), text("capture"), text("series"));
    let (from, at) = (req["from"].as_u64().unwrap_or(1) as u32, req["at"].as_u64().unwrap_or(0));
    let legacy = capture == LEGACY;
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&serde_json::to_vec(&req)?).await?;
    send.finish()?;
    let mut current: Option<(u32, tokio::fs::File, Writing)> = None;
    let mut resumed = false;
    let mut got = 0u64;
    let mut head = [0u8; FRAME_HEADER];
    let mut buf = Vec::new();
    while recv.read_exact(&mut head).await.is_ok() {
        let n = u32::from_le_bytes(head[..4].try_into()?);
        let len = u32::from_le_bytes(head[4..].try_into()?) as usize;
        if len > CHUNK || core.disk_full() || (!set.is_empty() && core.is_deleted(&room, &set)) {
            break;
        }
        if current.as_ref().is_none_or(|(k, _, _)| *k != n) {
            let path = target(core, &room, &set, &owner, &capture, &series, n);
            if let Some(dir) = path.parent() {
                tokio::fs::create_dir_all(dir).await?;
            }
            let part = if legacy { path.clone() } else { part_of(&path) };
            let resume = n == from && n > 0 && at > 0 && !resumed;
            resumed = true;
            core.writing.lock().unwrap().insert(path.clone());
            let guard = Writing { core, path };
            let file = tokio::fs::OpenOptions::new().create(true).append(resume).write(!resume).truncate(!resume).open(&part).await?;
            current = Some((n, file, guard));
        }
        if len == 0 {
            if let Some((_, file, guard)) = current.take() {
                drop(file);
                if !legacy {
                    tokio::fs::rename(part_of(&guard.path), &guard.path).await?;
                }
            }
            continue;
        }
        buf.resize(len, 0);
        recv.read_exact(&mut buf).await?;
        if let Some((_, file, _)) = current.as_mut() {
            file.write_all(&buf).await?;
        }
        got += len as u64;
    }
    Ok(got)
}

async fn live_info(core: &Core, id: &EndpointId, conn: &Connection) -> Option<Value> {
    let info: Value = request(conn, &json!({"t": "live"})).await.ok().and_then(|r| serde_json::from_slice(&r).ok()).unwrap_or_default();
    let sharing = info["sharing"] == true && info["capture"].as_str().is_some_and(capture_id);
    core.update_peer(id, |p| p.sharing = sharing);
    sharing.then_some(info)
}

async fn follow_live(core: Arc<Core>, id: EndpointId, conn: Connection, room: String, which: usize, key: String) {
    let series = SERIES[which];
    while core.active_room_id() == room && conn.close_reason().is_none() && core.has_peer(&id) {
        let Some(info) = live_info(&core, &id, &conn).await else { break };
        let capture = info["capture"].as_str().unwrap_or_default().to_string();
        let n = info["n"][which].as_u64().unwrap_or(0);
        if n > 0 {
            let init = !core.capture_dir(&room, &id.to_string(), &capture).join(series).join(INIT).exists();
            let req = json!({"t": "follow", "room": room, "owner": id.to_string(), "capture": capture, "series": series, "from": n, "at": 0, "init": init, "live": true});
            if receive(&core, &conn, req).await.is_err() {
                break;
            }
        }
        sleep(RETRY).await;
    }
    core.following.lock().unwrap().remove(&key);
}

async fn fetch(core: Arc<Core>, conn: Connection, req: Value, key: String) {
    let started = Instant::now();
    let what = format!("{} {} {} from {}", req["set"].as_str().unwrap_or_default(), short(req["owner"].as_str().unwrap_or_default()), req["series"], req["from"]);
    match receive(&core, &conn, req).await {
        Ok(got) => debug::event(&core, started, format!("{what}: fetched {got} B from {}", short(conn.remote_id()))),
        Err(e) => debug::event(&core, started, format!("{what}: fetch from {} stopped: {e}", short(conn.remote_id()))),
    }
    core.following.lock().unwrap().remove(&key);
}

fn fetch_history(core: &Arc<Core>, peer: &EndpointId, conn: &Connection, room: &str) {
    if core.disk_low() {
        return;
    }
    let via = peer.to_string();
    let haves: Vec<(String, HashMap<String, u64>)> = core.peers.lock().unwrap().iter().filter(|p| p.conn.is_some()).map(|p| (p.id.to_string(), p.have.clone())).collect();
    let count = |who: &str, key: &str| haves.iter().find(|(id, _)| id == who).and_then(|(_, h)| h.get(key)).copied().unwrap_or(0);
    let focus = core.focus.lock().unwrap().clone();
    let mut sets = room::sets(core, room, &room::logs(core, room));
    sets.sort_by_key(|s| Some(&s.id) != focus.as_ref());
    let mut waiting = false;
    for set in &sets {
        let focused = Some(&set.id) == focus.as_ref();
        for f in &set.files {
            for series in [SERIES[1], SERIES[0]] {
                let Some(run) = data::missing(core, room, set, f, series) else { continue };
                let key = room::have_key(&set.id, &f.owner, series);
                let busy = format!("hist/{room}/{}/{}/{}/{series}", set.id, f.owner, run.capture);
                if focused && core.following.lock().unwrap().contains(&busy) {
                    waiting = true;
                }
                if waiting && !focused {
                    continue;
                }
                let here = count(&via, &key) > run.index && (f.owner == via || count(&f.owner, &key) <= run.index);
                if here && core.following.lock().unwrap().insert(busy.clone()) {
                    waiting |= focused;
                    let req = json!({"t": "follow", "room": room, "set": set.id, "owner": f.owner, "capture": run.capture, "series": series, "from": run.from, "at": run.at, "to": run.to, "init": run.init});
                    tokio::spawn(fetch(core.clone(), conn.clone(), req, busy));
                }
            }
        }
    }
}

async fn pull_live(core: &Arc<Core>, id: &EndpointId, conn: &Connection, room: &str) {
    if live_info(core, id, conn).await.is_none() {
        return;
    }
    for which in 0..SERIES.len() {
        let key = format!("live/{id}/{room}/{}", SERIES[which]);
        if core.following.lock().unwrap().insert(key.clone()) {
            tokio::spawn(follow_live(core.clone(), *id, conn.clone(), room.to_string(), which, key));
        }
    }
}

async fn measure(core: &Core, id: &EndpointId, conn: &Connection, samples: &mut Vec<(i64, i64)>) -> Result<()> {
    let t0 = now_us();
    let reply = request(conn, &json!({"t": "ping"})).await?;
    let t3 = now_us();
    let t1 = i64::from_le_bytes(reply[..8].try_into()?);
    samples.push((t3 - t0, t1 - (t0 + t3) / 2));
    if samples.len() > OFFSET_SAMPLES {
        samples.remove(0);
    }
    let (rtt, offset) = *samples.iter().min().unwrap();
    core.update_peer(id, |p| p.offset_us = Some(offset));
    core.append_log(&core.active_room_id(), &json!({"k": "offset", "peer": short(id), "t_us": t3, "rtt_us": rtt, "offset_us": offset}));
    Ok(())
}

async fn hello(core: &Arc<Core>, id: &EndpointId, conn: &Connection) -> bool {
    let mut message = hello_message(core);
    message["t"] = json!("hello");
    let reply = request(conn, &message).await;
    let Ok(reply) = reply.and_then(|r| Ok(serde_json::from_slice::<Value>(&r)?)) else { return true };
    learn_hello(core, id, &reply) == PROTOCOL
}

async fn present_invite(core: &Arc<Core>, id: &EndpointId, conn: &Connection) {
    let pending = core.active_room().and_then(|r| r.invite.filter(|(peer, _)| *peer == id.to_string()).map(|(_, token)| (r.id, token)));
    let Some((pending, token)) = pending else { return };
    let name = core.name.lock().unwrap().clone();
    let Ok(reply) = request(conn, &json!({"t": "join", "token": token, "name": name})).await else { return };
    let Ok(reply) = serde_json::from_slice::<Value>(&reply) else { return };
    if reply["declined"] == true {
        room::leave_room(core, &pending);
        *core.notice.lock().unwrap() = "Your request to join was declined.".into();
        return;
    }
    if reply["wait"] == true {
        if let Some(r) = core.rooms.lock().unwrap().list.iter_mut().find(|r| r.id == pending) {
            r.waiting = true;
        }
        return;
    }
    if reply["ok"] != true {
        return;
    }
    let (Some(real), Some(name)) = (reply["room"]["id"].as_str(), reply["room"]["name"].as_str()) else { return };
    room::adopt_room(core, &pending, real, name, &strings::<Vec<String>>(&reply["members"]));
    room::apply_active(core);
}

async fn sync(core: &Arc<Core>, id: &EndpointId, conn: &Connection) -> Result<()> {
    let mut samples = Vec::new();
    let mut tick = 0u32;
    loop {
        if tick % HELLO_EVERY == HELLO_EVERY - 1 {
            hello(core, id, conn).await;
        }
        if tick < MEASURE_FIRST || tick % MEASURE_EVERY == 0 {
            measure(core, id, conn, &mut samples).await?;
        }
        tick += 1;
        let room = core.active_room_id();
        let via = id.to_string();
        let people: Vec<String> = core.active_room().map(|r| r.people()).unwrap_or_default().into_iter().filter(|p| p.parse::<EndpointId>().is_ok()).collect();
        let pulls: Vec<_> = people
            .into_iter()
            .map(|of| {
                let (core, room, conn) = (core.clone(), room.clone(), conn.clone());
                tokio::spawn(async move { (pull_log(&core, &room, &of, &conn).await, of) })
            })
            .collect();
        for pull in pulls {
            if let Ok((Err(e), of)) = pull.await {
                if of == via {
                    return Err(e);
                }
            }
        }
        room::apply_removals(core);
        pull_live(core, id, conn, &room).await;
        fetch_history(core, id, conn, &room);
        tokio::select! {
            _ = sleep(LOG_SYNC) => {}
            _ = core.wake.notified() => {}
        }
    }
}

pub async fn dial_loop(core: Arc<Core>, id: EndpointId) {
    loop {
        sleep(RETRY).await;
        if !core.has_peer(&id) {
            return;
        }
        let dialing = Instant::now();
        let conn = match timeout(CONNECT_TIMEOUT, core.ep.connect(id, ALPN)).await {
            Ok(Ok(conn)) => conn,
            other => {
                core.update_peer(&id, |p| p.failures += 1);
                debug::event(&core, dialing, format!("{} connect failed: {:?}", short(&id), other.map(|r| r.map(|_| ()))));
                continue;
            }
        };
        debug::event(&core, dialing, format!("{} connected", short(&id)));
        core.update_peer(&id, |p| {
            p.conn = Some(conn.clone());
            p.failures = 0;
        });
        watch_paths(&core, &conn, format!("{} outgoing", short(&id)));
        present_invite(&core, &id, &conn).await;
        let compatible = hello(&core, &id, &conn).await;
        if !compatible {
            debug::event(&core, dialing, format!("{} protocol differs from ours {PROTOCOL}", short(&id)));
            sleep(MISMATCH_WAIT).await;
        } else {
            let _ = sync(&core, &id, &conn).await;
        }
        core.update_peer(&id, |p| {
            p.conn = None;
            p.sharing = false;
        });
        conn.close(0u32.into(), b"done");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_gets_its_share_while_live_fills_the_limit() {
        let mut pacer = Pacer::new();
        let mut now = Instant::now();
        let (rate, chunk) = (1000.0, 100);
        let (mut live, mut history) = (0, 0);
        for _ in 0..2000 {
            while pacer.next <= now + Duration::from_millis(50) {
                pacer.reserve(now, chunk, false, rate);
                live += chunk;
            }
            if pacer.reserve(now, chunk, true, rate).is_some() {
                history += chunk;
            }
            now += Duration::from_millis(10);
        }
        let total = (live + history) as f64;
        assert!(history as f64 >= total * HISTORY_SHARE * 0.8, "{history} of {total}");
        assert!(total <= rate * 20.0 * 1.05, "{total}");
        let mut idle = Pacer::new();
        assert!(idle.reserve(Instant::now(), chunk, true, rate).is_some());
    }
}

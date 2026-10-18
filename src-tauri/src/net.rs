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

use crate::room::{self, valid_id};
use crate::record;
use crate::{data, debug, default_codecs, file_len, log, merge, now_us, short, strings, Core, ALPN, PROTOCOL, VERSION};

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
const HISTORY_PRIORITY: i32 = -1;
const HISTORY_WAIT: Duration = Duration::from_millis(50);
const UP: usize = 0;
const DOWN: usize = 1;

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
    proto
}

fn bytes_per_sec(core: &Core) -> Option<f64> {
    core.limit_mbps.lock().unwrap().map(|mbps| mbps * BYTES_PER_MBIT)
}

fn reserve(core: &Core, way: usize, bytes: usize, history: bool, rate: f64) -> Option<Duration> {
    let mut next = core.pacers[way].lock().unwrap();
    let now = Instant::now();
    if history && *next > now {
        return None;
    }
    *next = (*next).max(now) + Duration::from_secs_f64(bytes as f64 / rate);
    Some(*next - now)
}

async fn pace(core: &Core, way: usize, bytes: usize, history: bool) {
    let Some(rate) = bytes_per_sec(core) else { return };
    loop {
        match reserve(core, way, bytes, history, rate) {
            Some(due) => return sleep(due).await,
            None => sleep(HISTORY_WAIT).await,
        }
    }
}

fn holdings(core: &Core) -> Value {
    let room = core.active_room_id();
    json!({"have": room::have_list(core, &room), "gone": room::gone_list(core, &room)})
}

fn learn_holdings(core: &Core, id: &EndpointId, message: &Value) {
    if message["room"]["id"].as_str() != Some(core.active_room_id().as_str()) {
        return;
    }
    core.update_peer(id, |p| {
        p.have = strings(&message["have"]);
        p.gone = strings(&message["gone"]);
    });
}

fn hello_message(core: &Core) -> Value {
    let room = room_info(core);
    let mut message = json!({"app": VERSION, "proto": PROTOCOL, "name": *core.name.lock().unwrap(), "room": room, "members": room["members"], "set": core.active_set_json(), "codecs": *core.codecs.lock().unwrap()});
    merge(&mut message, &holdings(core));
    message
}

fn in_active_room(core: &Core, room: &str) -> bool {
    valid_id(room) && room == core.active_room_id()
}

fn follow_key(id: &EndpointId, room: &str, what: &str) -> String {
    format!("{id}/{room}/{what}")
}

async fn stream(core: &Core, send: &mut SendStream, path: std::path::PathBuf, from: u64, history: bool, still: impl Fn() -> bool) -> Result<()> {
    let Ok(mut file) = tokio::fs::File::open(path).await else { return Ok(()) };
    file.seek(std::io::SeekFrom::Start(from)).await?;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            if !still() {
                break;
            }
            sleep(FILE_WATCH).await;
            continue;
        }
        send.write_all(&buf[..n]).await?;
        pace(core, UP, n, history).await;
    }
    Ok(())
}

async fn follow(core: &Core, send: &mut SendStream, req: &Value) -> Result<()> {
    let (set, room, live) = (req["set"].as_str().unwrap_or_default(), req["room"].as_str().unwrap_or_default(), req["live"].as_str());
    if !in_active_room(core, room) || !core.share.load(Ordering::Relaxed) {
        return Ok(());
    }
    let own = core.ep.id().to_string();
    let path = match live {
        Some(name) if record::live_name(name).is_some_and(|(who, _, _)| who == own) => core.live_dir(room).join(name),
        None if valid_id(set) => core.own_file(room, set),
        _ => return Ok(()),
    };
    if live.is_none() {
        let _ = send.set_priority(HISTORY_PRIORITY);
    }
    let still = || {
        core.share.load(Ordering::Relaxed)
            && match live {
                Some(name) => core.engine.live_file().as_deref() == Some(name),
                None => core.is_recording(room, set),
            }
    };
    stream(core, send, path, req["from"].as_u64().unwrap_or(0), false, still).await
}

async fn serve_file(core: &Core, send: &mut SendStream, req: &Value) -> Result<()> {
    let (set, room, owner) = (req["set"].as_str().unwrap_or_default(), req["room"].as_str().unwrap_or_default(), req["owner"].as_str().unwrap_or_default());
    if !in_active_room(core, room) || !valid_id(set) || owner.parse::<EndpointId>().is_err() {
        return Ok(());
    }
    if owner == core.ep.id().to_string() && !core.share.load(Ordering::Relaxed) {
        return Ok(());
    }
    if !data::verified_files(core, &core.set_dir(room, set)).contains(&format!("{owner}.mp4")) {
        return Ok(());
    }
    let _ = send.set_priority(HISTORY_PRIORITY);
    stream(core, send, core.set_file(room, set, owner), req["from"].as_u64().unwrap_or(0), true, || false).await
}

fn react_to_set(core: &Arc<Core>, set: &Value) {
    let (Some(id), Some(room)) = (set["id"].as_str(), set["room"].as_str()) else { return };
    if !valid_id(id) || room != core.active_room_id() || core.is_deleted(room, id) {
        return;
    }
    if core.own_file(room, id).exists() {
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
            let path = match req["of"].as_str().map(|of| of.parse::<EndpointId>()) {
                None => Some(core.log_path(&core.ep.id())),
                Some(Ok(of)) if of != core.ep.id() && core.active_room().is_some_and(|r| r.members.contains(&of.to_string())) => Some(core.log_path(&of)),
                Some(_) => None,
            };
            if let Some(path) = path {
                if let Ok(mut file) = tokio::fs::File::open(path).await {
                    file.seek(std::io::SeekFrom::Start(req["from"].as_u64().unwrap_or(0))).await?;
                    let mut data = Vec::new();
                    file.take(PULL_MAX).read_to_end(&mut data).await?;
                    send.write_all(&data).await?;
                }
            }
        }
        Some("follow") => follow(&core, &mut send, &req).await?,
        Some("file") => serve_file(&core, &mut send, &req).await?,
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
            send.write_all(&serde_json::to_vec(&json!({"sharing": sharing, "live": core.engine.live_file().filter(|_| sharing)}))?).await?;
        }
        Some("ping") => send.write_all(&now_us().to_le_bytes()).await?,
        Some("ctl") => {
            let wall = req["wall_us"].as_i64().unwrap_or(0);
            let _ = core.app.emit("control", json!({"cmd": req["cmd"], "wall_us": wall, "set": req["set"], "sent_us": req["sent_us"], "origin": req["origin"], "from": from.to_string()}));
            send.write_all(b"ok").await?;
        }
        _ => {}
    }
    send.finish()?;
    Ok(())
}

async fn join(core: &Arc<Core>, from: EndpointId, mut send: SendStream, mut recv: RecvStream) -> Result<()> {
    let req: Value = serde_json::from_slice(&recv.read_to_end(JOIN_MAX).await?)?;
    let token = req["token"].as_str().unwrap_or_default().to_string();
    let room_id = core.active_room_id();
    let valid = req["t"] == "join" && {
        let mut invites = core.invites.lock().unwrap();
        let before = invites.len();
        invites.retain(|(t, r)| !(*t == token && *r == room_id));
        invites.len() < before
    };
    if valid {
        core.removed.lock().unwrap().remove(&from.to_string());
        room::add_member(core, &room_id, &from.to_string());
        let room = room_info(core);
        send.write_all(&serde_json::to_vec(&json!({"ok": true, "room": room, "members": room["members"]}))?).await?;
    }
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

async fn pull_log(core: &Core, of: &EndpointId, via: &EndpointId, conn: &Connection) -> Result<()> {
    tokio::fs::create_dir_all(core.peer_dir(of)).await?;
    let path = core.log_path(of);
    let from = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
    let mut req = json!({"t": "pull", "file": "log.bin", "from": from});
    if of != via {
        req["of"] = json!(of.to_string());
    }
    let data = request(conn, &req).await?;
    if !data.is_empty() {
        let _guard = core.log_lock.lock().unwrap();
        log::extend(&path, from, &data)?;
    }
    Ok(())
}

fn recording_ended(core: &Core, id: &EndpointId, set: &str) -> bool {
    core.log(id).iter().any(|r| r["k"] == "rec_end" && r["set"] == set)
}

async fn receive(core: &Core, conn: &Connection, mut req: Value, path: &std::path::Path) -> Result<u64> {
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    req["from"] = json!(tokio::fs::metadata(path).await.map(|m| m.len()).unwrap_or(0));
    let history = req["t"] == "file";
    let (room, set) = (req["room"].as_str().unwrap_or_default().to_string(), req["set"].as_str().unwrap_or_default().to_string());
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&serde_json::to_vec(&req)?).await?;
    send.finish()?;
    let mut file = tokio::fs::OpenOptions::new().create(true).append(true).open(path).await?;
    let mut got = 0u64;
    let mut buf = vec![0u8; CHUNK];
    while let Some(n) = recv.read(&mut buf).await? {
        if core.disk_full() || core.is_deleted(&room, &set) {
            break;
        }
        file.write_all(&buf[..n]).await?;
        got += n as u64;
        pace(core, DOWN, n, history).await;
    }
    Ok(got)
}

async fn follow_recording(core: Arc<Core>, id: EndpointId, conn: Connection, room: String, set: String) {
    let key = follow_key(&id, &room, &set);
    let path = core.set_file(&room, &set, &id.to_string());
    while core.active_room_id() == room && core.has_peer(&id) && !core.is_deleted(&room, &set) {
        match receive(&core, &conn, json!({"t": "follow", "set": set, "room": room}), &path).await {
            Ok(0) if recording_ended(&core, &id, &set) => break,
            Ok(_) => sleep(RETRY).await,
            Err(_) => break,
        }
    }
    core.following.lock().unwrap().remove(&key);
}

async fn live_info(core: &Core, id: &EndpointId, conn: &Connection) -> Option<String> {
    let info: Value = request(conn, &json!({"t": "live"})).await.ok().and_then(|r| serde_json::from_slice(&r).ok()).unwrap_or_default();
    let live = info["live"].as_str().filter(|n| record::live_name(n).is_some_and(|(who, _, _)| who == id.to_string())).map(String::from);
    core.update_peer(id, |p| {
        p.sharing = info["sharing"] == true;
        p.live = live.clone();
    });
    live
}

async fn follow_live(core: Arc<Core>, id: EndpointId, conn: Connection, room: String, mut name: String) {
    let key = follow_key(&id, &room, "live");
    while core.active_room_id() == room && conn.close_reason().is_none() {
        let path = core.live_dir(&room).join(&name);
        if receive(&core, &conn, json!({"t": "follow", "room": room, "live": name}), &path).await.is_err() {
            break;
        }
        match live_info(&core, &id, &conn).await {
            Some(next) if next != name => name = next,
            Some(_) => sleep(RETRY).await,
            None => break,
        }
    }
    core.following.lock().unwrap().remove(&key);
}

async fn fetch_file(core: Arc<Core>, conn: Connection, room: String, set: String, owner: String, key: String) {
    let path = core.set_file(&room, &set, &owner);
    let started = Instant::now();
    match receive(&core, &conn, json!({"t": "file", "room": room, "set": set, "owner": owner}), &path).await {
        Ok(got) => debug::event(&core, started, format!("{set} {}: fetched {got} B from {}", short(&owner), short(conn.remote_id()))),
        Err(e) => debug::event(&core, started, format!("{set} {}: fetch from {} stopped: {e}", short(&owner), short(conn.remote_id()))),
    }
    core.following.lock().unwrap().remove(&key);
}

fn fetch_history(core: &Arc<Core>, peer: &EndpointId, conn: &Connection, room: &str) {
    let via = peer.to_string();
    let offered = |owner: &str, key: &str| core.peers.lock().unwrap().iter().any(|p| p.id.to_string() == owner && p.conn.is_some() && p.have.contains(key));
    for set in room::sets(core, room, &room::logs(core, room)) {
        for f in &set.files {
            if f.end_us.is_none() {
                if f.owner == via && core.following.lock().unwrap().insert(follow_key(peer, room, &set.id)) {
                    tokio::spawn(follow_recording(core.clone(), *peer, conn.clone(), room.to_string(), set.id.clone()));
                }
                continue;
            }
            let Some(total) = f.total else { continue };
            if file_len(&core.set_file(room, &set.id, &f.owner)) >= total || core.disk_low() {
                continue;
            }
            let hk = room::have_key(&set.id, &f.owner);
            let here = offered(&via, &hk) && (f.owner == via || !offered(&f.owner, &hk));
            let key = format!("file/{room}/{}/{}", set.id, f.owner);
            if here && core.following.lock().unwrap().insert(key.clone()) {
                tokio::spawn(fetch_file(core.clone(), conn.clone(), room.to_string(), set.id.clone(), f.owner.clone(), key));
            }
        }
    }
}

async fn pull_live(core: &Arc<Core>, id: &EndpointId, conn: &Connection, room: &str) {
    let key = follow_key(id, room, "live");
    if core.following.lock().unwrap().contains(&key) {
        return;
    }
    if let Some(name) = live_info(core, id, conn).await {
        if core.following.lock().unwrap().insert(key) {
            tokio::spawn(follow_live(core.clone(), *id, conn.clone(), room.to_string(), name));
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
    core.append_log(&json!({"k": "offset", "peer": short(id), "t_us": t3, "rtt_us": rtt, "offset_us": offset}));
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
    let Ok(reply) = request(conn, &json!({"t": "join", "token": token})).await else { return };
    let Ok(reply) = serde_json::from_slice::<Value>(&reply) else { return };
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
        pull_log(core, id, id, conn).await?;
        let me = core.ep.id();
        let others: Vec<EndpointId> = core.active_room().map(|r| r.members).unwrap_or_default().iter().filter_map(|m| m.parse().ok()).filter(|m| m != id && *m != me).collect();
        let pulls: Vec<_> = others
            .into_iter()
            .map(|other| {
                let (core, id, conn) = (core.clone(), *id, conn.clone());
                tokio::spawn(async move {
                    let _ = pull_log(&core, &other, &id, &conn).await;
                })
            })
            .collect();
        for pull in pulls {
            let _ = pull.await;
        }
        let room = core.active_room_id();
        pull_live(core, id, conn, &room).await;
        fetch_history(core, id, conn, &room);
        sleep(LOG_SYNC).await;
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
            p.live = None;
        });
        conn.close(0u32.into(), b"done");
    }
}

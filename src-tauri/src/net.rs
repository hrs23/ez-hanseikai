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
use crate::{debug, log, now_us, short, Core, ALPN, PROTOCOL, VERSION};

const CHUNK: usize = 64 << 10;
const LOG_SYNC: Duration = Duration::from_secs(2);
const FILE_WATCH: Duration = Duration::from_millis(250);
const RETRY: Duration = Duration::from_secs(2);
const FINISHED_RECHECK: Duration = Duration::from_secs(30);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

async fn request(conn: &Connection, req: &Value) -> Result<Vec<u8>> {
    timeout(REQUEST_TIMEOUT, async {
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(&serde_json::to_vec(req)?).await?;
        send.finish()?;
        Ok(recv.read_to_end(32 << 20).await?)
    })
    .await?
}

pub async fn send_control(core: &Arc<Core>, message: &Value) {
    for conn in core.connections() {
        let _ = request(&conn, message).await;
    }
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
        Some(r) => json!({"id": r.id, "name": r.name, "members": r.members}),
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
}

fn announce_join(core: &Core, id: &EndpointId, name: &str) {
    let first = core.update_peer(id, |p| std::mem::take(&mut p.announce) && p.info.is_none()).unwrap_or(false);
    if first {
        let shown = if name.is_empty() { short(id) } else { name.to_string() };
        let _ = core.app.emit("joined", shown);
    }
}

fn codec_list(value: &Value) -> Vec<String> {
    let list: Vec<String> = value.as_array().into_iter().flatten().filter_map(|v| v.as_str()).filter(|c| ["h264", "hevc", "av1"].contains(c)).map(String::from).collect();
    if list.is_empty() { vec!["h264".to_string()] } else { list }
}

fn bytes_per_sec(core: &Core) -> Option<f64> {
    core.limit_mbps.lock().unwrap().map(|mbps| mbps * 125_000.0)
}

async fn pace(rate: Option<f64>, started: Instant, bytes: u64) {
    if let Some(rate) = rate {
        let due = Duration::from_secs_f64(bytes as f64 / rate);
        if due > started.elapsed() {
            sleep(due - started.elapsed()).await;
        }
    }
}

fn set_active(core: &Core, set: &str) -> bool {
    core.active_set.lock().unwrap().as_ref().is_some_and(|s| s.id == set)
}

async fn follow(core: &Core, send: &mut SendStream, req: &Value) -> Result<()> {
    let (set, room) = (req["set"].as_str().unwrap_or_default(), req["room"].as_str().unwrap_or_default());
    if !valid_id(set) || !valid_id(room) || room != core.active_room_id() || !core.share.load(Ordering::Relaxed) {
        return Ok(());
    }
    let path = core.set_dir(room, set).join(format!("{}.mp4", core.ep.id()));
    let Ok(mut file) = tokio::fs::File::open(path).await else { return Ok(()) };
    file.seek(std::io::SeekFrom::Start(req["from"].as_u64().unwrap_or(0))).await?;
    let started = Instant::now();
    let mut sent = 0u64;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file.read(&mut buf).await?;
        if n == 0 {
            if !set_active(core, set) || !core.share.load(Ordering::Relaxed) {
                break;
            }
            sleep(FILE_WATCH).await;
            continue;
        }
        send.write_all(&buf[..n]).await?;
        sent += n as u64;
        pace(bytes_per_sec(core), started, sent).await;
    }
    Ok(())
}

fn react_to_set(core: &Arc<Core>, set: &Value) {
    let (Some(id), Some(room)) = (set["id"].as_str(), set["room"].as_str()) else { return };
    if !valid_id(id) || room != core.active_room_id() || core.recorder.lock().unwrap().is_some() {
        return;
    }
    if core.set_dir(room, id).join(format!("{}.mp4", core.ep.id())).exists() {
        return;
    }
    let (core, id, by, start_us) = (core.clone(), id.to_string(), set["by"].as_str().unwrap_or_default().to_string(), set["start_us"].as_i64().unwrap_or(0));
    tokio::task::spawn_blocking(move || {
        let _ = room::begin_set(&core, &id, &by, start_us);
    });
}

async fn handle(core: Arc<Core>, from: EndpointId, mut send: SendStream, mut recv: RecvStream) -> Result<()> {
    let req: Value = serde_json::from_slice(&recv.read_to_end(4096).await?)?;
    match req["t"].as_str() {
        Some("pull") => {
            if req["file"].as_str() == Some("log.bin") {
                if let Ok(mut file) = tokio::fs::File::open(core.dir.join("log.bin")).await {
                    file.seek(std::io::SeekFrom::Start(req["from"].as_u64().unwrap_or(0))).await?;
                    let mut data = Vec::new();
                    file.take(8 << 20).read_to_end(&mut data).await?;
                    send.write_all(&data).await?;
                }
            }
        }
        Some("follow") => follow(&core, &mut send, &req).await?,
        Some("hello") => {
            let version = req["app"].as_str().unwrap_or("unknown").to_string();
            let proto = req["proto"].as_u64().unwrap_or(0) as u32;
            let name = crate::clean_name(req["name"].as_str().unwrap_or_default());
            announce_join(&core, &from, &name);
            let codecs = codec_list(&req["codecs"]);
            core.update_peer(&from, |p| {
                p.info = Some((version, proto));
                p.name = name;
                p.codecs = codecs;
            });
            learn_members(&core, &req["room"], &req["members"]);
            react_to_set(&core, &req["set"]);
            let own = core.name.lock().unwrap().clone();
            let room = room_info(&core);
            send.write_all(&serde_json::to_vec(&json!({"app": VERSION, "proto": PROTOCOL, "name": own, "room": room, "members": room["members"], "set": core.active_set_json(), "codecs": *core.codecs.lock().unwrap()}))?).await?;
        }
        Some("rec") => {
            if req["action"] == "start" {
                react_to_set(&core, &json!({"id": req["set"], "room": req["room"], "by": req["by"], "start_us": req["start_us"]}));
            } else if req["action"] == "stop" && set_active(&core, req["set"].as_str().unwrap_or_default()) {
                let core = core.clone();
                tokio::task::spawn_blocking(move || room::end_set(&core));
            }
            send.write_all(b"ok").await?;
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
    let req: Value = serde_json::from_slice(&recv.read_to_end(4096).await?)?;
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

async fn pull_log(core: &Core, id: &EndpointId, conn: &Connection) -> Result<()> {
    let dir = core.peer_dir(id);
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join("log.bin");
    let from = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
    let data = request(conn, &json!({"t": "pull", "file": "log.bin", "from": from})).await?;
    if !data.is_empty() {
        let mut file = tokio::fs::OpenOptions::new().create(true).append(true).open(&path).await?;
        file.write_all(&data).await?;
    }
    Ok(())
}

fn recording_ended(core: &Core, id: &EndpointId, set: &str) -> bool {
    log::read(&core.peer_dir(id).join("log.bin")).iter().any(|r| r["k"] == "rec_end" && r["set"] == set)
}

async fn receive(core: &Core, id: &EndpointId, conn: &Connection, room: &str, set: &str) -> Result<u64> {
    let dir = core.set_dir(room, set);
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(format!("{id}.mp4"));
    let from = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&serde_json::to_vec(&json!({"t": "follow", "set": set, "room": room, "from": from}))?).await?;
    send.finish()?;
    let mut file = tokio::fs::OpenOptions::new().create(true).append(true).open(&path).await?;
    let started = Instant::now();
    let mut got = 0u64;
    let mut buf = vec![0u8; CHUNK];
    while let Some(n) = recv.read(&mut buf).await? {
        file.write_all(&buf[..n]).await?;
        got += n as u64;
        pace(bytes_per_sec(core), started, got).await;
    }
    Ok(got)
}

async fn follow_recording(core: Arc<Core>, id: EndpointId, conn: Connection, room: String, set: String) {
    let key = format!("{id}/{room}/{set}");
    loop {
        if core.active_room_id() != room || !core.has_peer(&id) {
            break;
        }
        match receive(&core, &id, &conn, &room, &set).await {
            Ok(0) if recording_ended(&core, &id, &set) => sleep(FINISHED_RECHECK).await,
            Ok(_) => sleep(RETRY).await,
            Err(_) => break,
        }
    }
    core.following.lock().unwrap().remove(&key);
}

async fn measure(core: &Core, id: &EndpointId, conn: &Connection, samples: &mut Vec<(i64, i64)>) -> Result<()> {
    let t0 = now_us();
    let reply = request(conn, &json!({"t": "ping"})).await?;
    let t3 = now_us();
    let t1 = i64::from_le_bytes(reply[..8].try_into()?);
    samples.push((t3 - t0, t1 - (t0 + t3) / 2));
    if samples.len() > 8 {
        samples.remove(0);
    }
    let (rtt, offset) = *samples.iter().min().unwrap();
    core.update_peer(id, |p| p.offset_us = Some(offset));
    core.append_log(&json!({"k": "offset", "peer": short(id), "t_us": t3, "rtt_us": rtt, "offset_us": offset}));
    Ok(())
}

async fn hello(core: &Arc<Core>, id: &EndpointId, conn: &Connection) -> bool {
    let own = core.name.lock().unwrap().clone();
    let room = room_info(core);
    let message = json!({"t": "hello", "app": VERSION, "proto": PROTOCOL, "name": own, "room": room, "members": room["members"], "set": core.active_set_json(), "codecs": *core.codecs.lock().unwrap()});
    let reply = request(conn, &message).await;
    let Ok(reply) = reply.and_then(|r| Ok(serde_json::from_slice::<Value>(&r)?)) else { return true };
    let proto = reply["proto"].as_u64().unwrap_or(0) as u32;
    let version = reply["app"].as_str().unwrap_or("unknown").to_string();
    let name = crate::clean_name(reply["name"].as_str().unwrap_or_default());
    announce_join(core, id, &name);
    let codecs = codec_list(&reply["codecs"]);
    core.update_peer(id, |p| {
        p.info = Some((version, proto));
        p.name = name;
        p.codecs = codecs;
    });
    learn_members(core, &reply["room"], &reply["members"]);
    react_to_set(core, &reply["set"]);
    proto == PROTOCOL
}

async fn present_invite(core: &Arc<Core>, id: &EndpointId, conn: &Connection) {
    let token = core.pending_join.lock().unwrap().get(id).cloned();
    let Some(token) = token else { return };
    let Ok(reply) = request(conn, &json!({"t": "join", "token": token})).await else { return };
    let Ok(reply) = serde_json::from_slice::<Value>(&reply) else { return };
    if reply["ok"] != true {
        return;
    }
    core.pending_join.lock().unwrap().remove(id);
    let (Some(real), Some(name)) = (reply["room"]["id"].as_str(), reply["room"]["name"].as_str()) else { return };
    let members: Vec<String> = reply["members"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect();
    room::adopt_room(core, &format!("pending-{token}"), real, name, &members);
    room::apply_active(core);
}

async fn sync(core: &Arc<Core>, id: &EndpointId, conn: &Connection) -> Result<()> {
    let mut samples = Vec::new();
    let mut tick = 0u32;
    loop {
        if tick % 3 == 2 {
            hello(core, id, conn).await;
        }
        if tick < 5 || tick % 15 == 0 {
            measure(core, id, conn, &mut samples).await?;
        }
        tick += 1;
        pull_log(core, id, conn).await?;
        let room = core.active_room_id();
        let peer_log = log::read(&core.peer_dir(id).join("log.bin"));
        for r in peer_log.iter().filter(|r| r["k"] == "rec" && r["room"] == room.as_str()) {
            let Some(set) = r["set"].as_str().filter(|s| valid_id(s)) else { continue };
            if core.following.lock().unwrap().insert(format!("{id}/{room}/{set}")) {
                tokio::spawn(follow_recording(core.clone(), *id, conn.clone(), room.clone(), set.to_string()));
            }
        }
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
        let conn = match timeout(Duration::from_secs(15), core.ep.connect(id, ALPN)).await {
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
            sleep(Duration::from_secs(10)).await;
        } else {
            let _ = sync(&core, &id, &conn).await;
        }
        core.update_peer(&id, |p| p.conn = None);
        conn.close(0u32.into(), b"done");
    }
}

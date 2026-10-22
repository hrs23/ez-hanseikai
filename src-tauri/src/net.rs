use std::collections::{HashMap, HashSet};
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use iroh::endpoint::{Connection, ConnectionError, RecvStream, SendStream, Side};
use iroh::EndpointId;
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio::time::{sleep, timeout};

use crate::record::{self, LiveStep};
use crate::room::{self, capture_id, part_of, seg_name, valid_id, INIT, SERIES};
use crate::{data, debug, default_codecs, relay, tenth, file_len, log, merge, now_us, party, short, strings, test_flag, Core, ALPN, PROTOCOL, VERSION};

const CHUNK: usize = 64 << 10;
pub const LOG_SYNC: Duration = Duration::from_secs(2);
const FILE_WATCH: Duration = Duration::from_millis(250);
const RETRY: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(30);
const RETRY_DOUBLINGS: u32 = 4;
const RETRY_JITTER: f64 = 0.25;
const STABLE_FOR: Duration = Duration::from_secs(10);
const PING_WAIT: Duration = Duration::from_secs(3);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_MAX: usize = 4 << 20;
const SPEED_BYTES: usize = 2 << 20;
const SPEED_TIMEOUT: Duration = Duration::from_secs(60);
const RESPONSE_MAX: usize = 32 << 20;
const PULL_MAX: u64 = 1 << 20;
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
const HISTORY_FETCHES: usize = 4;
const HISTORY_KEY: &str = "hist/";
const FRAME_HEADER: usize = 8;

pub async fn request(conn: &Connection, req: &Value) -> Result<Vec<u8>> {
    request_within(conn, req, REQUEST_TIMEOUT).await
}

async fn request_within(conn: &Connection, req: &Value, limit: Duration) -> Result<Vec<u8>> {
    timeout(limit, async {
        let (mut send, mut recv) = conn.open_bi().await?;
        let body = serde_json::to_vec(req)?;
        debug::message("out", req["t"].as_str(), body.len());
        send.write_all(&body).await?;
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

fn learn_members(core: &Arc<Core>, room: &Value) {
    let Some(id) = room["id"].as_str().filter(|id| core.in_room(id)) else { return };
    for member in room["members"].as_array().into_iter().flatten().filter_map(|v| v.as_str()) {
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
        p.seen_us = Some(now_us());
        p.codecs = codecs;
    });
    room::remember_name(core, &id.to_string(), &name, true);
    if proto != PROTOCOL {
        return proto;
    }
    learn_members(core, &message["room"]);
    learn_holdings(core, id, message);
    react_to_set(core, &message["set"]);
    if core.in_room(message["room"]["id"].as_str().unwrap_or_default()) {
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
    loop {
        let Some(rate) = bytes_per_sec(core) else { return };
        let Some(due) = core.pacer.lock().unwrap().reserve(Instant::now(), bytes, history, rate) else {
            sleep(HISTORY_WAIT).await;
            continue;
        };
        let until = Instant::now() + due;
        while bytes_per_sec(core) == Some(rate) && Instant::now() < until {
            sleep(until.saturating_duration_since(Instant::now()).min(HISTORY_WAIT)).await;
        }
        return;
    }
}

fn holdings(core: &Core) -> Value {
    static HELD: Mutex<Option<(Instant, String, Value)>> = Mutex::new(None);
    let Some(room) = core.room_id() else { return json!({}) };
    let mut held = HELD.lock().unwrap();
    if let Some((_, _, value)) = held.as_ref().filter(|(at, r, _)| at.elapsed() < LOG_SYNC / 2 && *r == room) {
        return value.clone();
    }
    let value = count_holdings(core, &room);
    *held = Some((Instant::now(), room, value.clone()));
    value
}

fn count_holdings(core: &Core, room: &str) -> Value {
    let room = room.to_string();
    let logs = room::logs(core, &room);
    let sets = room::sets(core, &room, &logs);
    let lengths: HashMap<String, u64> = logs.iter().map(|l| (l.owner.clone(), file_len(&core.log_path(&room, &l.owner)))).collect();
    json!({"have": data::have_counts(core, &room, &sets), "logs": lengths})
}

fn learn_holdings(core: &Core, id: &EndpointId, message: &Value) {
    if !core.in_room(message["room"]["id"].as_str().unwrap_or_default()) {
        return;
    }
    core.update_peer(id, |p| {
        p.have = message["have"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_u64()?))).collect();
        p.logs = message["logs"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((format!("{}/{k}", message["room"]["id"].as_str()?), v.as_u64()?))).collect();
    });
}

fn hello_message(core: &Core) -> Value {
    let mut message = json!({"t": "hello", "app": VERSION, "proto": PROTOCOL, "name": *core.name.lock().unwrap(), "room": room_info(core), "set": core.active_set_json(), "codecs": *core.codecs.lock().unwrap(), "asks": pending_asks(core)});
    merge(&mut message, &holdings(core));
    message
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

async fn send_file(core: &Core, send: &mut SendStream, n: u32, path: &Path, at: u64, history: bool, tail: &impl Fn() -> bool, what: &debug::What) -> Result<bool> {
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
    let row = debug::start("out", what, n);
    let mut sent = at;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let k = file.read(&mut buf).await?;
        if k > 0 {
            while history && core.live_at.lock().unwrap().is_some_and(|t| t.elapsed() < HISTORY_WAIT) {
                sleep(HISTORY_WAIT).await;
            }
            let wrote = Instant::now();
            write_frame(send, n, &buf[..k]).await?;
            if !history {
                *core.live_at.lock().unwrap() = Some(Instant::now());
            }
            debug::slow_send(what, wrote.elapsed());
            row.add(k);
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
    row.finish();
    Ok(true)
}

fn note_drop(core: &Core, from: &EndpointId) {
    let now = Instant::now();
    let starts = core.update_peer(from, |p| {
        let starts = p.drops[1].is_none_or(|d| now.duration_since(d) >= record::THIN_WINDOW);
        p.drops = [p.drops[1], Some(now)];
        starts
    });
    if starts == Some(true) {
        debug::event(core, now, format!("{}: live picture fell behind, restarting at the newest segment", short(from)));
    }
    record::lower_if_thin(core);
}

async fn follow(core: &Core, conn: &Connection, send: &mut SendStream, req: &Value) -> Result<()> {
    let from = conn.remote_id();
    let text = |k: &str| req[k].as_str().unwrap_or_default().to_string();
    let (room, owner, capture, series) = (text("room"), text("owner"), text("capture"), text("series"));
    let set = Some(text("set")).filter(|s| valid_id(s));
    let own = owner == core.me();
    let live = req["live"] == true;
    let valid = core.in_room(&room) && room::member_id(&owner) && SERIES.contains(&series.as_str()) && capture_id(&capture);
    let picture = series == SERIES[0];
    let relayed = || picture && relay::relayed(conn);
    if !valid || (live && !own) || (!own && set.is_none()) || relayed() {
        return Ok(());
    }
    let _ = send.set_priority(priority(live, &series));
    let what = debug::what(from, req, priority(live, &series));
    let dir = core.series_dir(&room, set.as_deref(), &owner, &capture, &series);
    let which = if series == SERIES[0] { 0 } else { 1 };
    let current = || core.engine.live().filter(|l| l.room == room && l.capture == capture);
    let sets = if own { Vec::new() } else { room::sets(core, &room, &room::logs(core, &room)) };
    let file = sets.iter().filter(|s| set.as_ref() == Some(&s.id)).flat_map(|s| s.files.iter()).find(|f| f.owner == owner);
    let held = |n: u32| file.and_then(|f| f.segs_of(&capture, &series).find(|g| g.n == n)).is_some_and(|g| data::has(&core.files, &dir, &seg_name(n), Some(g.size)));
    if req["init"] == true {
        let ok = own || file.is_some_and(|f| data::has(&core.files, &dir, INIT, f.init(&capture, &series).map(|(size, _)| size)));
        if !ok || !send_file(core, send, 0, &dir.join(INIT), 0, !live, &|| false, &what).await? {
            return Ok(());
        }
    }
    let to = req["to"].as_u64().map(|t| t as u32);
    let (mut n, mut at) = (req["from"].as_u64().unwrap_or(1) as u32, req["at"].as_u64().unwrap_or(0));
    while to.is_none_or(|to| n <= to) && !relayed() {
        let path = dir.join(seg_name(n));
        let sent = if own && live {
            match current().map(|l| record::live_step(&l.recent[which], set.as_deref(), n)) {
                Some(LiveStep::Wait) => {
                    sleep(FILE_WATCH).await;
                    continue;
                }
                Some(LiveStep::Serve) => {
                    if picture {
                        core.update_peer(&from, |p| p.served = Some(Instant::now()));
                    }
                    send_file(core, send, n, &path, at, false, &|| current().is_some(), &what).await?
                }
                Some(LiveStep::Behind) if picture => {
                    note_drop(core, &from);
                    false
                }
                _ => false,
            }
        } else {
            let ready = if own { path.exists() } else { held(n) };
            ready && send_file(core, send, n, &path, at, true, &|| false, &what).await?
        };
        if !sent {
            break;
        }
        n += 1;
        at = 0;
    }
    Ok(())
}

fn react_to_set(core: &Arc<Core>, set: &Value) {
    let (Some(id), Some(room)) = (set["id"].as_str(), set["room"].as_str()) else { return };
    if !valid_id(id) || !core.in_room(room) {
        return;
    }
    let logs = room::logs(core, room);
    if !room::may_join(&core.own_log(room), id) || room::sets(core, room, &logs).iter().any(|s| s.id == id && s.stopped) || room::dropped(&logs).contains(id) {
        return;
    }
    let (by, start_us, voice) = (set["by"].as_str().unwrap_or_default().to_string(), set["start_us"].as_i64().unwrap_or(0), set["voice"] == true);
    let of = set["of"].as_str().filter(|of| valid_id(of)).map(String::from);
    let newer = match core.active_set.lock().unwrap().as_ref() {
        None => false,
        Some(a) if (a.start_us, a.id.as_str()) < (start_us, id) => true,
        Some(_) => return,
    };
    if !newer {
        let _ = room::begin_set(core, room, id, &by, start_us, voice, of);
        return;
    }
    let (core, room, id) = (core.clone(), room.to_string(), id.to_string());
    std::thread::spawn(move || {
        room::end_set(&core, false, None);
        let _ = room::begin_set(&core, &room, &id, &by, start_us, voice, of);
    });
}

fn compatible(core: &Core, id: &EndpointId) -> bool {
    core.peers.lock().unwrap().iter().find(|p| p.id == *id).is_none_or(|p| p.info.as_ref().is_none_or(|(_, proto)| *proto == PROTOCOL))
}

async fn handle(core: Arc<Core>, conn: Connection, mut send: SendStream, mut recv: RecvStream) -> Result<()> {
    let from = conn.remote_id();
    let body = recv.read_to_end(REQUEST_MAX).await?;
    let req: Value = serde_json::from_slice(&body)?;
    if !core.has_peer(&from) || (req["t"] != "hello" && !compatible(&core, &from)) {
        return Ok(());
    }
    let _ = send.set_priority(LOG_PRIORITY);
    debug::message("in", req["t"].as_str(), body.len());
    match req["t"].as_str() {
        Some("pull") => {
            let (room, of) = (req["room"].as_str().unwrap_or_default(), req["of"].as_str().unwrap_or_default());
            if room::may_share(&core, room, &from.to_string(), of) {
                if let Ok(mut file) = tokio::fs::File::open(core.log_path(room, of)).await {
                    let started = Instant::now();
                    file.seek(std::io::SeekFrom::Start(req["from"].as_u64().unwrap_or(0))).await?;
                    let mut data = Vec::new();
                    (&mut file).take(PULL_MAX).read_to_end(&mut data).await?;
                    let first = first_record(&data).filter(|n| *n > data.len() as u64 && *n <= RESPONSE_MAX as u64);
                    if let Some(need) = first {
                        (&mut file).take(need - data.len() as u64).read_to_end(&mut data).await?;
                    }
                    send.write_all(&data).await?;
                    debug::log_transfer("out", from, room, of, data.len(), started);
                }
            }
        }
        Some("follow") => follow(&core, &conn, &mut send, &req).await?,
        Some("speed") => {
            let _ = send.set_priority(Default::default());
            let mut left = req["bytes"].as_u64().unwrap_or(0).min(SPEED_BYTES as u64) as usize;
            let zeros = [0u8; CHUNK];
            while left > 0 {
                let k = left.min(CHUNK);
                send.write_all(&zeros[..k]).await?;
                pace(&core, k, false).await;
                left -= k;
            }
        }
        Some("report") => {
            let c = core.clone();
            let text = tokio::task::spawn_blocking(move || debug::report_text(&c, None)).await?;
            send.write_all(text.as_bytes()).await?;
            debug::event(&core, Instant::now(), format!("{}: sent our report", short(from)));
        }
        Some("hello") => {
            learn_hello(&core, &from, &req);
            send.write_all(&serde_json::to_vec(&hello_message(&core))?).await?;
        }
        Some("rec") => {
            let by = req["by"].as_str().unwrap_or_default();
            let set = req["set"].as_str().unwrap_or_default();
            let here = core.in_room(req["room"].as_str().unwrap_or_default());
            if req["action"] == "start" {
                if here {
                    room::announce(&core, "start", by, req["voice"] == true, set);
                }
                react_to_set(&core, &json!({"id": req["set"], "room": req["room"], "by": req["by"], "start_us": req["start_us"], "voice": req["voice"], "of": req["of"]}));
            } else if req["action"] == "stop" && here {
                let (core, by, set) = (core.clone(), by.to_string(), set.to_string());
                tokio::task::spawn_blocking(move || {
                    room::end_set(&core, true, Some(&set));
                    room::announce(&core, "end", &by, false, &set);
                });
            }
            send.write_all(b"ok").await?;
        }
        Some("live") => {
            let sharing = core.sharing_live();
            let reply = match core.engine.live().filter(|l| sharing && core.in_room(&l.room)) {
                Some(l) => {
                    let newest = l.recent.map(|r| r.last().cloned().unwrap_or_default());
                    json!({"sharing": true, "capture": l.capture, "start_us": l.start_us, "n": newest.clone().map(|(n, _)| n), "set": newest.map(|(_, set)| set)})
                }
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
        Some("speaking") => {
            let on = req["on"] == true;
            core.update_peer(&from, |p| p.speaking = on);
            let _ = core.app.emit("speaking", json!({"from": from.to_string(), "on": on}));
            send.write_all(b"ok").await?;
        }
        Some("draw") if core.in_room(req["room"].as_str().unwrap_or_default()) && core.party.lock().unwrap().on => {
            let _ = core.app.emit("draw", json!({"from": from.to_string(), "stroke": req["stroke"]}));
            send.write_all(b"ok").await?;
        }
        Some("asks") if core.in_room(req["room"].as_str().unwrap_or_default()) => {
            let asks = req["asks"].as_array().cloned().unwrap_or_default();
            core.update_peer(&from, |p| p.asks = asks);
            send.write_all(b"ok").await?;
        }
        Some("answer") => {
            if room::answer_ask(&mut core.asks.lock().unwrap(), req["id"].as_str().unwrap_or_default(), req["approve"] == true) {
                send_asks(&core);
            }
            send.write_all(b"ok").await?;
        }
        Some("remove") if core.in_room(req["room"].as_str().unwrap_or_default()) => {
            core.wake.notify_waiters();
            send.write_all(b"ok").await?;
        }
        Some("join") => send.write_all(&serde_json::to_vec(&json!({"ok": true, "room": room_info(&core)}))?).await?,
        _ => {}
    }
    send.finish()?;
    Ok(())
}

pub fn speed_json(up_bytes: usize, up_took: Duration, down_bytes: usize, down_took: Duration, rtt_ms: u64, path: &str) -> Value {
    let mbps = |bytes: usize, took: Duration| tenth(bytes as f64 / BYTES_PER_MBIT / took.as_secs_f64().max(f64::EPSILON));
    json!({"up_mbps": mbps(up_bytes, up_took), "down_mbps": mbps(down_bytes, down_took), "rtt_ms": rtt_ms, "path": path})
}

pub async fn measure_speed(core: &Arc<Core>, id: &str) -> Option<Value> {
    let conn = core.peers.lock().unwrap().iter().find(|p| p.id.to_string() == id).and_then(|p| p.alive().cloned())?;
    let body = serde_json::to_vec(&json!({"t": "speed", "bytes": SPEED_BYTES, "pad": "0".repeat(SPEED_BYTES)})).ok()?;
    let speed = timeout(SPEED_TIMEOUT, async {
        let (mut send, mut recv) = conn.open_bi().await?;
        debug::message("out", Some("speed"), body.len());
        let started = Instant::now();
        for chunk in body.chunks(CHUNK) {
            send.write_all(chunk).await?;
            pace(core, chunk.len(), false).await;
        }
        send.finish()?;
        let mut buf = vec![0u8; CHUNK];
        let Some(mut got) = recv.read(&mut buf).await? else { anyhow::bail!("no reply") };
        let first = Instant::now();
        while let Some(k) = recv.read(&mut buf).await? {
            got += k;
        }
        let (path, rtt_ms, _) = relay::path_kind(&conn);
        anyhow::Ok(speed_json(body.len(), first - started, got, first.elapsed(), rtt_ms, path))
    })
    .await
    .ok()?
    .ok()?;
    debug::event(core, Instant::now(), format!("{}: speed {speed}", short(id)));
    Some(speed)
}

pub fn pending_asks(core: &Core) -> Vec<Value> {
    core.asks.lock().unwrap().iter().filter(|a| a.answer.is_none()).map(|a| json!({"id": a.id, "name": a.name})).collect()
}

pub fn send_asks(core: &Arc<Core>) {
    let Some(room) = core.room_id() else { return };
    broadcast(core, json!({"t": "asks", "room": room, "asks": pending_asks(core)}));
}

pub fn answer(core: &Arc<Core>, id: &str, approve: bool) {
    let mine = room::answer_ask(&mut core.asks.lock().unwrap(), id, approve);
    if mine {
        send_asks(core);
    } else {
        broadcast(core, json!({"t": "answer", "id": id, "approve": approve}));
    }
}

async fn join(core: &Arc<Core>, from: EndpointId, mut send: SendStream, mut recv: RecvStream) -> Result<Option<room::Step>> {
    let req: Value = serde_json::from_slice(&recv.read_to_end(JOIN_MAX).await?)?;
    let (token, id, name) = (req["token"].as_str().unwrap_or_default(), from.to_string(), crate::clean_name(req["name"].as_str().unwrap_or_default()));
    let Some(room_id) = core.joined_room().ok().filter(|_| req["t"] == "join") else {
        send.finish()?;
        return Ok(None);
    };
    let step = {
        let mut invites = core.invites.lock().unwrap();
        let mut asks = core.asks.lock().unwrap();
        room::ask_step(&mut invites, &mut asks, &room_id, token, &id, &name)
    };
    let reply = match &step {
        room::Step::Refused => json!({"ok": false}),
        room::Step::Wait(changed) => {
            if *changed {
                send_asks(core);
            }
            json!({"wait": true})
        }
        room::Step::Declined => {
            send_asks(core);
            json!({"declined": true})
        }
        room::Step::Admitted => {
            send_asks(core);
            room::admit(core, &room_id, &id);
            tracing::info!("admitted {} to room {room_id}", short(&id));
            json!({"ok": true, "room": room_info(core)})
        }
    };
    send.write_all(&serde_json::to_vec(&reply)?).await?;
    send.finish()?;
    send.stopped().await.ok();
    Ok(Some(step))
}

fn watch_paths(core: &Arc<Core>, conn: &Connection, label: String) {
    let core = core.clone();
    let mut events = conn.path_events();
    let started = Instant::now();
    tokio::spawn(async move {
        use tokio_stream::StreamExt;
        let mut last = "";
        while let Some(event) = events.next().await {
            if let iroh::endpoint::PathEvent::Selected { remote_addr, .. } = &event {
                let now = relay::kind(remote_addr);
                if now != last {
                    last = now;
                    debug::event(&core, started, format!("{label}: {now} path"));
                }
            }
        }
    });
}

async fn admitted(core: &Arc<Core>, from: EndpointId, conn: &Connection) -> bool {
    let mut first = true;
    while let Ok((send, recv)) = conn.accept_bi().await {
        match join(core, from, send, recv).await {
            Ok(None) if first => return false,
            Ok(None | Some(room::Step::Wait(_))) | Err(_) => first = false,
            Ok(Some(room::Step::Admitted)) => return core.has_peer(&from),
            Ok(Some(_)) => return false,
        }
    }
    false
}

pub async fn accept_loop(core: Arc<Core>) {
    while let Some(incoming) = core.ep.accept().await {
        let core = core.clone();
        tokio::spawn(async move {
            let Ok(accepting) = incoming.accept() else { return };
            let Ok(conn) = accepting.await else { return };
            let from = conn.remote_id();
            if test_flag(&core.dir, "offline") {
                conn.close(2u32.into(), b"offline");
                return;
            }
            if !core.has_peer(&from) && !admitted(&core, from, &conn).await {
                conn.close(1u32.into(), b"not a member");
                return;
            }
            if !adopt(&core, &from, &conn).await {
                conn.close(0u32.into(), b"duplicate");
                return;
            }
            run(&core, from, conn, false).await;
        });
    }
}

pub fn first_record(data: &[u8]) -> Option<u64> {
    Some(log::HEADER as u64 + u32::from_le_bytes(data.get(..4)?.try_into().ok()?) as u64)
}

pub fn restored(own_len: u64, more: bool, reported: Option<u64>) -> bool {
    !more && reported.is_some_and(|r| r <= own_len)
}

fn reported(core: &Core, room: &str, of: &str) -> Option<u64> {
    let key = format!("{room}/{of}");
    let peers = core.peers.lock().unwrap();
    let heard: Vec<u64> = peers.iter().filter(|p| p.conn.is_some()).filter_map(|p| p.logs.get(&key).copied()).collect();
    (heard.len() == peers.len() || core.started.elapsed() > RETRY + CONNECT_TIMEOUT).then(|| heard.into_iter().max().unwrap_or(0))
}

async fn pull_log(core: &Core, room: &str, of: &str, conn: &Connection) -> Result<()> {
    let path = core.log_path(room, of);
    let own = of == core.me();
    loop {
        let from = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
        let started = Instant::now();
        let data = request(conn, &json!({"t": "pull", "room": room, "of": of, "from": from})).await?;
        debug::log_transfer("in", conn.remote_id(), room, of, data.len(), started);
        let more = data.len() as u64 >= PULL_MAX;
        {
            let _guard = core.log_lock.lock().unwrap();
            if own && !data.is_empty() {
                core.hold_own_log(room)?;
            }
            if !data.is_empty() {
                log::extend(&path, from, &data)?;
                let _ = core.app.emit("log", room);
            }
            if own && restored(file_len(&path), more, reported(core, room, of)) {
                core.settle(room)?;
            }
        }
        if !more || file_len(&path) == from {
            return Ok(());
        }
    }
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

fn open_part(part: &Path, at: Option<u64>) -> std::io::Result<std::fs::File> {
    let mut file = std::fs::OpenOptions::new().create(true).write(true).truncate(at.is_none()).open(part)?;
    if let Some(at) = at {
        file.set_len(at)?;
        file.seek(SeekFrom::End(0))?;
    }
    Ok(file)
}

fn finish_copy(bad: &Mutex<HashSet<(String, PathBuf)>>, peer: &str, path: &Path, expect: Option<&(u64, String)>, resumed: bool) -> std::io::Result<Option<u64>> {
    let part = part_of(path);
    if let Some((size, sha)) = expect {
        if !data::matches(&part, *size, sha)? {
            let len = file_len(&part);
            let _ = std::fs::remove_file(&part);
            if !resumed {
                bad.lock().unwrap().insert((peer.to_string(), path.to_path_buf()));
            }
            return Ok(Some(len));
        }
    }
    std::fs::rename(&part, path)?;
    Ok(None)
}

async fn receive(core: &Arc<Core>, conn: &Connection, req: Value, expect: HashMap<u32, (u64, String)>) -> Result<u64> {
    let text = |k: &str| req[k].as_str().unwrap_or_default().to_string();
    let (room, owner, capture, series) = (text("room"), text("owner"), text("capture"), text("series"));
    let set = Some(text("set")).filter(|s| valid_id(s));
    let dir = core.series_dir(&room, set.as_deref(), &owner, &capture, &series);
    let (from, at) = (req["from"].as_u64().unwrap_or(1) as u32, req["at"].as_u64().unwrap_or(0));
    let what = debug::what(conn.remote_id(), &req, priority(req["live"] == true, &series));
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&serde_json::to_vec(&req)?).await?;
    send.finish()?;
    let mut current: Option<(u32, bool, tokio::fs::File, Writing, debug::Transfer)> = None;
    let mut resumed = false;
    let mut got = 0u64;
    let mut head = [0u8; FRAME_HEADER];
    let mut buf = Vec::new();
    while recv.read_exact(&mut head).await.is_ok() {
        let n = u32::from_le_bytes(head[..4].try_into()?);
        let len = u32::from_le_bytes(head[4..].try_into()?) as usize;
        if len > CHUNK || core.disk_full() || set.as_ref().is_some_and(|set| room::wants(core, &room, set) == Some(false)) {
            break;
        }
        if current.as_ref().is_none_or(|(k, ..)| *k != n) {
            let path = if n == 0 { dir.join(INIT) } else { dir.join(seg_name(n)) };
            tokio::fs::create_dir_all(&dir).await?;
            if !core.writing.lock().unwrap().insert(path.clone()) {
                break;
            }
            let guard = Writing { core, path };
            let resume = n == from && n > 0 && at > 0 && !resumed;
            resumed |= n > 0;
            let file = open_part(&part_of(&guard.path), resume.then_some(at))?;
            current = Some((n, resume, tokio::fs::File::from_std(file), guard, debug::start("in", &what, n)));
        }
        if len == 0 {
            if let Some((k, resume, file, guard, row)) = current.take() {
                drop(file);
                let (c, peer, path, want) = (core.clone(), conn.remote_id().to_string(), guard.path.clone(), expect.get(&k).cloned());
                let Some(bad) = tokio::task::spawn_blocking(move || finish_copy(&c.bad, &peer, &path, want.as_ref(), resume)).await?? else {
                    row.finish();
                    data::forget(&core.files, &dir);
                    if set.is_none() && k > 0 {
                        data::trim_ring(&dir, k);
                    }
                    continue;
                };
                drop(row);
                let name = format!("{series}/{}", if k == 0 { INIT.to_string() } else { seg_name(k) });
                debug::hash_fail(&owner, &capture, &series, k, bad);
                if let Some(set) = &set {
                    core.append_log(&room, &json!({"k": "file_bad", "room": room, "set": set, "file": format!("{owner}/{capture}/{name}"), "size": bad, "t_us": now_us()}));
                }
                anyhow::bail!("{name} does not match its size and SHA-256");
            }
            continue;
        }
        buf.resize(len, 0);
        recv.read_exact(&mut buf).await?;
        if let Some((_, _, file, _, row)) = current.as_mut() {
            file.write_all(&buf).await?;
            file.flush().await?;
            row.add(len);
        }
        got += len as u64;
    }
    Ok(got)
}

#[derive(Clone, Default, PartialEq)]
pub struct PeerLive {
    pub capture: String,
    pub start_us: Option<i64>,
    pub n: [u32; 2],
    pub set: [Option<String>; 2],
}

fn peer_live(info: &Value) -> Option<PeerLive> {
    let capture = info["capture"].as_str().filter(|c| info["sharing"] == true && capture_id(c))?;
    let n = [0, 1].map(|i| info["n"][i].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0));
    let set = [0, 1].map(|i| info["set"][i].as_str().filter(|s| valid_id(s)).map(String::from));
    Some(PeerLive { capture: capture.to_string(), start_us: info["start_us"].as_i64(), n, set })
}

async fn live_info(core: &Core, id: &EndpointId, conn: &Connection) -> Option<PeerLive> {
    let info: Value = request(conn, &json!({"t": "live"})).await.ok().and_then(|r| serde_json::from_slice(&r).ok()).unwrap_or_default();
    let live = peer_live(&info);
    core.update_peer(id, |p| p.live = live.clone());
    live
}

async fn follow_live(core: Arc<Core>, id: EndpointId, conn: Connection, room: String, which: usize, key: String) {
    let (series, owner) = (SERIES[which], id.to_string());
    let mut capture = String::new();
    while core.in_room(&room) && conn.close_reason().is_none() && core.has_peer(&id) {
        let Some(info) = live_info(&core, &id, &conn).await else { break };
        if info.capture != capture {
            capture = info.capture.clone();
            data::keep_capture(&core.series_dir(&room, None, &owner, &capture, series));
        }
        let (n, set) = (info.n[which], info.set[which].clone());
        if n > 0 && !(which == 0 && relay::relayed(&conn)) {
            let dir = core.series_dir(&room, set.as_deref(), &owner, &capture, series);
            let mut req = json!({"t": "follow", "room": room, "owner": owner, "capture": capture, "series": series, "from": n, "at": 0, "init": !dir.join(INIT).exists(), "live": true});
            if let Some(set) = set {
                req["set"] = json!(set);
            }
            if receive(&core, &conn, req, HashMap::new()).await.is_err() {
                break;
            }
        }
        sleep(RETRY).await;
    }
    core.following.lock().unwrap().remove(&key);
}

async fn fetch(core: Arc<Core>, conn: Connection, req: Value, expect: HashMap<u32, (u64, String)>, key: String) {
    let started = Instant::now();
    let what = format!("{} {} {} from {}", req["set"].as_str().unwrap_or_default(), short(req["owner"].as_str().unwrap_or_default()), req["series"], req["from"]);
    match receive(&core, &conn, req, expect).await {
        Ok(got) => debug::event(&core, started, format!("{what}: fetched {got} B from {}", short(conn.remote_id()))),
        Err(e) => debug::event(&core, started, format!("{what}: fetch from {} stopped: {e}", short(conn.remote_id()))),
    }
    core.following.lock().unwrap().remove(&key);
}

pub fn receiving(core: &Core) -> bool {
    history_fetches(core) > 0
}

fn history_fetches(core: &Core) -> usize {
    core.following.lock().unwrap().iter().filter(|k| k.starts_with(HISTORY_KEY)).count()
}

fn fetch_history(core: &Arc<Core>, peer: &EndpointId, conn: &Connection, room: &str) {
    if core.disk_full() {
        return;
    }
    let via = peer.to_string();
    let haves: Vec<(String, HashMap<String, u64>)> = core.peers.lock().unwrap().iter().filter(|p| p.conn.is_some()).map(|p| (p.id.to_string(), p.sendable())).collect();
    let count = |who: &str, key: &str| haves.iter().find(|(id, _)| id == who).and_then(|(_, h)| h.get(key)).copied().unwrap_or(0);
    let focus = core.focus.lock().unwrap().clone();
    let logs = room::logs(core, room);
    let mut sets = room::sets(core, room, &logs);
    let analysis = crate::jobs::wanted_sets(core, room, &logs, &sets);
    sets.sort_by_key(|s| Some(&s.id) != focus.as_ref());
    let mut waiting = false;
    for set in &sets {
        let here = set.files.iter().any(|f| data::held(core, room, f).0 > 0);
        if !room::auto_receive(set.recording(), here || analysis.contains(&set.id), room::wants(core, room, &set.id)) {
            continue;
        }
        let focused = Some(&set.id) == focus.as_ref();
        for f in &set.files {
            for series in [SERIES[1], SERIES[0]] {
                let room_dir = core.room_dir(room);
                let Some(run) = data::missing(&core.files, &core.writing, &room_dir, f, series) else { continue };
                let dir = data::seg_dir(&room_dir, f, &run.capture, series);
                let bad = |path: PathBuf| core.bad.lock().unwrap().contains(&(via.clone(), path));
                if bad(dir.join(seg_name(run.from))) || (run.init && bad(dir.join(INIT))) {
                    continue;
                }
                let key = room::have_key(&set.id, &f.owner, series);
                let busy = format!("{HISTORY_KEY}{room}/{}/{}/{}/{series}", set.id, f.owner, run.capture);
                if focused && core.following.lock().unwrap().contains(&busy) {
                    waiting = true;
                }
                if waiting && !focused {
                    continue;
                }
                let here = count(&via, &key) > run.index && (f.owner == via || count(&f.owner, &key) <= run.index);
                if !focused && history_fetches(core) >= HISTORY_FETCHES {
                    return;
                }
                if here && core.following.lock().unwrap().insert(busy.clone()) {
                    waiting |= focused;
                    let req = json!({"t": "follow", "room": room, "set": set.id, "owner": f.owner, "capture": run.capture, "series": series, "from": run.from, "at": run.at, "to": run.to, "init": run.init});
                    tokio::spawn(fetch(core.clone(), conn.clone(), req, data::expected(f, &run, series), busy));
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

pub fn ping_time(reply: &[u8]) -> Option<i64> {
    Some(i64::from_le_bytes(reply.try_into().ok()?))
}

async fn measure(core: &Core, id: &EndpointId, conn: &Connection, room: &str, samples: &mut Vec<(i64, i64)>) -> Result<()> {
    let t0 = now_us();
    let reply = request(conn, &json!({"t": "ping"})).await?;
    let t3 = now_us();
    let t1 = ping_time(&reply).ok_or_else(|| anyhow::anyhow!("the other app closed the connection without answering a ping"))?;
    samples.push((t3 - t0, t1 - (t0 + t3) / 2));
    if samples.len() > OFFSET_SAMPLES {
        samples.remove(0);
    }
    let (rtt, offset) = *samples.iter().min().unwrap();
    core.update_peer(id, |p| p.offset_us = Some(offset));
    core.append_log(room, &json!({"k": "offset", "peer": short(id), "t_us": t3, "rtt_us": rtt, "offset_us": offset}));
    Ok(())
}

async fn hello(core: &Arc<Core>, id: &EndpointId, conn: &Connection) -> Option<u32> {
    let reply = request(conn, &hello_message(core)).await.ok()?;
    Some(learn_hello(core, id, &serde_json::from_slice(&reply).ok()?))
}

async fn present_invite(core: &Arc<Core>, id: &EndpointId, conn: &Connection) -> bool {
    loop {
        let pending = core.active_room().and_then(|r| r.invite.filter(|(peer, _)| *peer == id.to_string()).map(|(_, token)| (r.id, token)));
        let Some((pending, token)) = pending else { return true };
        let name = core.name.lock().unwrap().clone();
        let Ok(reply) = request(conn, &json!({"t": "join", "token": token, "name": name})).await else { return false };
        let Ok(reply) = serde_json::from_slice::<Value>(&reply) else { return false };
        if reply["declined"] == true {
            room::forget_room(core, &pending);
            *core.notice.lock().unwrap() = "Your request to join was declined.".into();
            return false;
        }
        if reply["wait"] == true {
            if let Some(r) = core.rooms.lock().unwrap().list.iter_mut().find(|r| r.id == pending) {
                r.waiting = true;
            }
            sleep(LOG_SYNC).await;
            continue;
        }
        if reply["ok"] != true {
            return false;
        }
        let (Some(real), Some(name)) = (reply["room"]["id"].as_str(), reply["room"]["name"].as_str()) else { return false };
        tracing::info!("joined room {real} through {}", short(id));
        room::adopt_room(core, &pending, real, name, &strings::<Vec<String>>(&reply["room"]["members"]));
        room::apply_active(core);
        return true;
    }
}

async fn sync(core: &Arc<Core>, id: &EndpointId, conn: &Connection) -> Result<()> {
    let mut samples = Vec::new();
    let mut tick = 0u32;
    let Some(room) = core.room_id() else { return Ok(()) };
    let mut told = Some(holdings(core));
    loop {
        if test_flag(&core.dir, "offline") {
            anyhow::bail!("offline");
        }
        let held = holdings(core);
        if tick % HELLO_EVERY == HELLO_EVERY - 1 || told.as_ref() != Some(&held) {
            hello(core, id, conn).await;
            told = Some(held);
        }
        if tick < MEASURE_FIRST || tick % MEASURE_EVERY == 0 {
            measure(core, id, conn, &room, &mut samples).await?;
        }
        tick += 1;
        let via = id.to_string();
        let people: Vec<String> = core.active_room().map(|r| r.people()).unwrap_or_default().into_iter().filter(|p| room::member_id(p)).collect();
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
        room::apply_stops(core);
        pull_live(core, id, conn, &room).await;
        fetch_history(core, id, conn, &room);
        tokio::select! {
            _ = sleep(LOG_SYNC) => {}
            _ = core.wake.notified() => {}
        }
    }
}

pub fn backoff(failures: u32, jitter: f64) -> Duration {
    let base = RETRY.saturating_mul(1 << failures.min(RETRY_DOUBLINGS)).min(RETRY_MAX);
    base.mul_f64(1.0 + RETRY_JITTER * (2.0 * jitter.clamp(0.0, 1.0) - 1.0))
}

fn jitter() -> f64 {
    iroh::SecretKey::generate().to_bytes()[0] as f64 / 255.0
}

pub fn preferred(me: &EndpointId, peer: &EndpointId, dialed_by_me: bool) -> bool {
    dialed_by_me == (me < peer)
}

fn alive(core: &Core, id: &EndpointId) -> Option<Connection> {
    core.peers.lock().unwrap().iter().find(|p| p.id == *id).and_then(|p| p.alive().cloned())
}

async fn adopt(core: &Core, id: &EndpointId, conn: &Connection) -> bool {
    if let Some(old) = alive(core, id).filter(|c| c.stable_id() != conn.stable_id()) {
        let kept = |c: &Connection| preferred(&core.ep.id(), id, c.side() == Side::Client);
        let wins = kept(conn) && !kept(&old);
        if !wins && request_within(&old, &json!({"t": "ping"}), PING_WAIT).await.is_ok() {
            return false;
        }
        old.close(0u32.into(), b"replaced");
    }
    core.update_peer(id, |p| p.conn = Some(conn.clone())).is_some()
}

async fn serve(core: Arc<Core>, conn: Connection) {
    while let Ok((send, recv)) = conn.accept_bi().await {
        tokio::spawn(handle(core.clone(), conn.clone(), send, recv));
    }
}

async fn run(core: &Arc<Core>, id: EndpointId, conn: Connection, outgoing: bool) {
    let started = Instant::now();
    let label = format!("{} {}", short(&id), if outgoing { "outgoing" } else { "incoming" });
    debug::event(core, started, format!("{label}: connected"));
    watch_paths(core, &conn, label.clone());
    let serving = tokio::spawn(serve(core.clone(), conn.clone()));
    if !outgoing || present_invite(core, &id, &conn).await {
        match hello(core, &id, &conn).await {
            Some(PROTOCOL) => {
                if let Err(e) = sync(core, &id, &conn).await {
                    debug::event(core, started, format!("{label}: sync stopped: {e}"));
                }
            }
            Some(_) => {
                debug::event(core, started, format!("{label}: protocol differs from ours {PROTOCOL}"));
                sleep(MISMATCH_WAIT).await;
            }
            None => debug::event(core, started, format!("{label}: hello not answered")),
        }
    }
    serving.abort();
    let path = relay::path_kind(&conn).0;
    let by = match conn.close_reason() {
        Some(ConnectionError::ApplicationClosed(c)) => format!(" by the other app: {}", String::from_utf8_lossy(&c.reason)),
        _ => String::new(),
    };
    conn.close(0u32.into(), b"done");
    let lived = started.elapsed();
    core.update_peer(&id, |p| {
        if p.conn.as_ref().is_some_and(|c| c.stable_id() == conn.stable_id()) {
            p.conn = None;
            p.live = None;
            p.speaking = false;
            if lived >= STABLE_FOR {
                p.seen_us = Some(now_us());
            }
            p.failures = if lived < STABLE_FOR { p.failures + 1 } else { 0 };
        }
    });
    debug::event(core, started, format!("{label}: closed{by} on the {path} path after {:.0} s", lived.as_secs_f64()));
}

fn repeated_failures(failed: u32) -> Option<String> {
    (failed > 1).then(|| format!("{failed} connect attempts failed in a row"))
}

fn reached(id: &EndpointId, failed: &mut u32) {
    if let Some(text) = repeated_failures(*failed) {
        tracing::info!("{}: {text}, then reachable", short(id));
    }
    *failed = 0;
}

pub async fn peer_loop(core: Arc<Core>, id: EndpointId) {
    let mut failed = 0u32;
    loop {
        let Some(failures) = core.update_peer(&id, |p| p.failures) else { return };
        sleep(backoff(failures, jitter())).await;
        if let Some(conn) = alive(&core, &id) {
            reached(&id, &mut failed);
            conn.closed().await;
            continue;
        }
        if test_flag(&core.dir, "offline") {
            continue;
        }
        let dialing = Instant::now();
        let conn = match timeout(CONNECT_TIMEOUT, core.ep.connect(id, ALPN)).await {
            Ok(Ok(conn)) => conn,
            other => {
                core.update_peer(&id, |p| p.failures += 1);
                failed += 1;
                if failed == 1 {
                    debug::event(&core, dialing, format!("{} connect failed: {:?}", short(&id), other.map(|r| r.map(|_| ()))));
                }
                continue;
            }
        };
        reached(&id, &mut failed);
        if !adopt(&core, &id, &conn).await {
            conn.close(0u32.into(), b"duplicate");
            continue;
        }
        run(&core, id, conn, true).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::tests::temp;

    #[test]
    fn a_fetched_copy_is_kept_only_when_it_matches_its_record() {
        use sha2::{Digest, Sha256};
        use std::io::Write;
        let dir = temp("arrive");
        let [path, other, init] = ["000001.m4s", "000002.m4s", INIT].map(|name| dir.join(name));
        let (bad, good) = (Mutex::new(HashSet::new()), (4, crate::hex(&Sha256::digest(b"good"))));
        let finish = |path: &Path, body: &[u8], expect: Option<&(u64, String)>, resumed: bool| {
            std::fs::write(part_of(path), body).unwrap();
            (finish_copy(&bad, "a", path, expect, resumed).unwrap(), std::fs::read(path).ok(), part_of(path).exists())
        };
        std::fs::write(part_of(&path), b"0123456789").unwrap();
        open_part(&part_of(&path), Some(4)).unwrap().write_all(b"abcd").unwrap();
        assert_eq!(std::fs::read(part_of(&path)).unwrap(), b"0123abcd");
        assert_eq!(finish(&path, b"good", Some(&good), false), (None, Some(b"good".to_vec()), false));
        assert_eq!(finish(&other, b"evil", Some(&good), true), (Some(4), None, false));
        assert_eq!(finish(&other, b"live", None, false), (None, Some(b"live".to_vec()), false));
        assert_eq!(finish(&init, b"tini", Some(&good), false), (Some(4), None, false));
        assert_eq!(*bad.lock().unwrap(), HashSet::from([("a".to_string(), init)]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_live_reply_names_the_recording_of_each_series() {
        let live = |info: Value| peer_live(&info).map(|l| (l.capture, l.start_us, l.n, l.set));
        assert_eq!(live(json!({"sharing": true, "capture": "1700", "start_us": 9, "n": [7, 5], "set": ["s1", null]})), Some(("1700".into(), Some(9), [7, 5], [Some("s1".into()), None])));
        assert_eq!(live(json!({"sharing": true, "capture": "1700", "n": [3], "set": ["../x", 5]})), Some(("1700".into(), None, [3, 0], [None, None])));
        assert_eq!((live(json!({"sharing": false, "capture": "1700"})), live(json!({"sharing": true, "capture": "../x"}))), (None, None));
    }

    #[test]
    fn speed_is_bytes_over_time_in_mbit_per_second_with_the_path_at_the_end() {
        let speed = speed_json(2_500_000, Duration::from_secs(2), 1_250_000, Duration::from_millis(500), 19, "direct");
        assert_eq!(speed, json!({"up_mbps": 10.0, "down_mbps": 20.0, "rtt_ms": 19, "path": "direct"}));
        assert_eq!(speed_json(1, Duration::ZERO, 0, Duration::ZERO, 0, "relay")["down_mbps"], 0.0);
    }

    #[test]
    fn only_a_run_of_failed_connects_is_reported_once() {
        assert_eq!(repeated_failures(0), None);
        assert_eq!(repeated_failures(1), None);
        assert_eq!(repeated_failures(5).as_deref(), Some("5 connect attempts failed in a row"));
    }

    #[test]
    fn a_ping_reply_is_eight_bytes_and_an_empty_one_is_no_answer() {
        assert_eq!(ping_time(&42i64.to_le_bytes()), Some(42));
        assert_eq!(ping_time(&[]), None);
        assert_eq!(ping_time(&[1, 2, 3]), None);
    }

    #[test]
    fn a_pull_holds_at_least_one_whole_record() {
        let record = log::encode(&json!({"text": "x".repeat(PULL_MAX as usize)}));
        assert_eq!(first_record(&record[..PULL_MAX as usize]), Some(record.len() as u64));
        assert_eq!(first_record(&record[..3]), None);
    }

    #[test]
    fn own_log_is_restored_only_when_no_member_holds_more() {
        assert!(restored(0, false, Some(0)));
        assert!(!restored(0, false, Some(10)));
        assert!(!restored(PULL_MAX, true, Some(PULL_MAX)));
        assert!(restored(500, false, Some(500)));
        assert!(!restored(500, false, Some(501)));
        assert!(!restored(500, false, None));
    }

    #[test]
    fn redials_back_off_with_jitter_up_to_a_limit() {
        assert_eq!(backoff(0, 0.5), RETRY);
        assert_eq!(backoff(1, 0.5), RETRY * 2);
        assert_eq!(backoff(3, 0.5), RETRY * 8);
        assert_eq!(backoff(9, 0.5), RETRY_MAX);
        assert_eq!(backoff(0, 0.0), RETRY.mul_f64(1.0 - RETRY_JITTER));
        assert_eq!(backoff(9, 1.0), RETRY_MAX.mul_f64(1.0 + RETRY_JITTER));
    }

    #[test]
    fn the_connection_dialed_by_the_lower_id_is_kept_on_both_sides() {
        let (a, b) = (iroh::SecretKey::generate().public(), iroh::SecretKey::generate().public());
        for dialed_by_a in [true, false] {
            assert_eq!(preferred(&a, &b, dialed_by_a), preferred(&b, &a, !dialed_by_a));
            assert_eq!(preferred(&a, &b, dialed_by_a), dialed_by_a == (a < b));
        }
    }

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

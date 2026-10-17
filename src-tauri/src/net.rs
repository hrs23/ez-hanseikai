use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::EndpointId;
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::time::{sleep, timeout};

use crate::{debug, log, now_us, short, Core, ALPN, PROTOCOL, VERSION};

const CHUNK: u64 = 16 << 20;

async fn request(conn: &Connection, req: &Value) -> Result<Vec<u8>> {
    timeout(Duration::from_secs(10), async {
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(&serde_json::to_vec(req)?).await?;
        send.finish()?;
        Ok(recv.read_to_end(32 << 20).await?)
    })
    .await?
}

pub async fn send_control(core: &Arc<Core>, cmd: &str, wall_us: i64) {
    for conn in core.connections() {
        let _ = request(&conn, &json!({"t": "ctl", "cmd": cmd, "wall_us": wall_us})).await;
    }
}

async fn handle(core: Arc<Core>, from: EndpointId, mut send: SendStream, mut recv: RecvStream) -> Result<()> {
    let req: Value = serde_json::from_slice(&recv.read_to_end(4096).await?)?;
    match req["t"].as_str() {
        Some("pull") => {
            let name = req["file"].as_str().unwrap_or_default();
            let valid = name == "log.bin" || (name.starts_with("rec-") && name.ends_with(".mp4") && !name.contains(['/', '\\']));
            if valid {
                if let Ok(mut file) = tokio::fs::File::open(core.dir.join(name)).await {
                    file.seek(std::io::SeekFrom::Start(req["from"].as_u64().unwrap_or(0))).await?;
                    let mut data = Vec::new();
                    file.take(CHUNK).read_to_end(&mut data).await?;
                    send.write_all(&data).await?;
                }
            }
        }
        Some("hello") => {
            let version = req["app"].as_str().unwrap_or("unknown").to_string();
            let proto = req["proto"].as_u64().unwrap_or(0) as u32;
            core.update_peer(&from, |p| p.info = Some((version, proto)));
            send.write_all(&serde_json::to_vec(&json!({"app": VERSION, "proto": PROTOCOL}))?).await?;
        }
        Some("ping") => send.write_all(&now_us().to_le_bytes()).await?,
        Some("ctl") => {
            let wall = req["wall_us"].as_i64().unwrap_or(0);
            let _ = core.app.emit("control", json!({"cmd": req["cmd"], "wall_us": wall, "from": from.to_string()}));
            send.write_all(b"ok").await?;
        }
        _ => {}
    }
    send.finish()?;
    Ok(())
}

fn watch_paths(core: &Arc<Core>, conn: &Connection, label: String) {
    let core = core.clone();
    let mut events = conn.path_events();
    let started = std::time::Instant::now();
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
                conn.close(1u32.into(), b"unknown peer");
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

async fn pull(core: &Core, id: &EndpointId, conn: &Connection, name: &str) -> Result<()> {
    let dir = core.peer_dir(id);
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(name);
    let from = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);
    let data = request(conn, &json!({"t": "pull", "file": name, "from": from})).await?;
    if !data.is_empty() {
        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new().create(true).append(true).open(&path).await?;
        file.write_all(&data).await?;
    }
    Ok(())
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

async fn sync(core: &Arc<Core>, id: &EndpointId, conn: &Connection) -> Result<()> {
    let mut samples = Vec::new();
    let mut tick = 0u32;
    loop {
        if tick < 5 || tick % 30 == 0 {
            measure(core, id, conn, &mut samples).await?;
        }
        tick += 1;
        pull(core, id, conn, "log.bin").await?;
        let peer_log = log::read(&core.peer_dir(id).join("log.bin"));
        let latest = peer_log.iter().rev().find(|r| r["k"] == "rec");
        if let Some(name) = latest.and_then(|r| r["name"].as_str()) {
            pull(core, id, conn, name).await?;
        }
        sleep(Duration::from_secs(1)).await;
    }
}

pub async fn dial_loop(core: Arc<Core>, id: EndpointId) {
    loop {
        sleep(Duration::from_secs(2)).await;
        if !core.has_peer(&id) {
            return;
        }
        let dialing = std::time::Instant::now();
        let conn = match timeout(Duration::from_secs(15), core.ep.connect(id, ALPN)).await {
            Ok(Ok(conn)) => conn,
            other => {
                debug::event(&core, dialing, format!("{} connect failed: {:?}", short(&id), other.map(|r| r.map(|_| ()))));
                continue;
            }
        };
        debug::event(&core, dialing, format!("{} connected", short(&id)));
        core.update_peer(&id, |p| p.conn = Some(conn.clone()));
        watch_paths(&core, &conn, format!("{} outgoing", short(&id)));
        let hello = request(&conn, &json!({"t": "hello", "app": VERSION, "proto": PROTOCOL})).await;
        let mut compatible = true;
        if let Ok(reply) = hello.and_then(|r| Ok(serde_json::from_slice::<Value>(&r)?)) {
            let proto = reply["proto"].as_u64().unwrap_or(0) as u32;
            let version = reply["app"].as_str().unwrap_or("unknown").to_string();
            core.update_peer(&id, |p| p.info = Some((version, proto)));
            if proto != PROTOCOL {
                debug::event(&core, dialing, format!("{} protocol {proto} differs from ours {PROTOCOL}", short(&id)));
                sleep(Duration::from_secs(10)).await;
                compatible = false;
            }
        }
        if compatible {
            let _ = sync(&core, &id, &conn).await;
        }
        core.update_peer(&id, |p| p.conn = None);
        conn.close(0u32.into(), b"done");
    }
}

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use iroh::endpoint::{Connection, RecvStream, SendStream};
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::time::{sleep, timeout};

use crate::{debug, log, now_us, Core, ALPN};

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

pub async fn send_control(core: &Arc<Core>, cmd: &str, wall_us: i64) -> Result<()> {
    let conn = core.conn.lock().unwrap().clone();
    let Some(conn) = conn else { bail!("no peer connected") };
    request(&conn, &json!({"t": "ctl", "cmd": cmd, "wall_us": wall_us})).await?;
    Ok(())
}

async fn handle(core: Arc<Core>, mut send: SendStream, mut recv: RecvStream) -> Result<()> {
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
        Some("ping") => send.write_all(&now_us().to_le_bytes()).await?,
        Some("ctl") => {
            let wall = req["wall_us"].as_i64().unwrap_or(0);
            let _ = core.app.emit("control", json!({"cmd": req["cmd"], "wall_us": wall}));
            send.write_all(b"ok").await?;
        }
        _ => {}
    }
    send.finish()?;
    Ok(())
}

fn watch_paths(core: &Arc<Core>, conn: &Connection, label: &'static str) {
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
            if Some(conn.remote_id()) != *core.peer.lock().unwrap() {
                conn.close(1u32.into(), b"unknown peer");
                return;
            }
            *core.incoming.lock().unwrap() = Some(conn.clone());
            watch_paths(&core, &conn, "incoming");
            while let Ok((send, recv)) = conn.accept_bi().await {
                tokio::spawn(handle(core.clone(), send, recv));
            }
        });
    }
}

async fn pull(core: &Core, conn: &Connection, name: &str) -> Result<()> {
    let dir = core.dir.join("peer");
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

async fn measure(core: &Core, conn: &Connection, samples: &mut Vec<(i64, i64)>) -> Result<()> {
    let t0 = now_us();
    let reply = request(conn, &json!({"t": "ping"})).await?;
    let t3 = now_us();
    let t1 = i64::from_le_bytes(reply[..8].try_into()?);
    samples.push((t3 - t0, t1 - (t0 + t3) / 2));
    if samples.len() > 8 {
        samples.remove(0);
    }
    let (rtt, offset) = *samples.iter().min().unwrap();
    *core.offset_us.lock().unwrap() = Some(offset);
    core.append_log(&json!({"k": "offset", "t_us": t3, "rtt_us": rtt, "offset_us": offset}));
    Ok(())
}

async fn sync(core: &Arc<Core>, conn: &Connection) -> Result<()> {
    let mut samples = Vec::new();
    let mut tick = 0u32;
    loop {
        if tick < 5 || tick % 30 == 0 {
            measure(core, conn, &mut samples).await?;
        }
        tick += 1;
        pull(core, conn, "log.bin").await?;
        let peer_log = log::read(&core.dir.join("peer").join("log.bin"));
        let latest = peer_log.iter().rev().find(|r| r["k"] == "rec");
        if let Some(name) = latest.and_then(|r| r["name"].as_str()) {
            pull(core, conn, name).await?;
        }
        sleep(Duration::from_secs(1)).await;
    }
}

pub async fn dial_loop(core: Arc<Core>) {
    loop {
        sleep(Duration::from_secs(2)).await;
        let Some(peer) = *core.peer.lock().unwrap() else { continue };
        let dialing = std::time::Instant::now();
        let conn = match timeout(Duration::from_secs(15), core.ep.connect(peer, ALPN)).await {
            Ok(Ok(conn)) => conn,
            other => {
                debug::event(&core, dialing, format!("connect failed: {:?}", other.map(|r| r.map(|_| ()))));
                continue;
            }
        };
        debug::event(&core, dialing, "connected".into());
        *core.conn.lock().unwrap() = Some(conn.clone());
        watch_paths(&core, &conn, "outgoing");
        let _ = sync(&core, &conn).await;
        *core.conn.lock().unwrap() = None;
    }
}

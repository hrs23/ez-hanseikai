#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod check;
mod debug;
mod log;
mod net;
mod record;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iroh::endpoint::{presets, Connection};
use iroh::{Endpoint, EndpointId, SecretKey, Watcher};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

pub const ALPN: &[u8] = b"ez-hanseikai/0";
pub const PROTOCOL: u32 = 1;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn now_us() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as i64
}

pub struct Core {
    pub dir: PathBuf,
    pub ep: Endpoint,
    pub app: AppHandle,
    pub peer: Mutex<Option<EndpointId>>,
    pub conn: Mutex<Option<Connection>>,
    pub incoming: Mutex<Option<Connection>>,
    pub ffmpeg_status: Mutex<String>,
    pub peer_info: Mutex<Option<(String, u32)>>,
    pub events: Mutex<Vec<String>>,
    pub started: Instant,
    pub offset_us: Mutex<Option<i64>>,
    pub recorder: Mutex<Option<record::Recorder>>,
    log_lock: Mutex<()>,
}

impl Core {
    pub fn append_log(&self, record: &Value) {
        let _guard = self.log_lock.lock().unwrap();
        let _ = log::append(&self.dir.join("log.bin"), record);
    }

    fn own_log(&self) -> Vec<Value> {
        log::read(&self.dir.join("log.bin"))
    }

    fn peer_log(&self) -> Vec<Value> {
        log::read(&self.dir.join("peer").join("log.bin"))
    }
}

fn latest_rec(records: &[Value]) -> Option<(String, i64)> {
    let r = records.iter().rev().find(|r| r["k"] == "rec")?;
    Some((r["name"].as_str()?.to_string(), r["start_us"].as_i64()?))
}

fn size(path: PathBuf) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

fn set_peer_id(core: &Core, id: &str) -> Result<(), String> {
    let peer: EndpointId = id.trim().parse().map_err(|e| format!("{e}"))?;
    std::fs::write(core.dir.join("peer.txt"), peer.to_string()).map_err(|e| e.to_string())?;
    *core.peer.lock().unwrap() = Some(peer);
    Ok(())
}

fn start_recording_now(core: &Core, audio_pid: u32) -> Result<(), String> {
    let mut slot = core.recorder.lock().unwrap();
    if slot.is_some() {
        return Err("already recording".into());
    }
    *slot = Some(record::Recorder::start(&core.dir, audio_pid).map_err(|e| e.to_string())?);
    Ok(())
}

fn stop_recording_now(core: &Core) {
    if let Some(recorder) = core.recorder.lock().unwrap().take() {
        recorder.stop();
    }
}

fn pid_of(name: &str) -> Option<u32> {
    processes().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, pid)| pid)
}

fn processes() -> Vec<(String, u32)> {
    use std::os::windows::process::CommandExt;
    let Ok(out) = std::process::Command::new("tasklist").args(["/FO", "CSV", "/NH"]).creation_flags(0x08000000).output() else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut cells = line.trim().trim_matches('"').split("\",\"");
            let name = cells.next()?.to_string();
            let pid = cells.next()?.parse().ok()?;
            Some((name, pid))
        })
        .collect()
}

#[tauri::command]
async fn list_processes() -> Vec<String> {
    let mut names: Vec<String> = processes().into_iter().map(|(name, _)| name).collect();
    names.sort_by_key(|n| (!n.to_lowercase().contains("apex"), n.to_lowercase()));
    names.dedup();
    names
}

#[tauri::command]
async fn state(core: State<'_, Arc<Core>>) -> Result<Value, String> {
    let dead = core.recorder.lock().unwrap().as_mut().is_some_and(|r| r.is_dead());
    if dead {
        tracing::warn!("recording stopped on its own");
        stop_recording_now(&core);
    }
    let own = core.own_log();
    let theirs = core.peer_log();
    let mine = latest_rec(&own).map(|(name, start_us)| {
        json!({"path": core.dir.join(&name), "start_us": start_us, "size": size(core.dir.join(&name))})
    });
    let peer = latest_rec(&theirs).map(|(name, start_us)| {
        let path = core.dir.join("peer").join(&name);
        json!({"path": path, "start_us": start_us, "size": size(path.clone())})
    });
    let mut notes: Vec<Value> = Vec::new();
    for (who, records) in [("me", &own), ("peer", &theirs)] {
        for r in records.iter().filter(|r| r["k"] == "note") {
            notes.push(json!({"who": who, "t_us": r["t_us"], "text": r["text"]}));
        }
    }
    notes.sort_by_key(|n| n["t_us"].as_i64());
    let peer_info = core.peer_info.lock().unwrap().clone();
    let peer_message = peer_info.as_ref().filter(|(_, proto)| *proto != PROTOCOL).map(|(version, _)| format!("peer has version {version}, update needed"));
    Ok(json!({
        "version": VERSION,
        "ffmpeg": core.ffmpeg_status.lock().unwrap().clone(),
        "peer_version": peer_info.map(|(version, _)| version),
        "peer_message": peer_message,
        "id": core.ep.id().to_string(),
        "peer": core.peer.lock().unwrap().map(|p| p.to_string()),
        "connected": core.conn.lock().unwrap().is_some(),
        "recording": core.recorder.lock().unwrap().is_some(),
        "offset_us": *core.offset_us.lock().unwrap(),
        "mine": mine,
        "peer_rec": peer,
        "notes": notes,
    }))
}

async fn warm(core: &Core) {
    let wait = Duration::from_secs(6);
    let _ = tokio::time::timeout(wait, core.ep.online()).await;
    let _ = tokio::time::timeout(wait, core.ep.net_report().initialized()).await;
}

#[tauri::command]
async fn check(core: State<'_, Arc<Core>>, audio_process: String) -> Result<Vec<Value>, String> {
    let core = core.inner().clone();
    warm(&core).await;
    tauri::async_runtime::spawn_blocking(move || check::run(&core, pid_of(&audio_process)))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn debug_text(core: State<'_, Arc<Core>>) -> Result<String, String> {
    Ok(debug::text(&core))
}

#[tauri::command]
async fn save_report(core: State<'_, Arc<Core>>, include_addresses: bool, audio_process: String) -> Result<String, String> {
    let core = core.inner().clone();
    warm(&core).await;
    tauri::async_runtime::spawn_blocking(move || {
        let lines = check::run(&core, pid_of(&audio_process));
        debug::report(&core, include_addresses, &check::text(&lines)).map(|p| p.display().to_string()).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn start_recording(core: State<'_, Arc<Core>>, audio_process: String) -> Result<(), String> {
    start_recording_now(&core, pid_of(&audio_process).unwrap_or(0))
}

#[tauri::command]
async fn stop_recording(core: State<'_, Arc<Core>>) -> Result<(), String> {
    stop_recording_now(&core);
    Ok(())
}

#[tauri::command]
fn set_peer(core: State<Arc<Core>>, id: String) -> Result<(), String> {
    set_peer_id(&core, &id)
}

#[tauri::command]
fn add_note(core: State<Arc<Core>>, text: String) {
    core.append_log(&json!({"k": "note", "t_us": now_us(), "text": text}));
}

#[tauri::command]
async fn control(core: State<'_, Arc<Core>>, cmd: String, wall_us: i64) -> Result<(), String> {
    net::send_control(&core, &cmd, wall_us).await.map_err(|e| e.to_string())
}

async fn init(app: AppHandle) -> Result<Arc<Core>, Box<dyn std::error::Error>> {
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(dir.join("peer"))?;
    debug::init(&dir);
    record::set_data_dir(dir.clone());
    log::repair(&dir.join("log.bin"));
    log::repair(&dir.join("peer").join("log.bin"));
    let key_path = dir.join("key.bin");
    let key = match std::fs::read(&key_path).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()) {
        Some(bytes) => SecretKey::from_bytes(&bytes),
        None => {
            let key = SecretKey::generate();
            std::fs::write(&key_path, key.to_bytes())?;
            key
        }
    };
    let ep = Endpoint::builder(presets::N0).secret_key(key).alpns(vec![ALPN.to_vec()]).bind().await?;
    std::fs::write(dir.join("id.txt"), ep.id().to_string())?;
    let peer = std::fs::read_to_string(dir.join("peer.txt")).ok().and_then(|s| s.trim().parse().ok());
    Ok(Arc::new(Core {
        dir,
        ep,
        app,
        peer: Mutex::new(peer),
        conn: Mutex::new(None),
        incoming: Mutex::new(None),
        ffmpeg_status: Mutex::new(String::new()),
        peer_info: Mutex::new(None),
        events: Mutex::new(Vec::new()),
        started: Instant::now(),
        offset_us: Mutex::new(None),
        recorder: Mutex::new(None),
        log_lock: Mutex::new(()),
    }))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let core = tauri::async_runtime::block_on(init(app.handle().clone()))?;
            app.manage(core.clone());
            tauri::async_runtime::spawn(net::accept_loop(core.clone()));
            tauri::async_runtime::spawn(net::dial_loop(core.clone()));
            let setup_core = core.clone();
            std::thread::spawn(move || {
                if !record::ffmpeg_present() {
                    *setup_core.ffmpeg_status.lock().unwrap() = "downloading ffmpeg...".into();
                    tracing::info!("downloading ffmpeg");
                    let result = record::download_ffmpeg();
                    *setup_core.ffmpeg_status.lock().unwrap() = match &result {
                        Ok(()) => String::new(),
                        Err(e) => format!("ffmpeg download failed: {e}"),
                    };
                    if let Err(e) = result {
                        tracing::warn!("ffmpeg download failed: {e}");
                    }
                }
                record::encoder_results();
            });
            let args: Vec<String> = std::env::args().collect();
            let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
            if let Some(id) = arg("--peer") {
                set_peer_id(&core, &id)?;
            }
            if let Some(seconds) = arg("--record").and_then(|s| s.parse::<u64>().ok()) {
                let pid = arg("--audio-pid").and_then(|s| s.parse().ok()).unwrap_or(0);
                std::thread::spawn(move || {
                    if let Err(e) = start_recording_now(&core, pid) {
                        eprintln!("record failed: {e}");
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(seconds));
                    stop_recording_now(&core);
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![state, check, list_processes, debug_text, save_report, start_recording, stop_recording, set_peer, add_note, control])
        .run(tauri::generate_context!())
        .expect("failed to run");
}

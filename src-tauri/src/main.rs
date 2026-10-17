#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod check;
mod data;
mod debug;
mod log;
mod net;
mod record;
mod shell;

use std::path::PathBuf;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iroh::endpoint::{presets, Connection};
use iroh::{Endpoint, EndpointId, SecretKey, Watcher};
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

pub const ALPN: &[u8] = b"ez-hanseikai/0";
pub const PROTOCOL: u32 = 1;
const NOTE_MAX: usize = 500;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn now_us() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as i64
}

pub fn short(id: &EndpointId) -> String {
    id.to_string().chars().take(10).collect()
}

pub fn clean_name(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(24).collect::<String>().trim().to_string()
}

pub struct Peer {
    pub id: EndpointId,
    pub name: String,
    pub conn: Option<Connection>,
    pub incoming: Option<Connection>,
    pub offset_us: Option<i64>,
    pub failures: u32,
    pub announce: bool,
    pub info: Option<(String, u32)>,
}

pub struct Core {
    pub dir: PathBuf,
    pub ep: Endpoint,
    pub app: AppHandle,
    pub peers: Mutex<Vec<Peer>>,
    pub name: Mutex<String>,
    pub share: AtomicBool,
    pub limit_mbps: Mutex<Option<f64>>,
    pub last_members: Mutex<Vec<String>>,
    pub invites: Mutex<Vec<String>>,
    pub audio_process: Mutex<String>,
    pub keep_days: Mutex<Option<u32>>,
    pub ready: AtomicBool,
    pub removed: Mutex<HashSet<String>>,
    pub pending_join: Mutex<HashMap<EndpointId, String>>,
    pub following: Mutex<HashSet<String>>,
    pub recording_name: Mutex<Option<String>>,
    pub recovery: Mutex<Option<String>>,
    pub notice: Mutex<String>,
    pub free_bytes: AtomicU64,
    pub previous_version: String,
    pub ffmpeg_status: Mutex<String>,
    pub events: Mutex<Vec<String>>,
    pub started: Instant,
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

    pub fn save_settings(&self) {
        let value = json!({
            "name": *self.name.lock().unwrap(),
            "share": self.share.load(Ordering::Relaxed),
            "limit_mbps": *self.limit_mbps.lock().unwrap(),
            "last_members": *self.last_members.lock().unwrap(),
            "removed": *self.removed.lock().unwrap(),
            "audio_process": *self.audio_process.lock().unwrap(),
            "keep_days": *self.keep_days.lock().unwrap(),
        });
        let _ = std::fs::write(self.dir.join("settings.json"), value.to_string());
    }

    pub fn peer_dir(&self, id: &EndpointId) -> PathBuf {
        self.dir.join("peer").join(short(id))
    }

    pub fn has_peer(&self, id: &EndpointId) -> bool {
        self.peers.lock().unwrap().iter().any(|p| p.id == *id)
    }

    pub fn update_peer<R>(&self, id: &EndpointId, f: impl FnOnce(&mut Peer) -> R) -> Option<R> {
        self.peers.lock().unwrap().iter_mut().find(|p| p.id == *id).map(f)
    }

    pub fn connections(&self) -> Vec<Connection> {
        self.peers.lock().unwrap().iter().filter_map(|p| p.conn.clone()).collect()
    }

    fn save_peers(&self) {
        let ids: Vec<String> = self.peers.lock().unwrap().iter().map(|p| p.id.to_string()).collect();
        let _ = std::fs::write(self.dir.join("peers.txt"), ids.join("\n"));
    }
}

fn path_kind(conn: &Connection) -> (&'static str, u64) {
    for path in conn.paths().iter() {
        if path.is_selected() {
            return (if path.is_ip() { "direct" } else { "relay" }, path.rtt().as_millis() as u64);
        }
    }
    ("none", 0)
}

fn latest_rec(records: &[Value]) -> Option<(String, i64)> {
    let r = records.iter().rev().find(|r| r["k"] == "rec")?;
    Some((r["name"].as_str()?.to_string(), r["start_us"].as_i64()?))
}

fn latest_notes(records: &[Value], from: &str, who: &str) -> Vec<Value> {
    let mut notes: Vec<Value> = Vec::new();
    for r in records {
        let id = r["id"].as_i64().or_else(|| r["t_us"].as_i64()).unwrap_or(0);
        let existing = notes.iter().position(|n| n["id"] == id);
        match r["k"].as_str() {
            Some("note") => notes.push(json!({"id": id, "from": from, "who": who, "t_us": r["t_us"], "at_us": r["at_us"].as_i64().unwrap_or(id), "text": r["text"], "deleted": false})),
            Some("note_edit") => { if let Some(i) = existing { notes[i]["text"] = r["text"].clone(); } }
            Some("note_del") => { if let Some(i) = existing { notes[i]["deleted"] = json!(true); } }
            Some("note_restore") => { if let Some(i) = existing { notes[i]["deleted"] = json!(false); } }
            _ => {}
        }
    }
    notes.retain(|n| n["deleted"] == false);
    notes
}

fn size(path: PathBuf) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

pub fn add_peer_id(core: &Arc<Core>, id: &str) -> Result<(), String> {
    let id: EndpointId = id.trim().parse().map_err(|_| "That is not a valid ID. Copy it again from the other person.".to_string())?;
    if id == core.ep.id() {
        return Err("that is your own ID".into());
    }
    if core.has_peer(&id) {
        return Ok(());
    }
    let announce = core.ready.load(Ordering::Relaxed);
    core.peers.lock().unwrap().push(Peer { id, name: String::new(), conn: None, incoming: None, offset_us: None, failures: 0, announce, info: None });
    core.save_peers();
    tauri::async_runtime::spawn(net::dial_loop(core.clone(), id));
    Ok(())
}

fn start_recording_now(core: &Core, audio_pid: u32, audio_name: &str) -> Result<(), String> {
    let mut slot = core.recorder.lock().unwrap();
    if slot.is_some() {
        return Err("already recording".into());
    }
    let recorder = record::Recorder::start(&core.dir, audio_pid, audio_name).map_err(|e| e.to_string())?;
    *core.recording_name.lock().unwrap() = Some(recorder.name.clone());
    *slot = Some(recorder);
    Ok(())
}

fn stop_recording_now(core: &Core) {
    if let Some(recorder) = core.recorder.lock().unwrap().take() {
        recorder.stop();
        *core.recording_name.lock().unwrap() = None;
        core.append_log(&json!({"k": "rec_end", "t_us": now_us()}));
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
    let free = core.free_bytes.load(Ordering::Relaxed);
    if free > 0 && free < (1 << 30) && core.recorder.lock().unwrap().is_some() {
        stop_recording_now(&core);
        *core.notice.lock().unwrap() = "Recording stopped: the disk is almost full.".into();
    }
    let dead = core.recorder.lock().unwrap().as_mut().is_some_and(|r| r.is_dead());
    if dead {
        tracing::warn!("recording stopped on its own");
        stop_recording_now(&core);
    }
    let own = core.own_log();
    let mine = latest_rec(&own).map(|(name, start_us)| {
        json!({"path": core.dir.join(&name), "start_us": start_us, "size": size(core.dir.join(&name))})
    });
    let mut notes: Vec<Value> = latest_notes(&own, "me", "me");
    let snapshot: Vec<(EndpointId, String, Option<Connection>, Option<Connection>, Option<i64>, Option<(String, u32)>, u32)> = core
        .peers
        .lock()
        .unwrap()
        .iter()
        .map(|p| (p.id, p.name.clone(), p.conn.clone(), p.incoming.clone(), p.offset_us, p.info.clone(), p.failures))
        .collect();
    let mut peers = Vec::new();
    for (id, name, conn, incoming, offset_us, info, failures) in snapshot {
        let theirs = log::read(&core.peer_dir(&id).join("log.bin"));
        let rec = latest_rec(&theirs).map(|(name, start_us)| {
            let path = core.peer_dir(&id).join(&name);
            json!({"path": path, "start_us": start_us, "size": size(path.clone())})
        });
        let who = if name.is_empty() { short(&id) } else { format!("{name} ({})", short(&id)) };
        notes.extend(latest_notes(&theirs, &id.to_string(), &who));
        let last = |kind: &str| theirs.iter().rposition(|r| r["k"] == kind);
        let recording = match (last("rec"), last("rec_end")) {
            (Some(start), Some(end)) => start > end,
            (Some(_), None) => true,
            _ => false,
        };
        let alive = conn.iter().chain(incoming.iter()).find(|c| c.close_reason().is_none());
        let message = info.as_ref().filter(|(_, proto)| *proto != PROTOCOL).map(|(version, proto)| {
            format!("peer has version {version} (protocol {proto}, ours {PROTOCOL}), update needed")
        });
        peers.push(json!({
            "id": id.to_string(),
            "short": short(&id),
            "name": name,
            "connected": alive.is_some(),
            "path": alive.map(|c| path_kind(c).0),
            "rtt_ms": alive.map(|c| path_kind(c).1),
            "failures": failures,
            "offset_us": offset_us,
            "version": info.map(|(version, _)| version),
            "message": message,
            "rec": rec,
            "recording": recording,
        }));
    }
    notes.sort_by_key(|n| n["t_us"].as_i64());
    Ok(json!({
        "version": VERSION,
        "ffmpeg": core.ffmpeg_status.lock().unwrap().clone(),
        "id": core.ep.id().to_string(),
        "name": core.name.lock().unwrap().clone(),
        "share": core.share.load(Ordering::Relaxed),
        "audio_process": core.audio_process.lock().unwrap().clone(),
        "keep_days": *core.keep_days.lock().unwrap(),
        "invites": core.invites.lock().unwrap().len(),
        "limit_mbps": *core.limit_mbps.lock().unwrap(),
        "can_rejoin": !core.last_members.lock().unwrap().is_empty(),
        "recovery": core.recovery.lock().unwrap().clone(),
        "notice": core.notice.lock().unwrap().clone(),
        "free_bytes": free,
        "previous_version": core.previous_version,
        "recording": core.recorder.lock().unwrap().is_some(),
        "mine": mine,
        "peers": peers,
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

pub fn start_recording_named(core: &Core, audio_process: &str) -> Result<(), String> {
    if !record::ffmpeg_present() {
        return Err("The video tool is still downloading. Wait a minute and try again.".into());
    }
    let found = processes().into_iter().find(|(name, _)| if audio_process.is_empty() { name.to_lowercase().starts_with("r5apex") } else { name.eq_ignore_ascii_case(&audio_process) });
    let (name, pid) = found.unwrap_or_default();
    start_recording_now(core, pid, &name)
}

#[tauri::command]
async fn start_recording(core: State<'_, Arc<Core>>) -> Result<(), String> {
    let name = core.audio_process.lock().unwrap().clone();
    start_recording_named(&core, &name)
}

#[tauri::command]
async fn stop_recording(core: State<'_, Arc<Core>>) -> Result<(), String> {
    stop_recording_now(&core);
    Ok(())
}

#[tauri::command]
fn add_peer(core: State<Arc<Core>>, id: String) -> Result<(), String> {
    add_peer_id(&core, &id)
}

#[tauri::command]
fn remove_peer(core: State<Arc<Core>>, id: String) {
    if let Ok(id) = id.parse::<EndpointId>() {
        core.removed.lock().unwrap().insert(id.to_string());
        core.save_settings();
        let removed: Vec<Peer> = {
            let mut peers = core.peers.lock().unwrap();
            let (gone, kept): (Vec<Peer>, Vec<Peer>) = std::mem::take(&mut *peers).into_iter().partition(|p| p.id == id);
            *peers = kept;
            gone
        };
        for peer in removed {
            for conn in peer.conn.iter().chain(peer.incoming.iter()) {
                conn.close(2u32.into(), b"removed");
            }
        }
        core.save_peers();
    }
}

#[tauri::command]
fn create_invite(core: State<Arc<Core>>, reuse: bool) -> String {
    if reuse {
        if let Some(token) = core.invites.lock().unwrap().last() {
            return format!("ezh-{}-{token}", core.ep.id());
        }
    }
    let token: String = SecretKey::generate().to_bytes()[..8].iter().map(|b| format!("{b:02x}")).collect();
    core.invites.lock().unwrap().push(token.clone());
    format!("ezh-{}-{token}", core.ep.id())
}

#[tauri::command]
fn cancel_invites(core: State<Arc<Core>>) {
    core.invites.lock().unwrap().clear();
}

#[tauri::command]
fn set_audio_process(core: State<Arc<Core>>, name: String) {
    *core.audio_process.lock().unwrap() = name;
    core.save_settings();
}

#[tauri::command]
fn set_keep_days(core: State<Arc<Core>>, days: Option<u32>) {
    *core.keep_days.lock().unwrap() = days.filter(|d| *d > 0);
    core.save_settings();
}

#[tauri::command]
fn join_room(core: State<Arc<Core>>, code: String) -> Result<(), String> {
    let bad = || "That invite code is not valid. Ask for a new one.".to_string();
    let text: String = code.split_whitespace().collect();
    let (id, token) = text.strip_prefix("ezh-").and_then(|t| t.split_once('-')).ok_or_else(bad)?;
    let peer: EndpointId = id.parse().map_err(|_| bad())?;
    core.removed.lock().unwrap().remove(id);
    add_peer_id(&core, id)?;
    core.pending_join.lock().unwrap().insert(peer, token.to_string());
    Ok(())
}

#[tauri::command]
fn leave_room(core: State<Arc<Core>>) {
    let peers = std::mem::take(&mut *core.peers.lock().unwrap());
    *core.last_members.lock().unwrap() = peers.iter().map(|p| p.id.to_string()).collect();
    for peer in &peers {
        for conn in peer.conn.iter().chain(peer.incoming.iter()) {
            conn.close(2u32.into(), b"left");
        }
    }
    core.save_peers();
    core.save_settings();
}

#[tauri::command]
fn rejoin(core: State<Arc<Core>>) {
    let ids = core.last_members.lock().unwrap().clone();
    for id in ids {
        core.removed.lock().unwrap().remove(&id);
        let _ = add_peer_id(&core, &id);
    }
}

#[tauri::command]
fn set_share(core: State<Arc<Core>>, on: bool) {
    core.share.store(on, Ordering::Relaxed);
    core.save_settings();
}

#[tauri::command]
fn set_limit(core: State<Arc<Core>>, mbps: Option<f64>) {
    *core.limit_mbps.lock().unwrap() = mbps.filter(|v| *v > 0.0);
    core.save_settings();
}

#[tauri::command]
fn note_action(core: State<Arc<Core>>, kind: String, id: i64, text: String) {
    let kind = match kind.as_str() {
        "edit" => "note_edit",
        "delete" => "note_del",
        "restore" => "note_restore",
        _ => return,
    };
    let text: String = text.chars().filter(|c| !c.is_control()).take(NOTE_MAX).collect();
    core.append_log(&json!({"k": kind, "id": id, "t_us": now_us(), "text": text}));
}

#[tauri::command]
fn log_layout(core: State<Arc<Core>>, layout: Value) {
    core.append_log(&json!({"k": "layout", "t_us": now_us(), "layout": layout}));
}

#[tauri::command]
fn set_name(core: State<Arc<Core>>, name: String) {
    let name = clean_name(&name);
    *core.name.lock().unwrap() = name;
    core.save_settings();
}

#[tauri::command]
fn open_data_folder(core: State<Arc<Core>>) {
    data::open_folder(&core.dir);
}

#[tauri::command]
async fn data_info(core: State<'_, Arc<Core>>) -> Result<Value, String> {
    let dir = core.dir.clone();
    let used = tauri::async_runtime::spawn_blocking({
        let dir = dir.clone();
        move || data::used_bytes(&dir)
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(json!({"dir": dir, "used_bytes": used, "free_bytes": core.free_bytes.load(Ordering::Relaxed)}))
}

#[tauri::command]
fn reset(core: State<Arc<Core>>, everything: bool, confirm: String) -> Result<(), String> {
    if everything && confirm != "RESET" {
        return Err("Type RESET to confirm.".into());
    }
    stop_recording_now(&core);
    data::reset(&core.dir, everything);
    core.app.restart()
}

#[tauri::command]
fn ui_ready(core: State<Arc<Core>>) {
    let _ = std::fs::remove_file(core.dir.join("running.marker"));
}

#[tauri::command]
fn dismiss_recovery(core: State<Arc<Core>>) {
    *core.recovery.lock().unwrap() = None;
}

#[tauri::command]
fn add_note(core: State<Arc<Core>>, text: String, at_us: i64) {
    let text: String = text.chars().filter(|c| !c.is_control()).take(NOTE_MAX).collect();
    if text.trim().is_empty() {
        return;
    }
    core.append_log(&json!({"k": "note", "t_us": now_us(), "at_us": at_us, "text": text}));
}

#[tauri::command]
async fn control(core: State<'_, Arc<Core>>, cmd: String, wall_us: i64) -> Result<(), String> {
    net::send_control(&core, &cmd, wall_us).await;
    Ok(())
}

async fn init(app: AppHandle) -> Result<Arc<Core>, Box<dyn std::error::Error>> {
    let dir = match std::env::var_os("EZH_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => {
            let dir = app.path().app_local_data_dir()?;
            data::migrate(&app.path().app_data_dir()?, &dir);
            dir
        }
    };
    std::fs::create_dir_all(dir.join("peer"))?;
    debug::init(&dir);
    record::set_data_dir(dir.clone());
    log::repair(&dir.join("log.bin"));
    let marker = dir.join("running.marker");
    let mut recovery = marker.exists().then(|| "The app did not start properly last time.".to_string());
    let _ = std::fs::write(&marker, VERSION);
    let previous_version = std::fs::read_to_string(dir.join("version.txt")).unwrap_or_default().trim().to_string();
    let _ = std::fs::write(dir.join("version.txt"), VERSION);
    let key_path = dir.join("key.bin");
    let key = if key_path.exists() {
        match std::fs::read(&key_path).ok().and_then(|b| <[u8; 32]>::try_from(b).ok()) {
            Some(bytes) => SecretKey::from_bytes(&bytes),
            None => {
                recovery = Some("The identity key file could not be read. It was left untouched; continuing uses a temporary ID.".into());
                SecretKey::generate()
            }
        }
    } else {
        let key = SecretKey::generate();
        std::fs::write(&key_path, key.to_bytes())?;
        key
    };
    let ep = Endpoint::builder(presets::N0).secret_key(key).alpns(vec![ALPN.to_vec()]).bind().await?;
    std::fs::write(dir.join("id.txt"), ep.id().to_string())?;
    let saved: Vec<EndpointId> = std::fs::read_to_string(dir.join("peers.txt"))
        .unwrap_or_default()
        .lines()
        .filter_map(|l| l.trim().parse().ok())
        .collect();
    let settings: Value = std::fs::read_to_string(dir.join("settings.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let name = settings["name"].as_str().map(clean_name).unwrap_or_else(|| "player".into());
    Ok(Arc::new(Core {
        dir,
        ep,
        app,
        peers: Mutex::new(saved.iter().map(|id| Peer { id: *id, name: String::new(), conn: None, incoming: None, offset_us: None, failures: 0, announce: false, info: None }).collect()),
        name: Mutex::new(name),
        share: AtomicBool::new(settings["share"].as_bool().unwrap_or(true)),
        limit_mbps: Mutex::new(settings["limit_mbps"].as_f64()),
        last_members: Mutex::new(settings["last_members"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()),
        invites: Mutex::new(Vec::new()),
        audio_process: Mutex::new(settings["audio_process"].as_str().unwrap_or_default().to_string()),
        keep_days: Mutex::new(settings["keep_days"].as_u64().map(|d| d as u32)),
        ready: AtomicBool::new(false),
        removed: Mutex::new(settings["removed"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()),
        pending_join: Mutex::new(HashMap::new()),
        following: Mutex::new(HashSet::new()),
        recording_name: Mutex::new(None),
        recovery: Mutex::new(recovery),
        notice: Mutex::new(String::new()),
        free_bytes: AtomicU64::new(0),
        previous_version,
        ffmpeg_status: Mutex::new(String::new()),
        events: Mutex::new(Vec::new()),
        started: Instant::now(),
        recorder: Mutex::new(None),
        log_lock: Mutex::new(()),
    }))
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            let core = tauri::async_runtime::block_on(init(app.handle().clone()))?;
            app.manage(core.clone());
            shell::setup(app, core.clone())?;
            core.ready.store(true, Ordering::Relaxed);
            let keep_core = core.clone();
            std::thread::spawn(move || loop {
                data::delete_old(&keep_core);
                std::thread::sleep(Duration::from_secs(3600));
            });
            tauri::async_runtime::spawn(net::accept_loop(core.clone()));
            let saved: Vec<EndpointId> = core.peers.lock().unwrap().iter().map(|p| p.id).collect();
            for id in saved {
                tauri::async_runtime::spawn(net::dial_loop(core.clone(), id));
            }
            let free_core = core.clone();
            std::thread::spawn(move || loop {
                free_core.free_bytes.store(data::free_bytes(&free_core.dir), Ordering::Relaxed);
                std::thread::sleep(Duration::from_secs(20));
            });
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
            for (i, a) in args.iter().enumerate() {
                if a == "--peer" {
                    if let Some(id) = args.get(i + 1) {
                        add_peer_id(&core, id)?;
                    }
                }
            }
            if let Some(seconds) = arg("--record").and_then(|s| s.parse::<u64>().ok()) {
                let pid = arg("--audio-pid").and_then(|s| s.parse().ok()).unwrap_or(0);
                std::thread::spawn(move || {
                    if let Err(e) = start_recording_now(&core, pid, "") {
                        eprintln!("record failed: {e}");
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(seconds));
                    stop_recording_now(&core);
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![state, check, list_processes, debug_text, save_report, start_recording, stop_recording, add_peer, remove_peer, create_invite, join_room, leave_room, rejoin, set_share, set_limit, cancel_invites, set_audio_process, set_keep_days, note_action, log_layout, set_name, open_data_folder, data_info, reset, ui_ready, dismiss_recovery, add_note, control])
        .run(tauri::generate_context!())
        .expect("failed to run");
}

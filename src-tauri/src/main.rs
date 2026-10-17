#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod check;
mod data;
mod debug;
mod log;
mod net;
mod record;
mod room;
mod shell;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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
    pub codecs: Vec<String>,
    pub info: Option<(String, u32)>,
    pub sharing: bool,
    pub live: Option<String>,
    pub have: HashSet<String>,
    pub gone: HashSet<String>,
}

pub struct Core {
    pub dir: PathBuf,
    pub ep: Endpoint,
    pub app: AppHandle,
    pub peers: Mutex<Vec<Peer>>,
    pub name: Mutex<String>,
    pub share: AtomicBool,
    pub limit_mbps: Mutex<Option<f64>>,
    pub invites: Mutex<Vec<(String, String)>>,
    pub rooms: Mutex<room::Rooms>,
    pub active_set: Mutex<Option<room::ActiveSet>>,
    pub tz_offset_s: i64,
    pub audio_process: Mutex<String>,
    pub keep_days: Mutex<Option<u32>>,
    pub ready: AtomicBool,
    pub codecs: Mutex<Vec<String>>,
    pub removed: Mutex<HashSet<String>>,
    pub pending_join: Mutex<HashMap<EndpointId, String>>,
    pub following: Mutex<HashSet<String>>,
    pub recovery: Mutex<Option<String>>,
    pub notice: Mutex<String>,
    pub free_bytes: AtomicU64,
    pub total_bytes: AtomicU64,
    pub deleted: Mutex<HashSet<String>>,
    pub pacers: [Mutex<Instant>; 2],
    pub previous_version: String,
    pub ffmpeg_status: Mutex<String>,
    pub events: Mutex<Vec<String>>,
    pub started: Instant,
    pub engine: record::Engine,
    pub layout: Mutex<Value>,
    log_lock: Mutex<()>,
}

impl Core {
    pub fn append_log(&self, record: &Value) {
        let _guard = self.log_lock.lock().unwrap();
        let _ = log::append(&self.dir.join("log.bin"), record);
    }

    pub fn own_log(&self) -> Vec<Value> {
        log::read(&self.dir.join("log.bin"))
    }

    pub fn save_settings(&self) {
        let value = json!({
            "name": *self.name.lock().unwrap(),
            "share": self.share.load(Ordering::Relaxed),
            "limit_mbps": *self.limit_mbps.lock().unwrap(),
            "removed": *self.removed.lock().unwrap(),
            "audio_process": *self.audio_process.lock().unwrap(),
            "keep_days": *self.keep_days.lock().unwrap(),
            "source": self.engine.source.lock().unwrap().clone(),
            "mic": self.engine.mic.lock().unwrap().clone(),
            "mix": self.engine.mix.lock().unwrap().clone(),
            "deleted_sets": *self.deleted.lock().unwrap(),
            "layout": *self.layout.lock().unwrap(),
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

    pub fn is_deleted(&self, room: &str, set: &str) -> bool {
        self.deleted.lock().unwrap().contains(&format!("{room}/{set}"))
    }

    fn disk_below(&self, share: f64) -> bool {
        let total = self.total_bytes.load(Ordering::Relaxed);
        total > 0 && (self.free_bytes.load(Ordering::Relaxed) as f64) < total as f64 * share
    }

    pub fn disk_low(&self) -> bool {
        self.disk_below(data::LOW_SHARE)
    }

    pub fn disk_full(&self) -> bool {
        self.disk_below(data::FULL_SHARE)
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

fn size(path: PathBuf) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

pub fn audio_target(wanted: &str) -> (String, u32) {
    processes()
        .into_iter()
        .find(|(name, _)| if wanted.is_empty() { name.to_lowercase().starts_with("r5apex") } else { name.eq_ignore_ascii_case(wanted) })
        .unwrap_or_default()
}

fn pid_of(name: &str) -> Option<u32> {
    processes().into_iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, pid)| pid)
}

pub fn processes() -> Vec<(String, u32)> {
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
    let core = core.inner().clone();
    let room_id = core.active_room_id();
    let my_id = core.ep.id().to_string();
    type Snapshot = (EndpointId, String, Option<Connection>, Option<Connection>, Option<i64>, Option<(String, u32)>, u32, bool, Option<String>);
    let snapshot: Vec<Snapshot> = core
        .peers
        .lock()
        .unwrap()
        .iter()
        .map(|p| (p.id, p.name.clone(), p.conn.clone(), p.incoming.clone(), p.offset_us, p.info.clone(), p.failures, p.sharing, p.live.clone()))
        .collect();
    let mut peers = Vec::new();
    let mut names: HashMap<String, String> = HashMap::from([(my_id.clone(), "You".to_string())]);
    let live_of = |name: Option<String>| {
        name.map(|n| {
            let path = core.live_dir(&room_id).join(&n);
            json!({"path": path, "size": size(path.clone()), "start_us": record::live_name(&n).map(|(_, ms)| ms * 1000)})
        })
    };
    for (id, name, conn, incoming, offset_us, info, failures, sharing, live) in snapshot {
        names.insert(id.to_string(), if name.is_empty() { short(&id) } else { format!("{name} ({})", short(&id)) });
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
            "sharing": sharing && alive.is_some(),
            "live": live_of(live.filter(|_| sharing && alive.is_some())),
        }));
    }
    let logs = room::logs(&core, &room_id);
    let mut notes = room::notes(&logs, &room_id);
    for n in notes.iter_mut() {
        let author = n["author"].as_str().unwrap_or_default().to_string();
        n["mine"] = json!(author == my_id);
        n["who"] = json!(names.get(&author).cloned().unwrap_or_else(|| author.chars().take(10).collect()));
        n["sets"] = json!([]);
        n["seekable"] = json!(false);
    }
    let sets = room::sets_json(&core, &room_id, &logs, &mut notes);
    let (free, total) = (core.free_bytes.load(Ordering::Relaxed), core.total_bytes.load(Ordering::Relaxed));
    let rooms = core.rooms.lock().unwrap();
    let room_list: Vec<Value> = rooms.list.iter().map(|r| json!({"id": r.id, "name": r.name, "active": rooms.active.as_deref() == Some(&r.id), "members": r.members.len()})).collect();
    drop(rooms);
    Ok(json!({
        "version": VERSION,
        "ffmpeg": core.ffmpeg_status.lock().unwrap().clone(),
        "id": my_id,
        "name": core.name.lock().unwrap().clone(),
        "share": core.share.load(Ordering::Relaxed),
        "audio_process": core.audio_process.lock().unwrap().clone(),
        "keep_days": *core.keep_days.lock().unwrap(),
        "invites": core.invites.lock().unwrap().len(),
        "limit_mbps": *core.limit_mbps.lock().unwrap(),
        "recovery": core.recovery.lock().unwrap().clone(),
        "notice": core.notice.lock().unwrap().clone(),
        "disk": {"free": free, "total": total, "low": core.disk_low(), "full": core.disk_full()},
        "previous_version": core.previous_version,
        "codec": record::pick(&core.codecs.lock().unwrap()).1,
        "recording": core.active_set.lock().unwrap().is_some(),
        "capture": capture_json(&core, live_of(core.engine.live_file())),
        "active_set": core.active_set_json(),
        "room": core.active_room().map(|r| json!({"id": r.id, "name": r.name})),
        "rooms": room_list,
        "layout": core.layout.lock().unwrap().clone(),
        "sets": sets,
        "peers": peers,
        "notes": notes,
    }))
}

fn capture_json(core: &Core, live: Option<Value>) -> Value {
    let e = &core.engine;
    let names = e.names.lock().unwrap().clone();
    let mix = |track| {
        let key = e.mix_key(track);
        let m = e.mix.lock().unwrap()[&key].clone();
        json!({"volume": m["volume"].as_f64().unwrap_or(1.0), "muted": m["muted"].as_bool().unwrap_or(false)})
    };
    json!({
        "source": e.source.lock().unwrap().clone(),
        "source_name": names.source,
        "source_kind": names.kind,
        "source_lost": e.source_lost.load(Ordering::Relaxed),
        "sharing": e.sink.lock().unwrap().is_some() && core.share.load(Ordering::Relaxed),
        "status": e.status.lock().unwrap().clone(),
        "game": names.game,
        "mic": e.mic.lock().unwrap().clone(),
        "mic_name": names.mic,
        "mix": {"game": mix(record::GAME), "mic": mix(record::MIC)},
        "live": live,
    })
}

#[tauri::command]
async fn list_sources() -> Vec<Value> {
    record::list_sources()
}

#[tauri::command]
fn set_source(core: State<Arc<Core>>, source: Value) -> Result<(), String> {
    if !record::valid_source(&source) {
        return Err("unknown source".into());
    }
    *core.engine.source.lock().unwrap() = source;
    core.save_settings();
    Ok(())
}

#[tauri::command]
fn use_display(core: State<Arc<Core>>) {
    *core.engine.source.lock().unwrap() = record::primary_display();
    core.save_settings();
}

#[tauri::command]
async fn list_mics() -> Result<Vec<Value>, String> {
    tauri::async_runtime::spawn_blocking(record::list_mics).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn set_mic(core: State<Arc<Core>>, id: String) {
    *core.engine.mic.lock().unwrap() = id;
    core.save_settings();
}

#[tauri::command]
fn set_mix(core: State<Arc<Core>>, kind: String, volume: f64, muted: bool) -> Result<(), String> {
    let track = match kind.as_str() {
        "game" => record::GAME,
        "mic" => record::MIC,
        _ => return Err("unknown kind".into()),
    };
    let key = core.engine.mix_key(track);
    core.engine.mix.lock().unwrap()[&key] = json!({"volume": volume.clamp(0.0, 1.0), "muted": muted});
    core.save_settings();
    Ok(())
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
async fn start_recording(core: State<'_, Arc<Core>>) -> Result<(), String> {
    room::start_set(&core)
}

#[tauri::command]
async fn stop_recording(core: State<'_, Arc<Core>>) -> Result<(), String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || room::stop_set(&core)).await.map_err(|e| e.to_string())
}

#[tauri::command]
fn delete_set(core: State<Arc<Core>>, room: String, set: String) -> Result<(), String> {
    room::delete_set(&core, &room, &set)
}

#[tauri::command]
fn open_set_folder(core: State<Arc<Core>>, room: String, set: String) {
    if room::valid_id(&room) && room::valid_id(&set) {
        data::open_folder(&core.set_dir(&room, &set));
    }
}

#[tauri::command]
fn set_layout(core: State<Arc<Core>>, layout: Value) {
    *core.layout.lock().unwrap() = layout;
    core.save_settings();
    let active = core.active_set.lock().unwrap().clone();
    if let Some(active) = active {
        room::write_set_json(&core, &active, false);
    }
}

#[tauri::command]
fn remove_peer(core: State<Arc<Core>>, id: String) {
    room::remove_member(&core, &id);
}

#[tauri::command]
fn create_invite(core: State<Arc<Core>>, reuse: bool) -> String {
    let room = core.active_room_id();
    if reuse {
        if let Some((token, _)) = core.invites.lock().unwrap().iter().rev().find(|(_, r)| *r == room) {
            return format!("ezh-{}-{token}", core.ep.id());
        }
    }
    let token = room::random_hex(8);
    core.invites.lock().unwrap().push((token.clone(), room));
    format!("ezh-{}-{token}", core.ep.id())
}

#[tauri::command]
fn cancel_invites(core: State<Arc<Core>>) {
    core.invites.lock().unwrap().clear();
}

#[tauri::command]
fn set_codecs(core: State<Arc<Core>>, list: Vec<String>) {
    *core.codecs.lock().unwrap() = list;
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
fn create_room(core: State<Arc<Core>>, name: String) -> Result<String, String> {
    if core.active_set.lock().unwrap().is_some() {
        return Err("Stop recording before switching rooms.".into());
    }
    Ok(room::create_room(&core, &name))
}

#[tauri::command]
fn join_room(core: State<Arc<Core>>, code: String) -> Result<(), String> {
    let bad = || "That invite code is not valid. Ask for a new one.".to_string();
    let text: String = code.split_whitespace().collect();
    let (id, token) = text.strip_prefix("ezh-").and_then(|t| t.split_once('-')).ok_or_else(bad)?;
    let peer: EndpointId = id.parse().map_err(|_| bad())?;
    if core.active_set.lock().unwrap().is_some() {
        return Err("Stop recording before switching rooms.".into());
    }
    core.removed.lock().unwrap().remove(id);
    let pending = format!("pending-{token}");
    {
        let mut rooms = core.rooms.lock().unwrap();
        rooms.list.retain(|r| r.id != pending);
        rooms.list.push(room::Room { id: pending.clone(), name: "Joining...".into(), members: vec![core.ep.id().to_string(), id.to_string()] });
        rooms.active = Some(pending.clone());
    }
    core.save_rooms();
    core.pending_join.lock().unwrap().insert(peer, token.to_string());
    room::apply_active(&core);
    Ok(())
}

#[tauri::command]
fn switch_room(core: State<Arc<Core>>, id: String) -> Result<(), String> {
    room::switch_room(&core, &id)
}

#[tauri::command]
fn leave_room(core: State<Arc<Core>>, id: String) {
    room::leave_room(&core, &id);
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
    room::stop_set(core.inner());
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
    let id = now_us();
    core.append_log(&json!({"k": "note", "id": id, "t_us": id, "room": core.active_room_id(), "at_us": at_us, "text": text}));
}

#[tauri::command]
async fn control(core: State<'_, Arc<Core>>, cmd: String, wall_us: i64, set: String, sent_us: i64, origin: String) -> Result<(), String> {
    net::send_control(&core, &json!({"t": "ctl", "cmd": cmd, "wall_us": wall_us, "set": set, "sent_us": sent_us, "origin": origin})).await;
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
    let rooms: room::Rooms = std::fs::read_to_string(dir.join("rooms.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let settings: Value = std::fs::read_to_string(dir.join("settings.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let name = settings["name"].as_str().map(clean_name).unwrap_or_else(|| "player".into());
    Ok(Arc::new(Core {
        dir,
        ep,
        app,
        peers: Mutex::new(Vec::new()),
        rooms: Mutex::new(rooms),
        active_set: Mutex::new(None),
        tz_offset_s: data::tz_offset_seconds(),
        name: Mutex::new(name),
        share: AtomicBool::new(settings["share"].as_bool().unwrap_or(true)),
        limit_mbps: Mutex::new(settings["limit_mbps"].as_f64()),
        invites: Mutex::new(Vec::new()),
        audio_process: Mutex::new(settings["audio_process"].as_str().unwrap_or_default().to_string()),
        keep_days: Mutex::new(settings["keep_days"].as_u64().map(|d| d as u32)),
        ready: AtomicBool::new(false),
        codecs: Mutex::new(vec!["h264".to_string()]),
        removed: Mutex::new(settings["removed"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()),
        pending_join: Mutex::new(HashMap::new()),
        following: Mutex::new(HashSet::new()),
        recovery: Mutex::new(recovery),
        notice: Mutex::new(String::new()),
        free_bytes: AtomicU64::new(0),
        total_bytes: AtomicU64::new(0),
        deleted: Mutex::new(settings["deleted_sets"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default()),
        pacers: [Mutex::new(Instant::now()), Mutex::new(Instant::now())],
        previous_version,
        ffmpeg_status: Mutex::new(String::new()),
        events: Mutex::new(Vec::new()),
        started: Instant::now(),
        engine: record::Engine::new(&settings),
        layout: Mutex::new(settings["layout"].clone()),
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
            let unhashed = data::unhashed(&core);
            std::thread::spawn(move || data::keeper(keep_core, unhashed));
            tauri::async_runtime::spawn(net::accept_loop(core.clone()));
            room::apply_active(&core);
            let capture_core = core.clone();
            std::thread::spawn(move || record::supervise(capture_core));
            let free_core = core.clone();
            std::thread::spawn(move || loop {
                data::refresh_disk(&free_core);
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
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![state, check, list_processes, debug_text, save_report, start_recording, stop_recording, delete_set, open_set_folder, set_layout, remove_peer, create_invite, create_room, join_room, switch_room, leave_room, set_share, set_limit, set_codecs, cancel_invites, list_sources, set_source, use_display, list_mics, set_mic, set_mix, set_audio_process, set_keep_days, note_action, log_layout, set_name, open_data_folder, data_info, reset, ui_ready, dismiss_recovery, add_note, control])
        .run(tauri::generate_context!())
        .expect("failed to run");
}

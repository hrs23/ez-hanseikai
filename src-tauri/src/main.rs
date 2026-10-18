#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod check;
mod data;
mod debug;
mod export;
mod ffmpeg;
mod log;
mod net;
mod record;
mod room;
mod shell;
mod sources;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iroh::endpoint::{presets, Connection};
use iroh::{Endpoint, EndpointId, SecretKey, Watcher};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

pub const ALPN: &[u8] = b"ez-hanseikai/0";
pub const PROTOCOL: u32 = 1;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const NO_WINDOW: u32 = 0x08000000;
pub const BELOW_NORMAL: u32 = 0x00004000;
const NOTE_MAX: usize = 500;
const NAME_MAX: usize = 24;
pub const ROOM_NAME_MAX: usize = 40;
const DISK_EVERY: Duration = Duration::from_secs(20);

pub fn now_us() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_micros() as i64
}

pub fn short(id: impl std::fmt::Display) -> String {
    id.to_string().chars().take(10).collect()
}

pub fn clean_text(raw: &str, max: usize) -> String {
    raw.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(max).collect::<String>().trim().to_string()
}

pub fn clean_name(raw: &str) -> String {
    clean_text(raw, NAME_MAX)
}

fn clean_note(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control()).take(NOTE_MAX).collect()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn strings<C: FromIterator<String>>(value: &Value) -> C {
    value.as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect()
}

pub fn merge(into: &mut Value, from: &Value) {
    if let (Some(into), Some(from)) = (into.as_object_mut(), from.as_object()) {
        into.extend(from.clone());
    }
}

pub fn default_codecs() -> Vec<String> {
    vec!["h264".to_string()]
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

impl Peer {
    pub fn new(id: EndpointId, name: String) -> Self {
        Self { id, name, conn: None, incoming: None, offset_us: None, failures: 0, announce: true, codecs: default_codecs(), info: None, sharing: false, live: None, have: HashSet::new(), gone: HashSet::new() }
    }
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
    pub codecs: Mutex<Vec<String>>,
    pub removed: Mutex<HashSet<String>>,
    pub following: Mutex<HashSet<String>>,
    pub recovery: Mutex<Option<String>>,
    pub notice: Mutex<String>,
    pub free_bytes: AtomicU64,
    pub total_bytes: AtomicU64,
    pub deleted: Mutex<HashSet<String>>,
    pub pacers: [Mutex<Instant>; 2],
    pub previous_version: String,
    pub ffmpeg_status: Mutex<String>,
    pub events: Mutex<VecDeque<String>>,
    pub started: Instant,
    pub engine: record::Engine,
    pub layout: Mutex<Value>,
    pub export: Mutex<Option<export::Running>>,
    pub log_lock: Mutex<()>,
    logs: Mutex<HashMap<PathBuf, (Arc<Vec<Value>>, u64)>>,
    pub jsons: Mutex<HashMap<PathBuf, Value>>,
}

impl Core {
    pub fn append_log(&self, record: &Value) {
        let _guard = self.log_lock.lock().unwrap();
        let _ = log::append(&self.dir.join("log.bin"), record);
    }

    pub fn log_path(&self, id: &EndpointId) -> PathBuf {
        if *id == self.ep.id() { self.dir.join("log.bin") } else { self.peer_dir(id).join("log.bin") }
    }

    pub fn log(&self, id: &EndpointId) -> Arc<Vec<Value>> {
        let path = self.log_path(id);
        let mut cache = self.logs.lock().unwrap();
        let entry = cache.entry(path.clone()).or_default();
        let len = file_len(&path);
        if len < entry.1 {
            *entry = Default::default();
        }
        if len > entry.1 {
            let (more, used) = log::scan(&path, entry.1);
            if !more.is_empty() {
                Arc::make_mut(&mut entry.0).extend(more);
            }
            entry.1 += used;
        }
        entry.0.clone()
    }

    pub fn own_log(&self) -> Arc<Vec<Value>> {
        self.log(&self.ep.id())
    }

    pub fn save_settings(&self) {
        let value = json!({
            "name": *self.name.lock().unwrap(),
            "share": self.share.load(Ordering::Relaxed),
            "limit_mbps": *self.limit_mbps.lock().unwrap(),
            "audio_process": *self.audio_process.lock().unwrap(),
            "keep_days": *self.keep_days.lock().unwrap(),
            "removed": *self.removed.lock().unwrap(),
            "deleted_sets": *self.deleted.lock().unwrap(),
            "source": self.engine.source.lock().unwrap().clone(),
            "mic": self.engine.mic.lock().unwrap().clone(),
            "mix": self.engine.mix.lock().unwrap().clone(),
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

    pub fn sharing_live(&self) -> bool {
        self.engine.sink.lock().unwrap().is_some() && self.share.load(Ordering::Relaxed)
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

#[tauri::command]
async fn list_processes() -> Vec<String> {
    let mut names: Vec<String> = sources::processes().into_iter().map(|(name, _)| name).collect();
    names.sort_by_key(|n| (!n.to_lowercase().contains("apex"), n.to_lowercase()));
    names.dedup();
    names
}

#[tauri::command]
async fn state(core: State<'_, Arc<Core>>) -> Result<Value, String> {
    Ok(room::state_json(&core))
}

#[tauri::command]
async fn list_sources() -> Vec<Value> {
    sources::list()
}

#[tauri::command]
fn set_source(core: State<Arc<Core>>, source: Value) -> Result<(), String> {
    if !sources::valid(&source) {
        return Err("unknown source".into());
    }
    *core.engine.source.lock().unwrap() = source;
    core.save_settings();
    Ok(())
}

#[tauri::command]
fn use_display(core: State<Arc<Core>>) {
    *core.engine.source.lock().unwrap() = sources::primary_display();
    core.save_settings();
}

#[tauri::command]
async fn list_mics() -> Result<Vec<Value>, String> {
    tauri::async_runtime::spawn_blocking(audio::list_mics).await.map_err(|e| e.to_string())
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
    tauri::async_runtime::spawn_blocking(move || check::run(&core, sources::pid_of(&audio_process)))
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
        let lines = check::run(&core, sources::pid_of(&audio_process));
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
async fn export_set(core: State<'_, Arc<Core>>, room: String, set: String) -> Result<Value, String> {
    if !room::valid_id(&room) || !room::valid_id(&set) {
        return Err("Unknown set.".into());
    }
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        export::check(&core)?;
        let Some(path) = shell::pick_file(true, &format!("{set}.mp4")) else { return Ok(Value::Null) };
        export::start(&core, &room, &set, path).map(|skipped| json!({"skipped": skipped}))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn cancel_export(core: State<Arc<Core>>) {
    export::cancel(&core);
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
    let mut invites = core.invites.lock().unwrap();
    let token = match invites.iter().rev().find(|(_, r)| *r == room).filter(|_| reuse) {
        Some((token, _)) => token.clone(),
        None => {
            let token = room::random_hex(8);
            invites.push((token.clone(), room));
            token
        }
    };
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
    room::create_room(&core, &name)
}

#[tauri::command]
fn join_room(core: State<Arc<Core>>, code: String) -> Result<(), String> {
    room::join_room(&core, &code)
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
    core.append_log(&json!({"k": kind, "id": id, "t_us": now_us(), "text": clean_note(&text)}));
}

#[tauri::command]
fn log_layout(core: State<Arc<Core>>, layout: Value) {
    core.append_log(&json!({"k": "layout", "t_us": now_us(), "layout": layout}));
}

#[tauri::command]
fn set_name(core: State<Arc<Core>>, name: String) {
    *core.name.lock().unwrap() = clean_name(&name);
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
    let text = clean_note(&text);
    if text.trim().is_empty() {
        return;
    }
    let id = now_us();
    core.append_log(&json!({"k": "note", "id": id, "t_us": id, "room": core.active_room_id(), "at_us": at_us, "text": text}));
}

#[tauri::command]
fn control(core: State<Arc<Core>>, cmd: String, wall_us: i64, set: String, sent_us: i64, origin: String) {
    net::broadcast(&core, json!({"t": "ctl", "cmd": cmd, "wall_us": wall_us, "set": set, "sent_us": sent_us, "origin": origin}));
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
    ffmpeg::set_data_dir(dir.clone());
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
    let mut builder = Endpoint::builder(presets::N0).secret_key(key).alpns(vec![ALPN.to_vec()]);
    if std::env::var_os("EZH_RELAY_ONLY").is_some() {
        builder = builder.clear_ip_transports();
    }
    let ep = builder.bind().await?;
    std::fs::write(dir.join("id.txt"), ep.id().to_string())?;
    let rooms: room::Rooms = read_json(&dir.join("rooms.json"));
    let settings: Value = read_json(&dir.join("settings.json"));
    Ok(Arc::new(Core {
        dir,
        ep,
        app,
        peers: Mutex::new(Vec::new()),
        name: Mutex::new(settings["name"].as_str().map(clean_name).unwrap_or_else(|| "player".into())),
        share: AtomicBool::new(settings["share"].as_bool().unwrap_or(true)),
        limit_mbps: Mutex::new(settings["limit_mbps"].as_f64()),
        invites: Mutex::new(Vec::new()),
        rooms: Mutex::new(rooms),
        active_set: Mutex::new(None),
        tz_offset_s: data::tz_offset_seconds(),
        audio_process: Mutex::new(settings["audio_process"].as_str().unwrap_or_default().to_string()),
        keep_days: Mutex::new(settings["keep_days"].as_u64().map(|d| d as u32)),
        codecs: Mutex::new(default_codecs()),
        removed: Mutex::new(strings(&settings["removed"])),
        following: Mutex::new(HashSet::new()),
        recovery: Mutex::new(recovery),
        notice: Mutex::new(String::new()),
        free_bytes: AtomicU64::new(0),
        total_bytes: AtomicU64::new(0),
        deleted: Mutex::new(strings(&settings["deleted_sets"])),
        pacers: [Mutex::new(Instant::now()), Mutex::new(Instant::now())],
        previous_version,
        ffmpeg_status: Mutex::new(String::new()),
        events: Mutex::new(VecDeque::new()),
        started: Instant::now(),
        engine: record::Engine::new(&settings),
        layout: Mutex::new(settings["layout"].clone()),
        export: Mutex::new(None),
        log_lock: Mutex::new(()),
        logs: Mutex::new(HashMap::new()),
        jsons: Mutex::new(HashMap::new()),
    }))
}

fn prepare_ffmpeg(core: &Core) {
    if !ffmpeg::present() {
        *core.ffmpeg_status.lock().unwrap() = "downloading ffmpeg...".into();
        tracing::info!("downloading ffmpeg");
        let result = ffmpeg::download();
        *core.ffmpeg_status.lock().unwrap() = match &result {
            Ok(()) => String::new(),
            Err(e) => format!("ffmpeg download failed: {e}"),
        };
        if let Err(e) = result {
            tracing::warn!("ffmpeg download failed: {e}");
        }
    }
    ffmpeg::probe_encoders();
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
            let unhashed = data::unhashed(&core);
            let c = core.clone();
            std::thread::spawn(move || data::keeper(c, unhashed));
            tauri::async_runtime::spawn(net::accept_loop(core.clone()));
            room::apply_active(&core);
            let c = core.clone();
            std::thread::spawn(move || record::supervise(c));
            let c = core.clone();
            std::thread::spawn(move || loop {
                data::refresh_disk(&c);
                std::thread::sleep(DISK_EVERY);
            });
            let c = core.clone();
            std::thread::spawn(move || prepare_ffmpeg(&c));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![state, check, list_processes, debug_text, save_report, start_recording, stop_recording, delete_set, open_set_folder, export_set, cancel_export, set_layout, remove_peer, create_invite, create_room, join_room, switch_room, leave_room, set_share, set_limit, set_codecs, cancel_invites, list_sources, set_source, use_display, list_mics, set_mic, set_mix, set_audio_process, set_keep_days, note_action, log_layout, set_name, open_data_folder, data_info, reset, ui_ready, dismiss_recovery, add_note, control])
        .run(tauri::generate_context!())
        .expect("failed to run");
}

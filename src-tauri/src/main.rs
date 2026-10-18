#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod check;
mod data;
mod debug;
mod download;
mod export;
mod ffmpeg;
mod jobs;
mod log;
mod media;
mod net;
mod party;
mod plugins;
mod proc;
mod record;
mod room;
mod shell;
mod sources;
mod stream;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iroh::endpoint::{presets, Connection};
use iroh::{Endpoint, EndpointId, SecretKey, Watcher};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

pub const ALPN: &[u8] = b"ez-hanseikai/0";
pub const PROTOCOL: u32 = 4;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub use proc::{BELOW_NORMAL, NO_WINDOW};
const NOTE_MAX: usize = 500;
const BOARD_MAX: usize = 4000;
const VALUES_MAX: usize = 32 << 10;
const EVENT_KINDS: [&str; 3] = ["note", "drawing", "board"];
const NAME_MAX: usize = 24;
pub const ROOM_NAME_MAX: usize = 40;
const DISK_EVERY: Duration = Duration::from_secs(20);
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";
static TEST_PORT: LazyLock<Option<String>> = LazyLock::new(|| std::env::var("EZH_TEST").ok());

pub fn test_mode() -> bool {
    TEST_PORT.is_some()
}

fn open_main(app: &tauri::App) -> tauri::Result<()> {
    let config = app.config().app.windows.first().cloned().unwrap_or_default();
    let mut builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?;
    if let Some(port) = TEST_PORT.as_ref().and_then(|p| p.parse::<u16>().ok()) {
        builder = builder.additional_browser_args(&format!("{BROWSER_ARGS} --remote-debugging-port={port}"));
    }
    builder.build().map(|window| shell::plain_keys(&window))
}

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

fn clean_note(raw: &str, max: usize) -> String {
    raw.chars().filter(|c| !c.is_control() || *c == '\n').take(max).collect()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

pub fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static WRITING: Mutex<()> = Mutex::new(());
    let _guard = WRITING.lock().unwrap();
    let part = room::part_of(path);
    std::fs::write(&part, bytes)?;
    std::fs::rename(part, path)
}

pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok()).unwrap_or_default()
}

pub fn strings<C: FromIterator<String>>(value: &Value) -> C {
    value.as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect()
}

pub fn merge(into: &mut Value, from: &Value) {
    if let (Some(into), Some(from)) = (into.as_object_mut(), from.as_object()) {
        into.extend(from.clone());
    }
}

pub fn os_language() -> &'static str {
    const JAPANESE: u16 = 0x11;
    const PRIMARY: u16 = 0x3ff;
    if unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() } & PRIMARY == JAPANESE { "ja" } else { "en" }
}

pub fn language(core: &Core) -> String {
    match core.language.lock().unwrap().as_str() {
        "" => os_language().to_string(),
        chosen => chosen.to_string(),
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
    pub have: HashMap<String, u64>,
    pub logs: HashMap<String, u64>,
    pub gone: HashSet<String>,
    pub asks: Vec<Value>,
}

impl Peer {
    pub fn new(id: EndpointId, name: String) -> Self {
        Self { id, name, conn: None, incoming: None, offset_us: None, failures: 0, announce: true, codecs: default_codecs(), info: None, sharing: false, have: HashMap::new(), logs: HashMap::new(), gone: HashSet::new(), asks: Vec::new() }
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
    pub following: Mutex<HashSet<String>>,
    pub recovery: Mutex<Option<String>>,
    pub notice: Mutex<String>,
    pub free_bytes: AtomicU64,
    pub total_bytes: AtomicU64,
    pub deleted: Mutex<HashSet<String>>,
    pub pacer: Mutex<net::Pacer>,
    pub previous_version: String,
    pub ffmpeg_status: Mutex<String>,
    pub events: Mutex<VecDeque<debug::Event>>,
    pub started: Instant,
    pub engine: record::Engine,
    pub layout: Mutex<Value>,
    pub export: Mutex<Option<export::Running>>,
    pub log_lock: Mutex<()>,
    pub wake: tokio::sync::Notify,
    pub focus: Mutex<Option<String>>,
    pub asks: Mutex<Vec<room::Ask>>,
    pub party: Mutex<party::Party>,
    pub language: Mutex<String>,
    pub hotkey: Mutex<String>,
    pub hid_once: AtomicBool,
    logs: Mutex<HashMap<PathBuf, (Arc<Vec<Value>>, u64)>>,
    pub jsons: Mutex<HashMap<PathBuf, Value>>,
    pub writing: Mutex<HashSet<PathBuf>>,
    pub indexes: Mutex<HashMap<PathBuf, (u64, Arc<Vec<media::Time>>)>>,
}

impl Core {
    pub fn me(&self) -> String {
        self.ep.id().to_string()
    }

    pub fn log_path(&self, room: &str, id: &str) -> PathBuf {
        self.dir.join("rooms").join(room).join("logs").join(format!("{id}.bin"))
    }

    fn pending_path(&self, room: &str) -> PathBuf {
        self.dir.join("rooms").join(room).join("pending.bin")
    }

    pub fn log_ready(&self, room: &str) -> bool {
        !self.rooms.lock().unwrap().list.iter().any(|r| r.id == room && r.members.len() > 1)
            || (file_len(&self.log_path(room, &self.me())) > 0 && !self.pending_path(room).exists())
    }

    pub fn append_log(&self, room: &str, record: &Value) {
        let _guard = self.log_lock.lock().unwrap();
        let path = if self.log_ready(room) { self.log_path(room, &self.me()) } else { self.pending_path(room) };
        let _ = log::append(&path, record);
    }

    pub fn hold_own_log(&self, room: &str) -> std::io::Result<()> {
        log::append_bytes(&self.pending_path(room), &[])
    }

    pub fn settle(&self, room: &str) -> std::io::Result<()> {
        let pending = self.pending_path(room);
        let Ok(bytes) = std::fs::read(&pending) else { return Ok(()) };
        let whole = log::parse(&bytes).1;
        if whole > 0 {
            log::append_bytes(&self.log_path(room, &self.me()), &bytes[..whole])?;
        }
        std::fs::remove_file(pending)
    }

    pub fn room_log(&self, room: &str, id: &str) -> Arc<Vec<Value>> {
        self.read_log(self.log_path(room, id))
    }

    fn read_log(&self, path: PathBuf) -> Arc<Vec<Value>> {
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

    pub fn own_log(&self, room: &str) -> Arc<Vec<Value>> {
        let own = self.room_log(room, &self.me());
        let pending = self.read_log(self.pending_path(room));
        if pending.is_empty() { own } else { Arc::new(own.iter().chain(pending.iter()).cloned().collect()) }
    }

    pub fn save_settings(&self) {
        let value = json!({
            "name": *self.name.lock().unwrap(),
            "share": self.share.load(Ordering::Relaxed),
            "limit_mbps": *self.limit_mbps.lock().unwrap(),
            "audio_process": *self.audio_process.lock().unwrap(),
            "keep_days": *self.keep_days.lock().unwrap(),
            "deleted_sets": *self.deleted.lock().unwrap(),
            "source": self.engine.source.lock().unwrap().clone(),
            "mic": self.engine.mic.lock().unwrap().clone(),
            "mix": self.engine.mix.lock().unwrap().clone(),
            "quality": self.engine.quality.lock().unwrap().clone(),
            "language": *self.language.lock().unwrap(),
            "layout": *self.layout.lock().unwrap(),
            "hotkey": *self.hotkey.lock().unwrap(),
            "hid_once": self.hid_once.load(Ordering::Relaxed),
        });
        let _ = write_whole(&self.dir.join("settings.json"), value.to_string().as_bytes());
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
    names.sort_by_key(|n| n.to_lowercase());
    names.dedup();
    names
}

#[tauri::command]
async fn state(core: State<'_, Arc<Core>>) -> Result<Value, String> {
    Ok(room::state_json(&core))
}

fn display_sizes() -> Vec<(u32, u32)> {
    windows_capture::monitor::Monitor::enumerate().unwrap_or_default().iter().map(|m| (m.width().unwrap_or(0), m.height().unwrap_or(0))).collect()
}

fn with_sizes(mut list: Vec<Value>, sizes: &[(u32, u32)]) -> Vec<Value> {
    for s in list.iter_mut() {
        let index = s["id"]["index"].as_u64().unwrap_or(0) as usize;
        if let Some((w, h)) = index.checked_sub(1).and_then(|i| sizes.get(i)).filter(|(w, h)| *w > 0 && *h > 0) {
            s["width"] = json!(w);
            s["height"] = json!(h);
        }
    }
    list
}

#[tauri::command]
async fn list_sources() -> Vec<Value> {
    with_sizes(sources::list(), &display_sizes())
}

#[tauri::command]
fn set_source(core: State<Arc<Core>>, source: Value) -> Result<(), String> {
    if !sources::valid(&source) || source.is_null() {
        return Err("That capture source is not available.".into());
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
        "app" => record::APP,
        "mic" => record::MIC,
        _ => return Err(format!("unknown kind {kind}")),
    };
    let key = core.engine.mix_key(track);
    core.engine.mix.lock().unwrap()[&key] = json!({"volume": volume.clamp(0.0, 1.0), "muted": muted});
    core.save_settings();
    Ok(())
}

#[tauri::command]
fn set_quality(core: State<Arc<Core>>, id: String) -> Result<(), String> {
    if !record::QUALITIES.iter().any(|q| q.0 == id) {
        return Err(format!("unknown quality {id}"));
    }
    *core.engine.quality.lock().unwrap() = id;
    core.save_settings();
    Ok(())
}

#[tauri::command]
fn set_language(core: State<Arc<Core>>, language: String) -> Result<(), String> {
    if !["", "en", "ja"].contains(&language.as_str()) {
        return Err(format!("unknown language {language}"));
    }
    *core.language.lock().unwrap() = language;
    core.save_settings();
    shell::rebuild_menus(&core.app, &core);
    Ok(())
}

#[tauri::command]
fn rename_room(core: State<Arc<Core>>, name: String) -> Result<(), String> {
    room::rename_room(&core, &name)
}

#[tauri::command]
async fn check_room(core: State<'_, Arc<Core>>) -> Result<Vec<Value>, String> {
    let core = core.inner().clone();
    warm(&core).await;
    let mine = {
        let core = core.clone();
        let pid = sources::pid_of(&core.audio_process.lock().unwrap().clone());
        tauri::async_runtime::spawn_blocking(move || check::run(&core, pid))
    };
    let others = net::check_peers(&core).await;
    let mut out = vec![json!({"id": core.me(), "lines": mine.await.map_err(|e| e.to_string())?})];
    out.extend(others);
    Ok(out)
}

#[tauri::command]
fn draw(core: State<Arc<Core>>, stroke: Value) {
    if core.party.lock().unwrap().on {
        net::broadcast(&core, json!({"t": "draw", "room": core.active_room_id(), "stroke": stroke}));
    }
}

#[tauri::command]
async fn open_debug(app: AppHandle) {
    shell::open_debug(&app);
}

#[tauri::command]
fn party_typing(core: State<Arc<Core>>, on: bool) {
    core.party.lock().unwrap().typing = on;
    party::send(&core);
}

pub async fn warm(core: &Core) {
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
async fn delete_set(core: State<'_, Arc<Core>>, room: String, set: String) -> Result<(), String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || room::delete_set(&core, &room, &set)).await.map_err(|e| e.to_string())?
}

fn update_allowed(core: &Core) -> Result<(), String> {
    if core.active_set.lock().unwrap().is_some() {
        return Err("Stop recording before updating.".into());
    }
    if core.export.lock().unwrap().is_some() {
        return Err("Wait for the export to finish before updating.".into());
    }
    Ok(())
}

#[tauri::command]
async fn install_update(core: State<'_, Arc<Core>>) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    update_allowed(&core)?;
    let update = core.app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?.ok_or("No update is available.")?;
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    update_allowed(&core)?;
    jobs::cancel_job();
    update.install(bytes).map_err(|e| e.to_string())
}

#[tauri::command]
fn open_set_folder(core: State<Arc<Core>>, room: String, set: String) {
    if room::valid_id(&room) && room::valid_id(&set) {
        data::open_folder(&core.dir.join("rooms").join(&room).join(core.me()));
    }
}

#[tauri::command]
async fn export_set(core: State<'_, Arc<Core>>, room: String, set: String) -> Result<Value, String> {
    if !room::valid_id(&room) || !room::valid_id(&set) {
        return Err("That session is not on this PC.".into());
    }
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        export::check(&core)?;
        let name = format!("{set}.mp4");
        let Some(path) = (if test_mode() { Some(core.dir.join(name)) } else { shell::save_file(&core.app, &name, "MP4", "mp4") }) else { return Ok(Value::Null) };
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
    room::log_set_layout(&core);
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
            let token = room::random_hex(room::TOKEN_BYTES);
            invites.push((token.clone(), room));
            token
        }
    };
    format!("ezh-{}-{token}", core.ep.id())
}

#[tauri::command]
fn cancel_invites(core: State<Arc<Core>>) {
    core.invites.lock().unwrap().clear();
    core.asks.lock().unwrap().clear();
    net::broadcast(&core, net::asks_message(&core));
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
async fn leave_room(core: State<'_, Arc<Core>>, id: String) -> Result<(), String> {
    let core = core.inner().clone();
    if core.active_set.lock().unwrap().as_ref().is_some_and(|s| s.room == id) {
        let c = core.clone();
        tauri::async_runtime::spawn_blocking(move || room::end_set(&c, false, None)).await.map_err(|e| e.to_string())?;
    }
    if core.room_id().as_deref() == Some(id.as_str()) {
        net::farewell(&core).await;
    }
    room::leave_room(&core, &id);
    Ok(())
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
fn event_action(core: State<Arc<Core>>, kind: String, id: i64, text: String) {
    let kind = match kind.as_str() {
        "edit" => "event_edit",
        "delete" => "event_del",
        "restore" => "event_restore",
        _ => return,
    };
    let text = clean_note(&text, BOARD_MAX);
    core.append_log(&core.active_room_id(), &json!({"k": kind, "id": id, "t_us": now_us(), "text": text, "tags": room::tags(&text)}));
    party::send(&core);
}

#[tauri::command]
fn log_layout(core: State<Arc<Core>>, layout: Value) {
    core.append_log(&core.active_room_id(), &json!({"k": "layout", "t_us": now_us(), "layout": layout}));
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
async fn reset(core: State<'_, Arc<Core>>, everything: bool, confirm: String) -> Result<(), String> {
    if everything && confirm != "RESET" {
        return Err("Type RESET to confirm.".into());
    }
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        room::stop_set(&core);
        export::cancel(&core);
        data::reset(&core.dir, everything);
        core.app.restart()
    })
    .await
    .map_err(|e| e.to_string())
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
fn add_event(core: State<Arc<Core>>, kind: String, text: String, target: Value, values: Value) -> Result<i64, String> {
    if !EVENT_KINDS.contains(&kind.as_str()) || !target.is_object() || values.to_string().len() > VALUES_MAX {
        return Err(format!("not a valid {kind} event"));
    }
    let text = clean_note(&text, if kind == "board" { BOARD_MAX } else { NOTE_MAX });
    if kind == "note" && text.trim().is_empty() {
        return Err("Write something first.".into());
    }
    Ok(log_event(&core, &core.active_room_id(), &kind, &text, target, values))
}

fn log_event(core: &Arc<Core>, room: &str, kind: &str, text: &str, target: Value, values: Value) -> i64 {
    let id = now_us();
    core.append_log(room, &json!({"k": "event", "id": id, "t_us": id, "kind": kind, "target": target, "text": text, "values": values, "tags": room::tags(text)}));
    party::send(core);
    id
}

pub fn mark_moment(core: &Arc<Core>) -> Result<i64, String> {
    let room = core.active_set.lock().unwrap().as_ref().map(|s| s.room.clone()).ok_or("Start a session first.")?;
    Ok(log_event(core, &room, "mark", "", json!({"t_us": now_us()}), Value::Null))
}

#[tauri::command]
fn mark(core: State<Arc<Core>>) -> Result<i64, String> {
    mark_moment(&core)
}

#[tauri::command]
fn set_hotkey(core: State<Arc<Core>>, keys: String) -> Result<(), String> {
    shell::set_hotkey(&core, &keys)
}

#[tauri::command]
fn retry_ffmpeg() {
    ffmpeg::retry();
}

#[tauri::command]
fn answer_ask(core: State<Arc<Core>>, id: String, approve: bool) {
    net::answer(&core, &id, approve);
}

#[tauri::command]
fn focus_set(core: State<Arc<Core>>, set: Option<String>) {
    *core.focus.lock().unwrap() = set;
}

#[tauri::command]
fn party_on(core: State<Arc<Core>>, on: bool, view: Value) {
    let me = core.me();
    core.party.lock().unwrap().turn(on, &me, &view, &core.active_room_id(), now_us());
    party::send(&core);
}

#[tauri::command]
fn party_act(core: State<Arc<Core>>, view: Value) {
    let me = core.me();
    core.party.lock().unwrap().act(&me, &view, now_us());
    party::send(&core);
}

async fn init(app: AppHandle) -> Result<Arc<Core>, Box<dyn std::error::Error>> {
    let dir = match std::env::var_os("EZH_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => app.path().app_local_data_dir()?,
    };
    std::fs::create_dir_all(&dir)?;
    debug::init(&dir);
    ffmpeg::set_data_dir(dir.clone());
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
        if dir.join("rooms.json").exists() || dir.join("settings.json").exists() {
            recovery = Some("key.bin was missing, so this PC now has a new identity. Import your identity (File menu) to keep your old one.".into());
        }
        let key = SecretKey::generate();
        write_whole(&key_path, &key.to_bytes())?;
        key
    };
    let mut builder = Endpoint::builder(presets::N0).secret_key(key).alpns(vec![ALPN.to_vec()]);
    if std::env::var_os("EZH_RELAY_ONLY").is_some() {
        builder = builder.clear_ip_transports();
    }
    let ep = builder.bind().await?;
    std::fs::write(dir.join("id.txt"), ep.id().to_string())?;
    let rooms = room::read_rooms(&read_json(&dir.join("rooms.json")));
    data::repair_logs(&dir);
    let settings = clean_settings(read_json(&dir.join("settings.json")));
    Ok(Arc::new(Core {
        dir,
        ep,
        app,
        peers: Mutex::new(Vec::new()),
        name: Mutex::new(settings["name"].as_str().map(clean_name).unwrap_or_default()),
        share: AtomicBool::new(settings["share"].as_bool().unwrap_or(true)),
        limit_mbps: Mutex::new(settings["limit_mbps"].as_f64()),
        invites: Mutex::new(Vec::new()),
        rooms: Mutex::new(rooms),
        active_set: Mutex::new(None),
        tz_offset_s: data::tz_offset_seconds(),
        audio_process: Mutex::new(settings["audio_process"].as_str().unwrap_or_default().to_string()),
        keep_days: Mutex::new(settings["keep_days"].as_u64().map(|d| d as u32)),
        codecs: Mutex::new(default_codecs()),
        following: Mutex::new(HashSet::new()),
        recovery: Mutex::new(recovery),
        notice: Mutex::new(String::new()),
        free_bytes: AtomicU64::new(0),
        total_bytes: AtomicU64::new(0),
        deleted: Mutex::new(strings(&settings["deleted_sets"])),
        pacer: Mutex::new(net::Pacer::new()),
        previous_version,
        ffmpeg_status: Mutex::new(String::new()),
        events: Mutex::new(VecDeque::new()),
        started: Instant::now(),
        engine: record::Engine::new(&settings),
        layout: Mutex::new(settings["layout"].clone()),
        export: Mutex::new(None),
        log_lock: Mutex::new(()),
        wake: tokio::sync::Notify::new(),
        focus: Mutex::new(None),
        asks: Mutex::new(Vec::new()),
        party: Mutex::new(party::Party::default()),
        language: Mutex::new(settings["language"].as_str().unwrap_or_default().to_string()),
        hotkey: Mutex::new(settings["hotkey"].as_str().filter(|h| shell::hotkey(h).is_some()).unwrap_or(shell::DEFAULT_HOTKEY).to_string()),
        hid_once: AtomicBool::new(settings["hid_once"].as_bool().unwrap_or(false)),
        logs: Mutex::new(HashMap::new()),
        jsons: Mutex::new(HashMap::new()),
        writing: Mutex::new(HashSet::new()),
        indexes: Mutex::new(HashMap::new()),
    }))
}

pub fn clean_settings(mut settings: Value) -> Value {
    if !settings.is_object() {
        settings = json!({});
    }
    if !settings["limit_mbps"].as_f64().is_some_and(|v| v.is_finite() && v > 0.0) {
        settings["limit_mbps"] = Value::Null;
    }
    if !settings["keep_days"].as_u64().is_some_and(|d| d > 0 && d <= u32::MAX as u64) {
        settings["keep_days"] = Value::Null;
    }
    if !sources::valid(&settings["source"]) {
        settings["source"] = Value::Null;
    }
    if !settings["language"].as_str().is_some_and(|l| ["", "en", "ja"].contains(&l)) {
        settings["language"] = json!("");
    }
    if !settings["quality"].as_str().is_some_and(|q| record::QUALITIES.iter().any(|(id, _)| *id == q)) {
        settings["quality"] = Value::Null;
    }
    settings
}

fn main() {
    if let Some(code) = plugins::child() {
        std::process::exit(code);
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_window_state::Builder::default().with_denylist(&["debug"]).build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        if let Some(core) = app.try_state::<Arc<Core>>() {
                            shell::marked(core.inner());
                        }
                    }
                })
                .build(),
        )
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                match window.app_handle().try_state::<Arc<Core>>() {
                    Some(core) if window.label() == "main" => shell::close_main(window.app_handle(), core.inner()),
                    _ => {
                        let _ = window.hide();
                    }
                }
            }
        })
        .register_asynchronous_uri_scheme_protocol("ezh", |ctx, request, responder| {
            let core = ctx.app_handle().try_state::<Arc<Core>>().map(|c| c.inner().clone());
            std::thread::spawn(move || match core {
                Some(core) => responder.respond(stream::respond(&core, &request)),
                None => responder.respond(tauri::http::Response::builder().status(503).body(Vec::new()).unwrap()),
            });
        })
        .setup(|app| {
            open_main(app)?;
            let core = tauri::async_runtime::block_on(init(app.handle().clone()))?;
            app.manage(core.clone());
            shell::setup(app, core.clone())?;
            record::recover(&core);
            let c = core.clone();
            std::thread::spawn(move || data::keeper(c));
            tauri::async_runtime::spawn(net::accept_loop(core.clone()));
            tauri::async_runtime::spawn(party::repeat(core.clone()));
            room::apply_active(&core);
            let c = core.clone();
            std::thread::spawn(move || record::supervise(c));
            let c = core.clone();
            std::thread::spawn(move || loop {
                data::refresh_disk(&c);
                std::thread::sleep(DISK_EVERY);
            });
            let c = core.clone();
            std::thread::spawn(move || ffmpeg::prepare(&c.ffmpeg_status));
            let c = core.clone();
            std::thread::spawn(move || jobs::worker(c));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![state, check, list_processes, debug::debug_view, debug::debug_report, debug::set_report_sharing, save_report, start_recording, stop_recording, delete_set, open_set_folder, export_set, cancel_export, set_layout, remove_peer, create_invite, create_room, join_room, switch_room, leave_room, set_share, set_limit, set_codecs, cancel_invites, list_sources, set_source, use_display, list_mics, set_mic, set_mix, set_audio_process, set_keep_days, event_action, log_layout, set_name, open_data_folder, data_info, reset, ui_ready, dismiss_recovery, add_event, answer_ask, focus_set, party_on, party_act, jobs::room_plugins, jobs::set_room_plugin, jobs::prepare_plugin, jobs::jobs, jobs::retry_job, jobs::cancel_job, install_update, set_quality, set_language, rename_room, check_room, draw, party_typing, open_debug, mark, set_hotkey, retry_ffmpeg])
        .run(tauri::generate_context!())
        .expect("failed to run");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_files_may_start_with_a_byte_order_mark() {
        let path = std::env::temp_dir().join(format!("ezh-bom-{}.json", std::process::id()));
        std::fs::write(&path, "\u{feff}{\"name\": \"Ren\"}").unwrap();
        let value: Value = read_json(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(value["name"], "Ren");
    }

    #[test]
    fn displays_get_their_size_and_windows_do_not() {
        let list = vec![json!({"id": {"kind": "window", "exe": "a.exe", "title": "A"}}), json!({"id": {"kind": "display", "index": 2}}), json!({"id": {"kind": "display", "index": 3}})];
        let out = with_sizes(list, &[(1920, 1080), (2560, 1440)]);
        assert!(out[0]["width"].is_null());
        assert_eq!((out[1]["width"].as_u64(), out[1]["height"].as_u64()), (Some(2560), Some(1440)));
        assert!(out[2]["width"].is_null());
    }

    #[test]
    fn settings_from_older_versions_fall_back_to_defaults() {
        let old = clean_settings(json!({"name": "Ren", "limit_mbps": 0, "keep_days": 0, "source": "display:1", "language": "fr", "quality": "ultra", "obs_port": 4455}));
        assert_eq!(old["name"], "Ren");
        assert!(old["limit_mbps"].is_null() && old["keep_days"].is_null() && old["source"].is_null() && old["quality"].is_null());
        assert_eq!(old["language"], "");
        let kept = clean_settings(json!({"limit_mbps": 8.5, "keep_days": 30, "source": {"kind": "display", "index": 1}, "language": "ja", "quality": "high"}));
        assert_eq!((kept["limit_mbps"].as_f64(), kept["keep_days"].as_u64(), kept["language"].as_str(), kept["quality"].as_str()), (Some(8.5), Some(30), Some("ja"), Some("high")));
        assert_eq!(kept["source"]["index"], 1);
        assert_eq!(clean_settings(json!([1, 2])), clean_settings(json!({})));
    }

    #[test]
    fn rooms_from_older_versions_keep_what_still_reads() {
        let rooms = room::read_rooms(&json!({"active": "aa11", "list": [{"id": "aa11", "name": "Team", "members": ["x"]}, {"id": "bb22", "title": "old shape"}, {"id": "../x", "name": "bad", "members": []}], "version": 3}));
        assert_eq!(rooms.list.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(), ["aa11"]);
        assert_eq!(rooms.active.as_deref(), Some("aa11"));
        assert!(room::read_rooms(&json!("rooms")).list.is_empty());
    }

    #[test]
    fn whole_writes_replace_the_file_and_leave_no_part() {
        let dir = log::tests::temp("whole");
        let path = dir.join("rooms.json");
        write_whole(&path, b"old").unwrap();
        write_whole(&path, b"new").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert!(!room::part_of(&path).exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}

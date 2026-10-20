#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod audio;
mod bundle;
mod capture;
mod check;
mod data;
mod debug;
mod download;
mod export;
mod ffmpeg;
mod hw;
mod import;
mod jobs;
mod log;
mod media;
mod net;
mod party;
mod plugins;
mod proc;
mod record;
mod room;
mod session;
mod shell;
mod sources;
mod stream;
mod thumbs;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iroh::endpoint::{presets, Connection};
use iroh::{Endpoint, EndpointId, SecretKey, Watcher};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager, State};

pub const ALPN: &[u8] = b"ez-hanseikai/0";
pub const PROTOCOL: u32 = 5;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub use proc::{BELOW_NORMAL, NO_WINDOW};
const NOTE_MAX: usize = 500;
const BOARD_MAX: usize = 4000;
const VALUES_MAX: usize = 32 << 10;
const EVENT_KINDS: [&str; 3] = ["note", "drawing", "board"];
const NAME_MAX: usize = 24;
pub const ROOM_NAME_MAX: usize = 40;
pub const CHANNELS: [&str; 2] = ["stable", "beta"];
const DISK_EVERY: Duration = Duration::from_secs(20);
const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";
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
    builder.build().map(|window| {
        shell::plain_keys(&window);
        shell::match_caption(&window);
    })
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
    pub offset_us: Option<i64>,
    pub failures: u32,
    pub announce: bool,
    pub codecs: Vec<String>,
    pub info: Option<(String, u32)>,
    pub hw: Value,
    pub sharing: bool,
    pub seen_us: Option<i64>,
    pub have: HashMap<String, u64>,
    pub logs: HashMap<String, u64>,
    pub asks: Vec<Value>,
}

impl Peer {
    pub fn alive(&self) -> Option<&Connection> {
        self.conn.as_ref().filter(|c| c.close_reason().is_none())
    }

    pub fn new(id: EndpointId, name: String) -> Self {
        Self { id, name, conn: None, offset_us: None, failures: 0, announce: true, codecs: default_codecs(), info: None, hw: Value::Null, sharing: false, seen_us: None, have: HashMap::new(), logs: HashMap::new(), asks: Vec::new() }
    }
}

pub struct Core {
    pub dir: PathBuf,
    pub ep: Endpoint,
    pub app: AppHandle,
    pub peers: Mutex<Vec<Peer>>,
    pub name: Mutex<String>,
    pub limit_mbps: Mutex<Option<f64>>,
    pub invites: Mutex<Vec<(String, String)>>,
    pub rooms: Mutex<room::Rooms>,
    pub active_set: Mutex<Option<room::ActiveSet>>,
    pub tz_offset_s: i64,
    pub audio_process: Mutex<String>,
    pub warn_hours: Mutex<u32>,
    pub channel: Mutex<String>,
    pub codecs: Mutex<Vec<String>>,
    pub following: Mutex<HashSet<String>>,
    pub recovery: Mutex<Option<String>>,
    pub notice: Mutex<String>,
    pub free_bytes: AtomicU64,
    pub total_bytes: AtomicU64,
    pub want: Mutex<HashMap<String, bool>>,
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
    pub skip_plugins: Mutex<HashSet<String>>,
    pub api_on: Mutex<bool>,
    pub api_lan: Mutex<bool>,
    pub api_token: Mutex<String>,
    pub api_error: Mutex<String>,
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
            "limit_mbps": *self.limit_mbps.lock().unwrap(),
            "audio_process": *self.audio_process.lock().unwrap(),
            "warn_hours": *self.warn_hours.lock().unwrap(),
            "channel": *self.channel.lock().unwrap(),
            "mic": self.engine.mic.lock().unwrap().clone(),
            "mix": self.engine.mix.lock().unwrap().clone(),
            "quality": self.engine.quality.lock().unwrap().clone(),
            "language": *self.language.lock().unwrap(),
            "layout": *self.layout.lock().unwrap(),
            "hotkey": *self.hotkey.lock().unwrap(),
            "skip_plugins": *self.skip_plugins.lock().unwrap(),
            "api": *self.api_on.lock().unwrap(),
            "api_lan": *self.api_lan.lock().unwrap(),
            "api_token": *self.api_token.lock().unwrap(),
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

    pub fn sharing_live(&self) -> bool {
        self.engine.sink.lock().unwrap().as_ref().is_some_and(|s| !s.voice)
    }

    fn disk_below(&self, share: f64) -> bool {
        let total = self.total_bytes.load(Ordering::Relaxed);
        total > 0 && (self.free_bytes.load(Ordering::Relaxed) as f64) < total as f64 * share
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
    if !sources::valid(&source) {
        return Err("That capture source is not available.".into());
    }
    tracing::info!("capture source chosen: {}", if source.is_null() { "none" } else { source["kind"].as_str().unwrap_or_default() });
    *core.engine.source.lock().unwrap() = source;
    Ok(())
}

#[tauri::command]
fn use_display(core: State<Arc<Core>>) {
    *core.engine.source.lock().unwrap() = sources::primary_display();
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
    core.engine.mix.lock().unwrap()[record::MIX_KEYS[track]] = json!({"volume": volume.clamp(0.0, 1.0), "muted": muted});
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
fn draw(core: State<Arc<Core>>, stroke: Value) {
    if core.party.lock().unwrap().on {
        net::broadcast(&core, json!({"t": "draw", "room": core.active_room_id(), "stroke": stroke}));
    }
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
    tauri::async_runtime::spawn_blocking(move || check::with_text(check::run(&core, sources::pid_of(&audio_process))))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_recording(core: State<'_, Arc<Core>>, debrief: Option<bool>) -> Result<(), String> {
    room::start_set(&core, debrief)
}

#[tauri::command]
async fn stop_recording(core: State<'_, Arc<Core>>) -> Result<(), String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || room::stop_set(&core)).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_set(core: State<'_, Arc<Core>>, room: String, set: String) -> Result<bool, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || room::delete_set(&core, &room, &set)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn discard_set(core: State<'_, Arc<Core>>, room: String, set: String) -> Result<bool, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || room::discard_set(&core, &room, &set)).await.map_err(|e| e.to_string())?
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

pub fn update_url(channel: &str) -> String {
    let file = if channel == CHANNELS[1] { "latest-beta.json" } else { "latest.json" };
    format!("https://github.com/hrs23/ez-hanseikai/releases/download/updater/{file}")
}

async fn find_update(core: &Core) -> Result<Option<tauri_plugin_updater::Update>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let channel = core.channel.lock().unwrap().clone();
    let url = tauri::Url::parse(&update_url(&channel)).map_err(|e| e.to_string())?;
    let updater = core.app.updater_builder().endpoints(vec![url]).and_then(|b| b.build()).map_err(|e| e.to_string())?;
    updater.check().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn check_update(core: State<'_, Arc<Core>>) -> Result<Option<String>, String> {
    Ok(find_update(&core).await?.map(|u| u.version))
}

#[tauri::command]
async fn save_text(core: State<'_, Arc<Core>>, name: String, text: String) -> Result<bool, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let path = if test_mode() { Some(core.dir.join(&name)) } else { shell::save_file(&core.app, &name, "Text", "txt") };
        match path {
            Some(path) => std::fs::write(path, text).map(|_| true).map_err(|e| e.to_string()),
            None => Ok(false),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn install_update(core: State<'_, Arc<Core>>) -> Result<(), String> {
    update_allowed(&core)?;
    let update = find_update(&core).await?.ok_or("No update is available.")?;
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    update_allowed(&core)?;
    tracing::info!("installing version {}", update.version);
    let c = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || jobs::stop_and_wait(&c)).await.map_err(|e| e.to_string())?;
    data::unlock(&core.dir);
    let installed = update.install(bytes).map_err(|e| e.to_string());
    if installed.is_err() {
        let _ = data::lock(&core.dir);
    }
    installed
}

#[tauri::command]
fn identity(core: State<Arc<Core>>, export: bool) {
    let core = core.inner().clone();
    std::thread::spawn(move || if export { shell::export_identity(&core.app, &core) } else { shell::import_identity(&core.app, &core) });
}

#[tauri::command]
fn download_set(core: State<Arc<Core>>, room: String, set: String) -> Result<(), String> {
    room::download_set(&core, &room, &set)
}

#[tauri::command]
fn open_set_folder(core: State<Arc<Core>>, room: String, set: String) {
    if room::valid_id(&room) && room::valid_id(&set) {
        let dir = core.recording_dir(&room, &set);
        let _ = std::fs::create_dir_all(&dir);
        data::open_folder(&dir);
    }
}

#[tauri::command]
async fn export_set(core: State<'_, Arc<Core>>, room: String, set: String) -> Result<Value, String> {
    if !room::valid_id(&room) || !room::valid_id(&set) {
        return Err("That recording is not on this PC.".into());
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
async fn pick_import(core: State<'_, Arc<Core>>, folder: bool) -> Result<Value, String> {
    let app = core.app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if folder {
            shell::pick_folder(&app).map_or(Value::Null, |p| json!({"folder": p.to_string_lossy()}))
        } else {
            let files = shell::pick_files(&app, "Video", &session::VIDEO_EXTENSIONS.iter().chain(session::AUDIO_EXTENSIONS.iter()).copied().collect::<Vec<_>>());
            if files.is_empty() { Value::Null } else { json!({"files": files.iter().map(|f| f.to_string_lossy()).collect::<Vec<_>>()}) }
        }
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn import_plan(spec: Value) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || import::plan_from_spec(&spec).map(|p| import::plan_json(&p))).await.map_err(|e| e.to_string())?
}

#[tauri::command]
async fn import_recording(core: State<'_, Arc<Core>>, spec: Value) -> Result<Value, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || import::run(&core, &spec)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn cancel_import() {
    import::cancel();
}

#[tauri::command]
async fn export_folder(core: State<'_, Arc<Core>>, room: String, set: String, aligned: bool) -> Result<Value, String> {
    if !room::valid_id(&room) || !room::valid_id(&set) {
        return Err("That recording is not on this PC.".into());
    }
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        export::check(&core)?;
        let parent = if test_mode() { Some(core.dir.join("bundles")) } else { shell::pick_folder(&core.app) };
        let Some(parent) = parent else { return Ok(Value::Null) };
        let dir = (0..).map(|i| parent.join(if i == 0 { set.clone() } else { format!("{set} {}", i + 1) })).find(|d| !d.exists()).unwrap_or_default();
        bundle::run(&core, &room, &set, &dir, aligned)
    })
    .await
    .map_err(|e| e.to_string())?
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
fn set_warn_hours(core: State<Arc<Core>>, hours: u32) {
    *core.warn_hours.lock().unwrap() = hours;
    core.save_settings();
}

#[tauri::command]
fn set_channel(core: State<Arc<Core>>, channel: String) -> Result<(), String> {
    if !CHANNELS.contains(&channel.as_str()) {
        return Err(format!("unknown channel {channel}"));
    }
    *core.channel.lock().unwrap() = channel;
    core.save_settings();
    Ok(())
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

async fn quit_room(core: &Arc<Core>, id: &str) -> Result<(), String> {
    if core.active_set.lock().unwrap().as_ref().is_some_and(|s| s.room == id) {
        let c = core.clone();
        tauri::async_runtime::spawn_blocking(move || room::end_set(&c, false, None)).await.map_err(|e| e.to_string())?;
    }
    if core.room_id().as_deref() == Some(id) {
        net::farewell(core).await;
    }
    Ok(())
}

#[tauri::command]
async fn leave_room(core: State<'_, Arc<Core>>, id: String) -> Result<(), String> {
    let core = core.inner().clone();
    quit_room(&core, &id).await?;
    room::close_room(&core, &id);
    Ok(())
}

#[tauri::command]
async fn remove_room(core: State<'_, Arc<Core>>, id: String) -> Result<(), String> {
    let core = core.inner().clone();
    if !core.rooms.lock().unwrap().list.iter().any(|r| r.id == id) {
        return Err("That room is not in your list.".into());
    }
    if core.export.lock().unwrap().as_ref().is_some_and(|e| e.set.0 == id) {
        return Err("This recording is being exported. Cancel the export first.".into());
    }
    quit_room(&core, &id).await?;
    tauri::async_runtime::spawn_blocking(move || {
        room::leave_room(&core, &id);
        jobs::release_room(&id);
        data::remove_room(&core, &id);
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
async fn check_peer(core: State<'_, Arc<Core>>, id: String) -> Result<Value, String> {
    let core = core.inner().clone();
    if id == core.me() {
        warm(&core).await;
        let pid = sources::pid_of(&core.audio_process.lock().unwrap().clone());
        let c = core.clone();
        return tauri::async_runtime::spawn_blocking(move || json!(check::run(&c, pid))).await.map_err(|e| e.to_string());
    }
    Ok(net::check_peer(&core, &id).await)
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
async fn reset(core: State<'_, Arc<Core>>, keep_rooms: bool) -> Result<(), String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        tracing::info!("resetting the app{}", if keep_rooms { ", keeping the rooms" } else { "" });
        room::stop_set(&core);
        export::cancel(&core);
        jobs::stop_and_wait(&core);
        data::unlock(&core.dir);
        data::reset(&core.dir, keep_rooms);
        core.app.restart()
    })
    .await
    .map_err(|e| e.to_string())
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
    core.append_log(room, &room::user_event(id, kind, text, target, values));
    party::send(core);
    id
}

pub fn mark_moment(core: &Arc<Core>) -> Result<i64, String> {
    let room = core.active_set.lock().unwrap().as_ref().map(|s| s.room.clone()).ok_or("Start recording first.")?;
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

fn enable_api(core: &Arc<Core>, on: bool) {
    *core.api_on.lock().unwrap() = on;
    if on && core.api_token.lock().unwrap().is_empty() {
        *core.api_token.lock().unwrap() = room::random_hex(api::TOKEN_BYTES);
    }
    core.save_settings();
    api::apply(core);
}

#[tauri::command]
fn set_api(core: State<Arc<Core>>, on: bool) {
    enable_api(&core, on);
}

#[tauri::command]
fn set_api_lan(core: State<Arc<Core>>, lan: bool) {
    *core.api_lan.lock().unwrap() = lan;
    core.save_settings();
    api::apply(&core);
}

#[tauri::command]
fn new_api_token(core: State<Arc<Core>>) {
    *core.api_token.lock().unwrap() = room::random_hex(api::TOKEN_BYTES);
    core.save_settings();
    api::refresh(&core);
}

#[tauri::command]
fn api_info(core: State<Arc<Core>>) -> Value {
    let on = *core.api_on.lock().unwrap();
    let lan = *core.api_lan.lock().unwrap();
    json!({"on": on, "lan": lan, "address": api::address(), "hosts": if on && lan { api::lan_addresses() } else { Vec::new() }, "token": if on { core.api_token.lock().unwrap().clone() } else { String::new() }, "error": core.api_error.lock().unwrap().clone()})
}

#[tauri::command]
fn open_link(url: String) {
    shell::open_link(&url);
}

#[tauri::command]
fn open_llms() {
    api::open_docs();
}

#[tauri::command]
fn api_reference() -> Vec<Value> {
    api::reference()
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
    party::log(&core);
}

fn data_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
    match std::env::var_os("EZH_DIR") {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => app.path().app_local_data_dir(),
    }
}

async fn init(app: AppHandle, dir: PathBuf, left_lock: bool) -> Result<Arc<Core>, Box<dyn std::error::Error>> {
    debug::init(&dir);
    tracing::info!("EZ Hanseikai {VERSION} started, process {}, data folder {}", std::process::id(), dir.display());
    ffmpeg::set_data_dir(dir.clone());
    let mut recovery = None;
    if left_lock {
        tracing::warn!("the app did not exit cleanly last time");
    }
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
        limit_mbps: Mutex::new(settings["limit_mbps"].as_f64()),
        invites: Mutex::new(Vec::new()),
        rooms: Mutex::new(rooms),
        active_set: Mutex::new(None),
        tz_offset_s: data::tz_offset_seconds(),
        audio_process: Mutex::new(settings["audio_process"].as_str().unwrap_or_default().to_string()),
        warn_hours: Mutex::new(settings["warn_hours"].as_u64().map_or(data::WARN_HOURS, |h| h as u32)),
        channel: Mutex::new(settings["channel"].as_str().unwrap_or(CHANNELS[0]).to_string()),
        codecs: Mutex::new(default_codecs()),
        following: Mutex::new(HashSet::new()),
        recovery: Mutex::new(recovery),
        notice: Mutex::new(String::new()),
        free_bytes: AtomicU64::new(0),
        total_bytes: AtomicU64::new(0),
        want: Mutex::new(HashMap::new()),
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
        skip_plugins: Mutex::new(strings(&settings["skip_plugins"])),
        api_on: Mutex::new(settings["api"] == true),
        api_lan: Mutex::new(settings["api_lan"] == true),
        api_token: Mutex::new(settings["api_token"].as_str().filter(|t| api::token_valid(t)).unwrap_or_default().to_string()),
        api_error: Mutex::new(String::new()),
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
    if !settings["warn_hours"].as_u64().is_some_and(|h| data::WARN_CHOICES.contains(&h)) {
        settings["warn_hours"] = Value::Null;
    }
    if !settings["channel"].as_str().is_some_and(|c| CHANNELS.contains(&c)) {
        settings["channel"] = Value::Null;
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
    let mut builder = tauri::Builder::default();
    if std::env::var_os("EZH_DIR").is_none() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| shell::show_window(app)));
    }
    let app = builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_window_state::Builder::default().build())
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
                    Some(core) if window.label() == "main" => {
                        let (app, core) = (window.app_handle().clone(), core.inner().clone());
                        std::thread::spawn(move || shell::exit_app(&app, &core));
                    }
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
            let dir = data_dir(app.handle())?;
            std::fs::create_dir_all(&dir)?;
            let left_lock = match data::lock(&dir) {
                Ok(left) => left,
                Err(_) => {
                    if !test_mode() {
                        shell::already_running();
                    }
                    std::process::exit(0);
                }
            };
            open_main(app)?;
            let core = tauri::async_runtime::block_on(init(app.handle().clone(), dir, left_lock))?;
            app.manage(core.clone());
            shell::setup(app, core.clone())?;
            record::recover(&core);
            let c = core.clone();
            std::thread::spawn(move || data::keeper(c));
            tauri::async_runtime::spawn(net::accept_loop(core.clone()));
            tauri::async_runtime::spawn(party::repeat(core.clone()));
            room::apply_active(&core);
            if api::requested() || *core.api_on.lock().unwrap() {
                enable_api(&core, true);
            } else {
                api::apply(&core);
            }
            hw::warm();
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
            let c = core.clone();
            std::thread::spawn(move || thumbs::keeper(c));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![state, check, list_processes, debug::debug_view, debug::debug_report, start_recording, stop_recording, delete_set, discard_set, download_set, identity, open_set_folder, export_set, cancel_export, pick_import, import_plan, import_recording, cancel_import, export_folder, set_layout, remove_peer, create_invite, create_room, join_room, switch_room, leave_room, remove_room, check_peer, set_limit, set_codecs, cancel_invites, list_sources, set_source, use_display, list_mics, set_mic, set_mix, set_audio_process, set_warn_hours, set_channel, event_action, log_layout, set_name, open_data_folder, data_info, reset, dismiss_recovery, add_event, answer_ask, focus_set, party_on, party_act, jobs::room_plugins, jobs::set_room_plugin, jobs::set_process_plugin, jobs::set_plugin_runner, jobs::prepare_plugin, jobs::jobs, jobs::retry_job, jobs::cancel_job, install_update, check_update, save_text, jobs::run_job_next, set_quality, set_language, rename_room, draw, party_typing, mark, set_hotkey, retry_ffmpeg, set_api, set_api_lan, new_api_token, api_info, api_reference, open_llms, open_link])
        .build(tauri::generate_context!())
        .expect("failed to start");
    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            if let Some(core) = app.try_state::<Arc<Core>>() {
                tracing::info!("exiting");
                api::forget(&core.dir);
                jobs::stop_now(&core);
                data::unlock(&core.dir);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_update_channel_reads_its_own_manifest() {
        assert!(update_url("stable").ends_with("/updater/latest.json"));
        assert!(update_url("beta").ends_with("/updater/latest-beta.json"));
        assert_eq!(update_url("nonsense"), update_url("stable"));
    }

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
        let old = clean_settings(json!({"name": "Ren", "limit_mbps": 0, "keep_days": 30, "warn_hours": 7, "channel": "nightly", "source": "display:1", "language": "fr", "quality": "ultra", "obs_port": 4455}));
        assert_eq!(old["name"], "Ren");
        assert!(old["limit_mbps"].is_null() && old["warn_hours"].is_null() && old["channel"].is_null() && old["source"].is_null() && old["quality"].is_null());
        assert_eq!(old["language"], "");
        let kept = clean_settings(json!({"limit_mbps": 8.5, "warn_hours": 20, "channel": "beta", "source": {"kind": "display", "index": 1}, "language": "ja", "quality": "high"}));
        assert_eq!((kept["limit_mbps"].as_f64(), kept["warn_hours"].as_u64(), kept["channel"].as_str(), kept["language"].as_str(), kept["quality"].as_str()), (Some(8.5), Some(20), Some("beta"), Some("ja"), Some("high")));
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

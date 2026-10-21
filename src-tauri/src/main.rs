#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod audio;
mod bundle;
mod capture;
mod check;
mod clock;
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
mod ops;
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
use std::time::{Duration, Instant};

use iroh::endpoint::{presets, Connection};
use iroh::{Endpoint, EndpointId, SecretKey};
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
    clock::pc_us() + clock::offset_us()
}

pub fn short(id: impl std::fmt::Display) -> String {
    id.to_string().chars().take(10).collect()
}

fn unwanted(c: char) -> bool {
    c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

pub fn clean_text(raw: &str, max: usize) -> String {
    raw.chars().filter(|&c| !unwanted(c)).collect::<String>().trim().chars().take(max).collect::<String>().trim().to_string()
}

pub fn clean_name(raw: &str) -> String {
    clean_text(raw, NAME_MAX)
}

fn clean_note(raw: &str, max: usize) -> String {
    raw.chars().filter(|&c| !unwanted(c) || matches!(c, '\n' | '\t')).take(max).collect()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn entries(dir: &Path) -> Vec<(String, PathBuf)> {
    std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| (e.file_name().to_string_lossy().to_string(), e.path())).collect()
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
    pub live: Option<net::PeerLive>,
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
        Self { id, name, conn: None, offset_us: None, failures: 0, announce: true, codecs: default_codecs(), info: None, hw: Value::Null, live: None, seen_us: None, have: HashMap::new(), logs: HashMap::new(), asks: Vec::new() }
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
    pub files: data::Files,
    pub writing: Mutex<HashSet<PathBuf>>,
    pub bad: Mutex<HashSet<(String, PathBuf)>>,
    pub removed_rooms: Mutex<HashSet<String>>,
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

async fn off_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn call(core: State<'_, Arc<Core>>, name: String, args: Value) -> Result<Value, String> {
    let op = ops::find(&name).ok_or("No such operation.")?;
    let core = core.inner().clone();
    off_thread(move || op(&core, args)).await?
}

#[tauri::command]
fn identity(core: State<Arc<Core>>, export: bool) {
    let core = core.inner().clone();
    std::thread::spawn(move || if export { shell::export_identity(&core.app, &core) } else { shell::import_identity(&core.app, &core) });
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
fn open_data_folder(core: State<Arc<Core>>) {
    data::open_folder(&core.dir);
}

#[tauri::command]
fn open_link(url: String) {
    shell::open_link(&url);
}

#[tauri::command]
async fn pick_folder(core: State<'_, Arc<Core>>) -> Result<Option<String>, String> {
    let app = core.app.clone();
    off_thread(move || shell::pick_folder(&app).map(|p| p.to_string_lossy().into_owned())).await
}

#[tauri::command]
async fn pick_files(core: State<'_, Arc<Core>>) -> Result<Vec<String>, String> {
    let app = core.app.clone();
    let extensions: Vec<&str> = session::VIDEO_EXTENSIONS.iter().chain(session::AUDIO_EXTENSIONS.iter()).copied().collect();
    off_thread(move || shell::pick_files(&app, "Video", &extensions).iter().map(|f| f.to_string_lossy().into_owned()).collect()).await
}

#[tauri::command]
async fn save_file(core: State<'_, Arc<Core>>, name: String) -> Result<Option<String>, String> {
    let app = core.app.clone();
    off_thread(move || shell::save_file(&app, &name).map(|p| p.to_string_lossy().into_owned())).await
}

#[tauri::command]
async fn save_text(core: State<'_, Arc<Core>>, name: String, text: String) -> Result<bool, String> {
    let app = core.app.clone();
    off_thread(move || match shell::save_file(&app, &name) {
        Some(path) => std::fs::write(path, text).map(|_| true).map_err(|e| e.to_string()),
        None => Ok(false),
    })
    .await?
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
        files: Mutex::new(HashMap::new()),
        writing: Mutex::new(HashSet::new()),
        bad: Mutex::new(HashSet::new()),
        removed_rooms: Mutex::new(HashSet::new()),
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
            data::clear_standby(&core.dir);
            data::note_old_layout(&core);
            let c = core.clone();
            std::thread::spawn(move || data::keeper(c));
            tauri::async_runtime::spawn(net::accept_loop(core.clone()));
            tauri::async_runtime::spawn(party::repeat(core.clone()));
            room::apply_active(&core);
            if api::requested() || *core.api_on.lock().unwrap() {
                ops::enable_api(&core, true);
            } else {
                api::apply(&core);
            }
            hw::warm();
            clock::start();
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
        .invoke_handler(tauri::generate_handler![call, identity, open_set_folder, open_data_folder, open_link, pick_folder, pick_files, save_file, save_text])
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
    fn notes_keep_line_breaks_and_tabs_and_drop_other_control_and_direction_characters() {
        assert_eq!(clean_note("- a\n\t- [ ] b\r\u{7}\u{1b}[0m", BOARD_MAX), "- a\n\t- [ ] b[0m");
        assert_eq!(clean_note("abcdef", 3), "abc");
        assert_eq!(clean_note("[\u{202E} moc.elgoog.ac](https://moc.elgoog.ac) \u{2066}x\u{2069}\u{202A}", BOARD_MAX), "[ moc.elgoog.ac](https://moc.elgoog.ac) x");
        assert_eq!(clean_name("\u{202E}Aki\u{2067}"), "Aki");
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

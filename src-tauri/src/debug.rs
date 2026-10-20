use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use iroh::endpoint::Connection;
use iroh::Watcher;
use regex::Regex;
use serde_json::{json, Value};
use tauri::State;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;

use crate::room::{self, SERIES};
use crate::{check, data, download, ffmpeg, hw, net, now_us, party, record, short, sources, Core, NO_WINDOW};

const MAX_LOG: u64 = 10 << 20;
pub const LOG_FILES: [&str; 3] = ["log.txt", "log.old.txt", "log.older.txt"];
const CRASH: &str = "crash.txt";
const EVENTS_KEPT: usize = 500;
const TRANSFERS_KEPT: usize = 1500;
const CUTS_KEPT: usize = 120;
const FFMPEG_KEPT: usize = 200;
const LOG_TAIL: u64 = 256 << 10;
const REPORT_LINES: usize = 400;
const ERRORS_KEPT: usize = 50;
const JOB_RECORDS_KEPT: usize = 200;
const JOB_LINES: usize = 40;
const NAME_MIN_CHARS: usize = 2;
pub const UPDATE_HOST: &str = "github.com";

pub const ARRIVED: usize = 0;
pub const TAKEN: usize = 1;
pub const ENCODED: usize = 2;
pub const REPEATED: usize = 3;
pub const DROPPED: usize = 4;
pub static FRAMES: [AtomicU64; 5] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

static SHARE_ADDRESSES: AtomicBool = AtomicBool::new(false);
static TRANSFERS: Mutex<Ring> = Mutex::new(Ring::new(TRANSFERS_KEPT));
static CUTS: Mutex<Ring> = Mutex::new(Ring::new(CUTS_KEPT));
static FFMPEG: Mutex<Ring> = Mutex::new(Ring::new(FFMPEG_KEPT));
static ACTIVE: Mutex<Vec<(u64, Value, Instant, Arc<AtomicU64>)>> = Mutex::new(Vec::new());
static MESSAGES: Mutex<BTreeMap<(String, String), (u64, u64)>> = Mutex::new(BTreeMap::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

pub struct Ring {
    items: VecDeque<(u64, Value)>,
    cap: usize,
    seq: u64,
}

impl Ring {
    pub const fn new(cap: usize) -> Self {
        Self { items: VecDeque::new(), cap, seq: 0 }
    }

    pub fn push(&mut self, mut item: Value) -> u64 {
        self.seq += 1;
        item["seq"] = json!(self.seq);
        self.items.push_back((self.seq, item));
        while self.items.len() > self.cap {
            self.items.pop_front();
        }
        self.seq
    }

    pub fn since(&self, seq: u64) -> Vec<Value> {
        self.items.iter().filter(|(s, _)| *s > seq).map(|(_, v)| v.clone()).collect()
    }

    fn rows(&self) -> impl DoubleEndedIterator<Item = &Value> {
        self.items.iter().map(|(_, v)| v)
    }
}

pub struct Event {
    pub t_us: i64,
    pub text: String,
}

#[derive(Clone)]
struct RollingFile {
    dir: PathBuf,
    file: Arc<Mutex<(File, u64)>>,
}

impl RollingFile {
    fn open(dir: &Path) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(dir.join(LOG_FILES[0]))?;
        let len = file.metadata().map(|m| m.len()).unwrap_or(0);
        Ok(Self { dir: dir.to_path_buf(), file: Arc::new(Mutex::new((file, len))) })
    }
}

struct Wall(i64);

impl tracing_subscriber::fmt::time::FormatTime for Wall {
    fn format_time(&self, w: &mut tracing_subscriber::fmt::format::Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", room::wall_clock(now_us(), self.0))
    }
}

impl Write for RollingFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut file = self.file.lock().unwrap();
        if file.1 > MAX_LOG {
            for i in (1..LOG_FILES.len()).rev() {
                let _ = fs::rename(self.dir.join(LOG_FILES[i - 1]), self.dir.join(LOG_FILES[i]));
            }
            *file = (OpenOptions::new().create(true).append(true).open(self.dir.join(LOG_FILES[0]))?, 0);
        }
        let n = file.0.write(buf)?;
        file.1 += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.lock().unwrap().0.flush()
    }
}

impl<'a> MakeWriter<'a> for RollingFile {
    type Writer = RollingFile;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

pub fn init(dir: &Path) {
    let tz = data::tz_offset_seconds();
    let Ok(writer) = RollingFile::open(dir) else { return };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn,ez_hanseikai=debug"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .with_timer(Wall(tz))
        .init();
    let crash = dir.join(CRASH);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let text = format!("{} version {} panic: {info}\n{}\n", room::wall_clock(now_us(), tz), crate::VERSION, std::backtrace::Backtrace::force_capture());
        let _ = OpenOptions::new().create(true).append(true).open(&crash).and_then(|mut f| f.write_all(text.as_bytes()).and_then(|_| f.sync_all()));
        tracing::error!("{text}");
        previous(info);
    }));
}

pub fn event(core: &Core, since: Instant, line: String) {
    let line = format!("{line} (+{:.2}s)", since.elapsed().as_secs_f64());
    tracing::info!("{line}");
    let mut events = core.events.lock().unwrap();
    events.push_back(Event { t_us: now_us(), text: line });
    while events.len() > EVENTS_KEPT {
        events.pop_front();
    }
}

pub fn kind(live: bool, series: &str, n: u32) -> &'static str {
    match (n == 0, live, series == SERIES[1]) {
        (true, _, _) => "init",
        (false, true, true) => "live voice",
        (false, true, false) => "live picture",
        (false, false, true) => "history voice",
        (false, false, false) => "history picture",
    }
}

#[derive(Clone)]
pub struct What {
    row: Value,
    live: bool,
    series: String,
    priority: i32,
}

pub fn what(peer: impl std::fmt::Display, req: &Value, priority: i32) -> What {
    let text = |k: &str| req[k].as_str().unwrap_or_default().to_string();
    let row = json!({"peer": short(peer), "room": text("room"), "set": text("set"), "owner": short(text("owner")), "capture": text("capture"), "series": text("series")});
    What { row, live: req["live"] == true, series: text("series"), priority }
}

pub struct Transfer {
    id: u64,
    row: Value,
    started: Instant,
    bytes: Arc<AtomicU64>,
    done: bool,
}

pub fn start(dir: &str, what: &What, n: u32) -> Transfer {
    let mut row = what.row.clone();
    row["dir"] = json!(dir);
    row["n"] = json!(n);
    row["kind"] = json!(kind(what.live, &what.series, n));
    row["priority"] = json!(what.priority);
    let (id, started, bytes) = (NEXT_ID.fetch_add(1, Ordering::Relaxed), Instant::now(), Arc::new(AtomicU64::new(0)));
    ACTIVE.lock().unwrap().push((id, row.clone(), started, bytes.clone()));
    Transfer { id, row, started, bytes, done: false }
}

impl Transfer {
    pub fn add(&self, bytes: usize) {
        self.bytes.fetch_add(bytes as u64, Ordering::Relaxed);
    }

    pub fn finish(mut self) {
        self.done = true;
    }
}

impl Drop for Transfer {
    fn drop(&mut self) {
        ACTIVE.lock().unwrap().retain(|(id, ..)| *id != self.id);
        let mut row = std::mem::take(&mut self.row);
        row["bytes"] = json!(self.bytes.load(Ordering::Relaxed));
        row["ms"] = json!(self.started.elapsed().as_millis() as u64);
        record(row, self.done);
    }
}

fn same_piece(a: &Value, b: &Value) -> bool {
    ["dir", "owner", "capture", "series", "n"].iter().all(|k| a[k] == b[k])
}

pub fn result(rows: &Ring, row: &Value, done: bool) -> &'static str {
    if !done {
        return "partial";
    }
    let failed_before = rows.rows().rev().find(|r| same_piece(r, row)).is_some_and(|r| r["result"] != "ok");
    if failed_before { "retry" } else { "ok" }
}

fn record(mut row: Value, done: bool) {
    row["t_us"] = json!(now_us());
    let mut rows = TRANSFERS.lock().unwrap();
    row["result"] = json!(result(&rows, &row, done));
    rows.push(row);
}

pub fn mark_bad(rows: &mut Ring, owner: &str, capture: &str, series: &str, n: u32, bytes: u64) {
    let wanted = json!({"dir": "in", "owner": short(owner), "capture": capture, "series": series, "n": n});
    if let Some((_, row)) = rows.items.iter_mut().rev().find(|(_, r)| same_piece(r, &wanted)) {
        row["result"] = json!("hash fail");
        return;
    }
    let mut row = wanted;
    row["kind"] = json!(kind(false, series, n));
    row["bytes"] = json!(bytes);
    row["result"] = json!("hash fail");
    row["t_us"] = json!(now_us());
    rows.push(row);
}

pub fn hash_fail(owner: &str, capture: &str, series: &str, n: u32, bytes: u64) {
    mark_bad(&mut TRANSFERS.lock().unwrap(), owner, capture, series, n, bytes);
}

pub fn queued_out(peer: &str) -> usize {
    ACTIVE.lock().unwrap().iter().filter(|(_, row, ..)| row["dir"] == "out" && row["peer"] == peer).count()
}

pub fn slow_send(what: &What, took: std::time::Duration) {
    let peer = what.row["peer"].as_str().unwrap_or_default();
    if took >= hw::SLOW_SEND {
        hw::slow_send(peer, took, queued_out(peer));
    }
}

pub fn log_transfer(dir: &str, peer: impl std::fmt::Display, room: &str, of: &str, bytes: usize, started: Instant) {
    if bytes == 0 {
        return;
    }
    let row = json!({"dir": dir, "peer": short(peer), "room": room, "owner": short(of), "kind": "log", "bytes": bytes, "ms": started.elapsed().as_millis() as u64});
    record(row, true);
}

pub fn message(dir: &str, kind: Option<&str>, bytes: usize) {
    let mut all = MESSAGES.lock().unwrap();
    let entry = all.entry((dir.to_string(), kind.unwrap_or("?").to_string())).or_default();
    entry.0 += 1;
    entry.1 += bytes as u64;
}

pub fn cut(capture: &str, series: &str, n: u32, set: Option<&str>, bytes: u64, fragments: u32, d_us: i64) {
    let row = json!({"t_us": now_us(), "capture": capture, "series": series, "n": n, "set": set, "bytes": bytes, "fragments": fragments, "d_us": d_us});
    CUTS.lock().unwrap().push(row);
}

pub fn ffmpeg_line(line: &str) {
    FFMPEG.lock().unwrap().push(json!({"t_us": now_us(), "text": line}));
}

pub fn frame(which: usize) {
    FRAMES[which].fetch_add(1, Ordering::Relaxed);
}

pub fn kinds(records: &[Value]) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for r in records {
        *out.entry(r["k"].as_str().unwrap_or("?").to_string()).or_insert(0) += 1;
    }
    out
}

pub fn ranges(numbers: &[u32]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < numbers.len() {
        let mut j = i;
        while j + 1 < numbers.len() && numbers[j + 1] == numbers[j] + 1 {
            j += 1;
        }
        out.push(if i == j { numbers[i].to_string() } else { format!("{}-{}", numbers[i], numbers[j]) });
        i = j + 1;
    }
    out.join(", ")
}

pub fn jobs(logs: &[(String, &[Value])]) -> Vec<Value> {
    let mut found: BTreeMap<String, Value> = BTreeMap::new();
    for (owner, records) in logs {
        for r in records.iter().filter(|r| r["k"] == "job") {
            let Some(id) = r["job"].as_str().map(String::from) else { continue };
            let t = r["t_us"].as_i64().unwrap_or(0);
            let job = found.entry(id.clone()).or_insert_with(|| json!({"job": id, "first_us": t, "t_us": i64::MIN, "records": 0}));
            job["records"] = json!(job["records"].as_u64().unwrap_or(0) + 1);
            for k in ["plugin", "version", "file", "set"] {
                if !r[k].is_null() {
                    job[k] = r[k].clone();
                }
            }
            if job["claimed_by"].is_null() {
                job["claimed_by"] = json!(owner);
            }
            if t >= job["t_us"].as_i64().unwrap_or(i64::MIN) {
                job["t_us"] = json!(t);
                job["state"] = r["state"].clone();
                job["by"] = json!(owner);
                if !r["progress"].is_null() {
                    job["progress"] = r["progress"].clone();
                }
            }
        }
    }
    let mut out: Vec<Value> = found.into_values().collect();
    out.sort_by_key(|j| std::cmp::Reverse(j["t_us"].as_i64()));
    out
}

pub struct Candidate {
    pub name: &'static str,
    pub codec: &'static str,
    pub probe: &'static str,
    pub playable: bool,
    pub chosen: bool,
}

pub fn choice(candidates: &[(&'static str, &'static str, f64)], results: &[(&str, bool)], usable: &[String], chosen: &str) -> Vec<Candidate> {
    candidates
        .iter()
        .map(|(name, codec, _)| Candidate {
            name: *name,
            codec: *codec,
            probe: match results.iter().find(|(n, _)| n == name) {
                Some((_, true)) => "works",
                Some((_, false)) => "not available",
                None if results.is_empty() => "not probed yet",
                None => "skipped",
            },
            playable: usable.iter().any(|u| u == codec),
            chosen: *name == chosen,
        })
        .collect()
}

fn path_json(conn: &Connection, addresses: bool) -> Vec<Value> {
    conn.paths()
        .iter()
        .map(|path| {
            let stats = path.stats();
            json!({
                "kind": if path.is_relay() { "relay" } else if path.is_ip() { "direct" } else { "other" },
                "selected": path.is_selected(),
                "addr": if addresses { format!("{:?}", path.remote_addr()) } else { String::new() },
                "rtt_ms": stats.rtt.as_secs_f64() * 1000.0,
                "tx": stats.udp_tx.bytes,
                "rx": stats.udp_rx.bytes,
                "mtu": stats.current_mtu,
            })
        })
        .collect()
}

fn conn_json(label: &str, conn: &Connection, addresses: bool) -> Value {
    let stats = conn.stats();
    json!({
        "label": label,
        "closed": conn.close_reason().map(|r| r.to_string()),
        "tx": stats.udp_tx.bytes,
        "rx": stats.udp_rx.bytes,
        "lost": stats.lost_packets,
        "paths": path_json(conn, addresses),
    })
}

fn endpoint_json(core: &Core, addresses: bool) -> Value {
    let report = core.ep.net_report().get();
    let relay = format!("{:?}", core.ep.home_relay_status().get());
    let hidden = |text: String| if addresses { text } else { String::new() };
    json!({
        "id": core.me(),
        "relay_connected": relay.contains("Connected"),
        "relay": hidden(relay),
        "sockets": hidden(format!("{:?}", core.ep.bound_sockets())),
        "addr": hidden(format!("{:?}", core.ep.addr())),
        "relay_only": std::env::var_os("EZH_RELAY_ONLY").is_some(),
        "net": report.map(|r| json!({
            "udp_v4": r.udp_v4,
            "udp_v6": r.udp_v6,
            "mapping_varies_v4": r.mapping_varies_by_dest_ipv4,
            "mapping_varies_v6": r.mapping_varies_by_dest_ipv6,
            "captive_portal": r.captive_portal,
            "preferred_relay": hidden(r.preferred_relay.map(|u| u.to_string()).unwrap_or_default()),
            "global_v4": hidden(r.global_v4.map(|a| a.to_string()).unwrap_or_default()),
            "global_v6": hidden(r.global_v6.map(|a| a.to_string()).unwrap_or_default()),
        })),
    })
}

fn connections_json(core: &Core, since: u64, addresses: bool) -> Value {
    let mut peers: Vec<Value> = core
        .peers
        .lock()
        .unwrap()
        .iter()
        .map(|p| {
            let conns: Vec<Value> = p.conn.iter().map(|c| conn_json("connection", c, addresses)).collect();
            json!({
                "id": p.id.to_string(), "short": short(p.id), "name": p.name, "failures": p.failures, "offset_us": p.offset_us,
                "version": p.info.as_ref().map(|i| &i.0), "proto": p.info.as_ref().map(|i| i.1), "codecs": p.codecs, "sharing": p.sharing, "conns": conns,
            })
        })
        .collect();
    for p in peers.iter_mut().filter(|p| p["name"] == "") {
        p["name"] = json!(room::display_name(core, p["id"].as_str().unwrap_or_default()));
    }
    let shown = |text: &str| if addresses { text.to_string() } else { hide_addresses(text) };
    let events: Vec<Value> = core.events.lock().unwrap().iter().filter(|e| e.t_us as u64 > since).map(|e| json!({"t_us": e.t_us, "text": shown(&e.text)})).collect();
    json!({"me": endpoint_json(core, addresses), "proto": crate::PROTOCOL, "peers": peers, "events": events})
}

fn network_json(core: &Core, since: u64) -> Value {
    let rows = TRANSFERS.lock().unwrap();
    let active: Vec<Value> = ACTIVE
        .lock()
        .unwrap()
        .iter()
        .map(|(_, row, started, bytes)| {
            let mut row = row.clone();
            row["bytes"] = json!(bytes.load(Ordering::Relaxed));
            row["ms"] = json!(started.elapsed().as_millis() as u64);
            row
        })
        .collect();
    let messages: Vec<Value> = MESSAGES.lock().unwrap().iter().map(|((dir, kind), (count, bytes))| json!({"dir": dir, "kind": kind, "count": count, "bytes": bytes})).collect();
    json!({"rows": rows.since(since), "seq": rows.seq, "active": active, "messages": messages, "pacer": net::pacer_json(core)})
}

fn recording_json(core: &Core, since: u64) -> Value {
    let usable = record::usable_codecs(core);
    let chosen = ffmpeg::pick(&usable);
    let candidates: Vec<Value> = choice(&ffmpeg::CANDIDATES, &ffmpeg::encoder_results(), &usable, chosen.0)
        .into_iter()
        .map(|c| json!({"name": c.name, "codec": c.codec, "probe": c.probe, "playable": c.playable, "chosen": c.chosen}))
        .collect();
    let frames: Vec<u64> = FRAMES.iter().map(|f| f.load(Ordering::Relaxed)).collect();
    json!({
        "engine": record::debug_json(core),
        "frames": {"arrived": frames[ARRIVED], "taken": frames[TAKEN], "encoded": frames[ENCODED], "repeated": frames[REPEATED], "dropped": frames[DROPPED]},
        "fps": record::FPS,
        "fragments_per_segment": record::FRAGMENTS_PER_SEGMENT,
        "usable": usable,
        "mine": *core.codecs.lock().unwrap(),
        "candidates": candidates,
        "ffmpeg": {"path": ffmpeg::path().display().to_string(), "present": ffmpeg::present(), "status": core.ffmpeg_status.lock().unwrap().clone(), "error": ffmpeg::failure()},
        "cuts": CUTS.lock().unwrap().since(since),
        "stderr": FFMPEG.lock().unwrap().since(0),
        "recording": core.active_set_json(),
    })
}

fn flow_json(core: &Core) -> Value {
    let home = format!("{:?}", core.ep.home_relay_status().get());
    let members: Vec<Value> = core
        .peers
        .lock()
        .unwrap()
        .iter()
        .map(|p| {
            let kind = p.alive().map(room::path_kind);
            json!({"id": p.id.to_string(), "path": kind.as_ref().map(|k| k.0).filter(|k| *k != "none"), "relay": kind.and_then(|k| k.2)})
        })
        .collect();
    json!({
        "members": members,
        "relay": if home.contains("Connected") { room::relay_host(&home) } else { None },
        "downloads": download::active(),
        "update": {"host": UPDATE_HOST, "channel": *core.channel.lock().unwrap()},
        "api": {"on": *core.api_on.lock().unwrap(), "lan": *core.api_lan.lock().unwrap()},
    })
}

pub fn flow_text(flow: &Value, addresses: bool) -> String {
    let host = |h: &Value| match h.as_str() {
        Some(h) if addresses => h.to_string(),
        Some(_) => "<host>".to_string(),
        None => "none".to_string(),
    };
    let mut out = String::from("where data goes\n");
    for m in flow["members"].as_array().into_iter().flatten() {
        let how = match m["path"].as_str() {
            Some("direct") => "direct".to_string(),
            Some(_) => format!("through the relay {}", host(&m["relay"])),
            None => "offline".to_string(),
        };
        out.push_str(&format!("  member {}: {how}\n", short(m["id"].as_str().unwrap_or_default())));
    }
    out.push_str(&format!("  address lookup: relay {}\n", host(&flow["relay"])));
    out.push_str(&format!("  update check: {}, {} channel\n", flow["update"]["host"].as_str().unwrap_or("?"), flow["update"]["channel"].as_str().unwrap_or("?")));
    let downloads: Vec<String> = flow["downloads"].as_array().into_iter().flatten().map(|d| format!("{} from {}", d["key"].as_str().unwrap_or("?"), d["hosts"].as_array().into_iter().flatten().filter_map(|h| h.as_str()).collect::<Vec<_>>().join(", "))).collect();
    out.push_str(&format!("  downloads: {}\n", if downloads.is_empty() { "none now".to_string() } else { downloads.join("; ") }));
    let api = if flow["api"]["on"] != true { "off" } else if flow["api"]["lan"] == true { "on, open to the network" } else { "on, this PC only" };
    out.push_str(&format!("  local API: {api}\n"));
    out
}

fn hardware_text(core: &Core) -> String {
    let peers: Vec<(String, Value, Option<i64>)> = core.peers.lock().unwrap().iter().map(|p| (short(p.id), p.hw.clone(), p.offset_us)).collect();
    let others: Vec<(String, Value)> = peers.iter().map(|(who, hw, _)| (who.clone(), hw.clone())).collect();
    let offsets: Vec<(String, Option<i64>)> = peers.iter().map(|(who, _, offset)| (who.clone(), *offset)).collect();
    hw::report(&hw::share(core), &others, &offsets, &hw::time_json())
}

fn set_detail(core: &Core, room: &str, set: &room::Set) -> Value {
    let peers = room::others_have(core);
    let mine = data::have_counts(core, room, std::slice::from_ref(set));
    let files: Vec<Value> = set
        .files
        .iter()
        .map(|f| {
            let mut series = json!({});
            for s in SERIES {
                let segs = f.series(s);
                let (mut missing, mut unchecked, mut bad, mut ok) = (Vec::new(), Vec::new(), Vec::new(), 0u64);
                for seg in &segs {
                    match data::seg_check(core, room, &f.owner, seg) {
                        Some(true) => ok += 1,
                        Some(false) => bad.push(seg.n),
                        None if core.seg_path(room, &f.owner, seg).exists() => unchecked.push(seg.n),
                        None => missing.push(seg.n),
                    }
                }
                let key = room::have_key(&set.id, &f.owner, s);
                let mut have = json!({"you": mine.get(&key).copied().unwrap_or(0)});
                for (id, h, online) in &peers {
                    have[short(id)] = json!({"n": h.get(&key).copied().unwrap_or(0), "online": online});
                }
                series[s] = json!({"total": segs.len(), "verified": ok, "missing": ranges(&missing), "unchecked": ranges(&unchecked), "bad": ranges(&bad), "have": have});
            }
            json!({"owner": f.owner, "short": short(&f.owner), "name": room::display_name(core, &f.owner), "start_us": f.start_us, "end_us": f.end_us, "captures": f.captures(), "series": series})
        })
        .collect();
    json!({"id": set.id, "files": files})
}

fn data_json(core: &Core, set: Option<&str>) -> Value {
    let active = core.active_room_id();
    let list = core.rooms.lock().unwrap().list.clone();
    let rooms: Vec<Value> = list
        .iter()
        .map(|r| {
            let logs = room::logs(core, &r.id);
            let members: Vec<Value> = logs
                .iter()
                .map(|l| json!({"id": l.owner, "short": short(&l.owner), "name": room::display_name(core, &l.owner), "head": l.records.len(), "bytes": crate::file_len(&core.log_path(&r.id, &l.owner)), "kinds": kinds(&l.records)}))
                .collect();
            let pending = core.dir.join("rooms").join(&r.id).join("pending.bin");
            json!({"id": r.id, "name": r.name, "active": r.id == active, "members": members, "pending": crate::file_len(&pending)})
        })
        .collect();
    let logs = room::logs(core, &active);
    let sets = room::sets(core, &active, &logs);
    let bad: Vec<Value> = logs.iter().flat_map(|l| l.records.iter().filter(|r| r["k"] == "file_bad").map(move |r| json!({"by": short(&l.owner), "record": r}))).collect();
    let summary: Vec<Value> = sets.iter().map(|s| json!({"id": s.id, "live": s.end().is_none(), "files": s.files.len(), "segs": s.files.iter().map(|f| f.segs.len()).sum::<usize>()})).collect();
    let detail = set.and_then(|id| sets.iter().find(|s| s.id == id)).map(|s| set_detail(core, &active, s));
    let party = core.party.lock().unwrap();
    let now = Instant::now();
    let viewers: Value = party
        .viewers
        .iter()
        .map(|(id, v)| (short(id), json!({"set": v.set, "heads": v.heads, "progress": v.progress, "stuck": v.stuck, "seen_ms": now.duration_since(v.seen).as_millis() as u64})))
        .collect::<serde_json::Map<String, Value>>()
        .into();
    let party = json!({"on": party.on, "room": party.room, "lamport": party.lamport, "shared": party.shared, "heads": party.heads, "own": room::holdings(&logs), "viewers": viewers, "repeat_ms": party::REPEAT.as_millis() as u64});
    json!({"rooms": rooms, "sets": summary, "detail": detail, "bad": bad, "party": party})
}

fn jobs_json(core: &Core) -> Value {
    let room = core.active_room_id();
    let logs = room::logs(core, &room);
    let pairs: Vec<(String, &[Value])> = logs.iter().map(|l| (short(&l.owner), l.records.as_slice())).collect();
    let mut records: Vec<Value> = logs
        .iter()
        .flat_map(|l| l.records.iter().filter(|r| r["k"].as_str().is_some_and(|k| k.starts_with("job"))).map(move |r| json!({"by": short(&l.owner), "record": r})))
        .collect();
    records.sort_by_key(|r| std::cmp::Reverse(r["record"]["t_us"].as_i64()));
    records.truncate(JOB_RECORDS_KEPT);
    json!({"jobs": jobs(&pairs), "records": records})
}

pub fn tail(path: &Path, from: u64, max: u64) -> (String, u64, bool) {
    let Ok(mut file) = File::open(path) else { return (String::new(), 0, true) };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let restart = from > len;
    let start = if restart { 0 } else { from }.max(len.saturating_sub(max));
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(start)).is_ok() {
        let _ = file.take(len - start).read_to_end(&mut bytes);
    }
    let skipped = start > from || restart;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if skipped && start > 0 {
        text = text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default();
    }
    (text, len, skipped)
}

fn log_json(core: &Core, from: u64, addresses: bool) -> Value {
    let (text, len, reset) = tail(&core.dir.join("log.txt"), from, LOG_TAIL);
    json!({"text": if addresses { text } else { hide_addresses(&text) }, "len": len, "reset": reset})
}

pub fn view(core: &Core, tab: &str, since: u64, set: Option<&str>, addresses: bool) -> Value {
    match tab {
        "connections" => connections_json(core, since, addresses),
        "network" => network_json(core, since),
        "recording" => recording_json(core, since),
        "data" => data_json(core, set),
        "jobs" => jobs_json(core),
        "flow" => flow_json(core),
        "log" => log_json(core, since, addresses),
        _ => Value::Null,
    }
}

#[tauri::command]
pub async fn debug_view(core: State<'_, Arc<Core>>, tab: String, since: u64, set: Option<String>, addresses: bool) -> Result<Value, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || view(&core, &tab, since, set.as_deref(), addresses)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_report_sharing(on: bool) {
    SHARE_ADDRESSES.store(on, Ordering::Relaxed);
}

#[tauri::command]
pub async fn debug_report(core: State<'_, Arc<Core>>, include_addresses: bool, everyone: bool, audio_process: String, save: bool, head: Option<String>) -> Result<String, String> {
    let core = core.inner().clone();
    let text = diagnostics(core.clone(), include_addresses, everyone, audio_process, head).await?;
    if !save {
        return Ok(text);
    }
    let path = core.dir.join("report.txt");
    fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}

pub async fn diagnostics(core: Arc<Core>, include_addresses: bool, everyone: bool, audio_process: String, head: Option<String>) -> Result<String, String> {
    crate::warm(&core).await;
    let c = core.clone();
    let own = tauri::async_runtime::spawn_blocking(move || {
        let lines = check::run(&c, sources::pid_of(&audio_process));
        let text = report_text(&c, include_addresses, !everyone && include_addresses, Some(&check::text(&lines)));
        match head.filter(|h| !h.is_empty()) {
            Some(h) if include_addresses => format!("{h}\n\n{text}"),
            Some(h) => format!("{}\n\n{text}", redact(&h, &home(), &known_names(&c))),
            None => text,
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(if everyone { combine(own, team(&core).await) } else { own })
}

async fn team(core: &Arc<Core>) -> Vec<(String, String)> {
    let asks: Vec<_> = core
        .connections()
        .into_iter()
        .map(|conn| tokio::spawn(async move {
            let who = short(conn.remote_id());
            let text = match net::request(&conn, &json!({"t": "report"})).await {
                Ok(bytes) if !bytes.is_empty() => String::from_utf8_lossy(&bytes).into_owned(),
                Ok(_) => "no report".to_string(),
                Err(e) => format!("no report: {e}"),
            };
            (who, text)
        }))
        .collect();
    let mut out = Vec::new();
    for ask in asks {
        if let Ok(answer) = ask.await {
            out.push(answer);
        }
    }
    out
}

pub fn combine(own: String, others: Vec<(String, String)>) -> String {
    let mut out = format!("=== this PC ===\n{own}\n");
    for (who, text) in others {
        out.push_str(&format!("\n=== member {who} ===\n{text}\n"));
    }
    out
}

pub fn shared_report(core: &Core) -> String {
    report_text(core, SHARE_ADDRESSES.load(Ordering::Relaxed), false, None)
}

fn describe(conn: &Connection, addresses: bool, out: &mut String) {
    out.push_str(&format!("connection to {}\n", conn.remote_id().fmt_short()));
    for p in path_json(conn, addresses) {
        out.push_str(&format!(
            "  path {}{} {}rtt {:.1} ms sent {} B received {} B mtu {}\n",
            p["kind"].as_str().unwrap_or_default(),
            if p["selected"] == true { " (selected)" } else { "" },
            p["addr"].as_str().filter(|a| !a.is_empty()).map(|a| format!("remote {a} ")).unwrap_or_default(),
            p["rtt_ms"].as_f64().unwrap_or(0.0),
            p["tx"],
            p["rx"],
            p["mtu"]
        ));
    }
    let stats = conn.stats();
    out.push_str(&format!("  total sent {} B received {} B lost packets {}\n", stats.udp_tx.bytes, stats.udp_rx.bytes, stats.lost_packets));
}

pub fn text(core: &Core, peer_addresses: bool) -> String {
    let mut out = String::new();
    out.push_str(&format!("encoder {} ({})\n", ffmpeg::encoder(), ffmpeg::path().display()));
    out.push_str(&format!("encoder probe {:?}\n", ffmpeg::encoder_results()));
    out.push_str(&format!("endpoint {}\n", endpoint_json(core, true)));
    let peers: Vec<(String, Option<Connection>, Option<i64>)> = core.peers.lock().unwrap().iter().map(|p| (p.id.to_string(), p.conn.clone(), p.offset_us)).collect();
    for (id, conn, offset) in peers {
        out.push_str(&format!("peer {id} clock offset {offset:?} us\n"));
        match conn {
            Some(conn) => describe(&conn, peer_addresses, &mut out),
            None => out.push_str("connection: none\n"),
        }
    }
    let room = core.active_room_id();
    let logs = room::logs(core, &room);
    for set in room::sets_json(core, &room, &logs, &[]) {
        out.push_str(&format!("set {} {}\n", set["id"].as_str().unwrap_or_default(), set["status"].as_str().unwrap_or_default()));
        for p in set["players"].as_array().into_iter().flatten() {
            out.push_str(&format!(
                "  {} have {} of {} B in {} parts verified by {}\n",
                short(p["owner"].as_str().unwrap_or_default()),
                p["have"],
                p["total"],
                p["parts"].as_array().map_or(0, |a| a.len()),
                p["verified_by"].as_array().into_iter().flatten().filter_map(|k| k.as_str()).map(short).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    out.push_str("transfers\n");
    for (key, n) in transfer_summary(&TRANSFERS.lock().unwrap()) {
        out.push_str(&format!("  {key}: {n}\n"));
    }
    out.push_str("messages\n");
    for ((dir, kind), (count, bytes)) in MESSAGES.lock().unwrap().iter() {
        out.push_str(&format!("  {dir} {kind}: {count} ({bytes} B)\n"));
    }
    out.push_str("events\n");
    for e in core.events.lock().unwrap().iter() {
        out.push_str(&format!("  {} {}\n", e.t_us / 1000, e.text));
    }
    out
}

pub fn transfer_summary(rows: &Ring) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for r in rows.rows() {
        *out.entry(format!("{} {} {}", r["dir"].as_str().unwrap_or("?"), r["kind"].as_str().unwrap_or("?"), r["result"].as_str().unwrap_or("?"))).or_insert(0) += 1;
    }
    out
}

pub fn command_line(program: &str, args: &[&str]) -> String {
    Command::new(program)
        .args(args)
        .creation_flags(NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|e| format!("unavailable: {e}"))
}

const ADDRESSES: [(&str, &str); 3] = [
    (r"https?://[^\s/\x22,)]+", "<host>"),
    (r"\b\d{1,3}(?:\.\d{1,3}){3}\b", "<ip>"),
    (r"\[?[0-9a-fA-F]{0,4}(?::[0-9a-fA-F]{0,4}){3,7}\]?", "<ip6>"),
];

fn replace_all(text: &str, patterns: &[(&str, &str)]) -> String {
    let mut text = text.to_string();
    for (pattern, replacement) in patterns {
        text = Regex::new(pattern).unwrap().replace_all(&text, *replacement).into_owned();
    }
    text
}

pub fn hide_addresses(text: &str) -> String {
    replace_all(text, &ADDRESSES)
}

const PATH_END: &str = r"[^\s\x22',;)\]}]";

fn hide_home(text: &str, home: &str) -> String {
    let parts: Vec<String> = home.split(['\\', '/']).filter(|p| !p.is_empty()).map(regex::escape).collect();
    if parts.is_empty() {
        return text.to_string();
    }
    Regex::new(&format!(r"(?i){}{PATH_END}*", parts.join(r"[\\/]+"))).unwrap().replace_all(text, "<path>").into_owned()
}

fn home() -> String {
    std::env::var("USERPROFILE").unwrap_or_default()
}

pub fn redact(text: &str, home: &str, names: &[String]) -> String {
    let text = hide_home(&hide_addresses(text), home);
    let mut text = replace_all(&text, &[(format!(r"(?i)\b[a-z]:\\{PATH_END}+").as_str(), "<path>"), (r"\b[0-9a-f]{64}\b", "<id>")]);
    let mut names: Vec<&String> = names.iter().filter(|n| n.chars().count() >= NAME_MIN_CHARS).collect();
    names.sort_by_key(|n| std::cmp::Reverse(n.len()));
    for name in names {
        text = text.replace(name.as_str(), "<name>");
    }
    text
}

fn known_names(core: &Core) -> Vec<String> {
    let mut names = vec![core.name.lock().unwrap().clone()];
    names.extend(core.peers.lock().unwrap().iter().map(|p| p.name.clone()));
    for r in core.rooms.lock().unwrap().list.iter() {
        names.push(r.name.clone());
        names.extend(r.names.values().cloned());
    }
    names
}

pub fn errors(log: &str) -> Vec<&str> {
    let mut out: Vec<&str> = log.lines().filter(|l| l.contains(" WARN ") || l.contains(" ERROR ")).collect();
    out.drain(..out.len().saturating_sub(ERRORS_KEPT));
    out
}

pub fn job_line(job: &Value) -> String {
    let text = |k: &str| job[k].as_str().unwrap_or("?");
    let progress = job["progress"].as_f64().map(|p| format!(" {}%", (p * 100.0).floor())).unwrap_or_default();
    format!("  {} {} {}{} by {}", text("plugin"), text("set"), text("state"), progress, job["by"].as_str().map(String::from).unwrap_or_else(|| "?".into()))
}

fn jobs_text(core: &Core) -> String {
    let jobs = jobs_json(core);
    let lines: Vec<String> = jobs["jobs"].as_array().into_iter().flatten().take(JOB_LINES).map(job_line).collect();
    if lines.is_empty() { "  none".into() } else { lines.join("\n") }
}

pub fn report_text(core: &Core, include_addresses: bool, peer_addresses: bool, check: Option<&str>) -> String {
    let log = fs::read_to_string(core.dir.join("log.txt")).unwrap_or_default();
    let recent: Vec<&str> = log.lines().rev().take(REPORT_LINES).collect();
    let recent: Vec<&str> = recent.into_iter().rev().collect();
    let mut out = String::new();
    out.push_str(&format!("ez-hanseikai {} on {} {}\n", crate::VERSION, std::env::consts::OS, command_line("cmd", &["/c", "ver"])));
    out.push_str(&format!("previous version {}\n", if core.previous_version.is_empty() { "none" } else { &core.previous_version }));
    out.push_str(&format!("addresses and name {}\n", if include_addresses { "included" } else { "removed" }));
    if include_addresses {
        out.push_str(&format!("name {}\n", core.name.lock().unwrap()));
    }
    out.push_str(&format!("ffmpeg {}\n\n", command_line(&ffmpeg::path().to_string_lossy(), &["-version"]).lines().next().unwrap_or("")));
    let infos: Vec<String> = core.peers.lock().unwrap().iter().map(|p| format!("{} {:?}", short(p.id), p.info)).collect();
    out.push_str(&format!("peers {}\n\nenvironment check\n{}\n\n", infos.join(", "), check.unwrap_or("not run")));
    out.push_str(&text(core, peer_addresses));
    out.push_str(&format!("\n{}\n{}", hardware_text(core), flow_text(&flow_json(core), include_addresses)));
    out.push_str("\nrecent errors\n");
    out.push_str(&errors(&log).join("\n"));
    out.push_str("\n\njobs\n");
    out.push_str(&jobs_text(core));
    out.push_str("\n\nrecent crashes\n");
    out.push_str(&tail(&core.dir.join(CRASH), 0, LOG_TAIL).0);
    out.push_str("\n\nrecent log\n");
    out.push_str(&recent.join("\n"));
    if include_addresses { out } else { redact(&out, &home(), &known_names(core)) }
}

pub fn report(core: &Core, include_addresses: bool, check: &str) -> std::io::Result<PathBuf> {
    let path = core.dir.join("report.txt");
    fs::write(&path, report_text(core, include_addresses, include_addresses, Some(check)))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(dir: &str, n: u32, result: &str) -> Value {
        json!({"dir": dir, "owner": "a", "capture": "1", "series": "v", "n": n, "result": result})
    }

    #[test]
    fn the_ring_keeps_the_newest_and_counts_on() {
        let mut ring = Ring::new(3);
        for n in 0..5 {
            ring.push(json!({"n": n}));
        }
        assert_eq!(ring.since(0).iter().map(|r| r["n"].as_u64().unwrap()).collect::<Vec<_>>(), [2, 3, 4]);
        assert_eq!(ring.since(4).len(), 1);
        assert_eq!(ring.seq, 5);
    }

    #[test]
    fn kinds_follow_live_series_and_init() {
        assert_eq!(kind(true, "m", 3), "live voice");
        assert_eq!(kind(true, "v", 3), "live picture");
        assert_eq!(kind(false, "m", 3), "history voice");
        assert_eq!(kind(false, "v", 3), "history picture");
        assert_eq!(kind(true, "v", 0), "init");
    }

    #[test]
    fn a_piece_after_a_failure_is_a_retry() {
        let mut ring = Ring::new(10);
        assert_eq!(result(&ring, &row("in", 1, ""), true), "ok");
        assert_eq!(result(&ring, &row("in", 1, ""), false), "partial");
        ring.push(row("in", 1, "partial"));
        ring.push(row("in", 2, "ok"));
        assert_eq!(result(&ring, &row("in", 1, ""), true), "retry");
        assert_eq!(result(&ring, &row("out", 1, ""), true), "ok");
        assert_eq!(result(&ring, &row("in", 2, ""), true), "ok");
    }

    #[test]
    fn a_hash_failure_marks_the_last_receipt_or_adds_a_row() {
        let mut ring = Ring::new(10);
        ring.push(json!({"dir": "in", "owner": short("abcdefghijklmnop"), "capture": "1", "series": "v", "n": 4, "result": "ok"}));
        mark_bad(&mut ring, "abcdefghijklmnop", "1", "v", 4, 10);
        assert_eq!(ring.since(0)[0]["result"], "hash fail");
        mark_bad(&mut ring, "abcdefghijklmnop", "1", "m", 5, 10);
        let rows = ring.since(0);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1]["kind"], "history voice");
        assert_eq!(result(&ring, &rows[1], true), "retry");
    }

    #[test]
    fn numbers_become_ranges() {
        assert_eq!(ranges(&[]), "");
        assert_eq!(ranges(&[3]), "3");
        assert_eq!(ranges(&[1, 2, 3, 5, 7, 8]), "1-3, 5, 7-8");
    }

    #[test]
    fn records_are_counted_by_kind() {
        let counts = kinds(&[json!({"k": "seg"}), json!({"k": "seg"}), json!({"k": "rec"}), json!({})]);
        assert_eq!(counts.get("seg"), Some(&2));
        assert_eq!(counts.get("rec"), Some(&1));
        assert_eq!(counts.get("?"), Some(&1));
    }

    #[test]
    fn jobs_are_grouped_with_their_latest_state() {
        let a = [json!({"k": "job", "state": "claim", "job": "j1", "plugin": "p", "file": "f", "t_us": 10}), json!({"k": "job", "state": "claim", "job": "j1", "progress": 0.5, "t_us": 20})];
        let b = [json!({"k": "job", "state": "done", "job": "j1", "t_us": 30}), json!({"k": "job", "state": "claim", "job": "j2", "t_us": 5}), json!({"k": "event", "job": "x"})];
        let jobs = jobs(&[("a".into(), &a[..]), ("b".into(), &b[..])]);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0]["job"], "j1");
        assert_eq!(jobs[0]["claimed_by"], "a");
        assert_eq!(jobs[0]["state"], "done");
        assert_eq!(jobs[0]["by"], "b");
        assert_eq!(jobs[0]["progress"], 0.5);
        assert_eq!(jobs[0]["plugin"], "p");
        assert_eq!(jobs[0]["records"], 3);
        assert_eq!(jobs[1]["job"], "j2");
    }

    #[test]
    fn the_encoder_choice_explains_each_candidate() {
        let candidates = [("av1_x", "av1", 0.5), ("hevc_x", "hevc", 0.7), ("hevc_y", "hevc", 0.7), ("h264_x", "h264", 1.0)];
        let rows = choice(&candidates, &[("av1_x", true), ("hevc_x", true), ("h264_x", true)], &["hevc".into(), "h264".into()], "hevc_x");
        assert_eq!(rows.iter().map(|r| r.probe).collect::<Vec<_>>(), ["works", "works", "skipped", "works"]);
        assert_eq!(rows.iter().map(|r| r.playable).collect::<Vec<_>>(), [false, true, true, true]);
        assert_eq!(rows.iter().filter(|r| r.chosen).map(|r| r.name).collect::<Vec<_>>(), ["hevc_x"]);
        assert_eq!(choice(&candidates, &[], &[], "h264_x")[0].probe, "not probed yet");
    }

    #[test]
    fn reports_hide_addresses_ids_and_names() {
        let id = "a".repeat(64);
        let text = format!("from 203.0.113.20:5000 via https://relay.example.net/x by {id} at C:\\Users\\someone\\data named Ren and Rena");
        let out = redact(&text, "", &["Ren".into(), "Rena".into(), "x".into()]);
        assert!(!out.contains("203.0") && !out.contains("relay.example") && !out.contains(&id) && !out.contains("someone") && !out.contains("Ren"), "{out}");
        assert!(out.contains("<name> and <name>") && out.contains("<path> named"), "{out}");
    }

    #[test]
    fn reports_hide_a_user_folder_with_spaces() {
        for home in ["C:\\Users\\Taro Yamada", "C:\\Users\\山田\u{3000}太郎"] {
            let text = format!("data folder {home}\\AppData\\Local\\app ready\nencoder x ({}\\AppData\\ffmpeg.exe)\nopen \"{}\\\\v\"", home.to_lowercase(), home.replace('\\', "\\\\"));
            let out = redact(&text, home, &[]);
            assert!(!out.to_lowercase().contains("yamada") && !out.contains("山田") && !out.contains("太郎"), "{out}");
            assert!(out.contains("data folder <path> ready") && out.contains("(<path>)") && out.contains("open \"<path>\""), "{out}");
        }
    }

    #[test]
    fn a_job_reads_as_one_line() {
        let job = json!({"plugin": "transcribe", "set": "s1", "state": "running", "progress": 0.426, "by": "ab12"});
        assert_eq!(job_line(&job), "  transcribe s1 running 42% by ab12");
        assert_eq!(job_line(&json!({})), "  ? ? ? by ?");
    }

    #[test]
    fn the_page_hides_addresses_but_keeps_the_rest() {
        let out = hide_addresses("Closed { remote_addr: Ip(198.51.100.7:58007) } via https://relay.example.net./ by Ren");
        assert!(!out.contains("198.51") && !out.contains("relay.example"), "{out}");
        assert!(out.contains("Ren") && out.contains("Closed"), "{out}");
    }

    #[test]
    fn errors_keep_the_newest_warnings() {
        let log = (0..ERRORS_KEPT + 5).map(|i| format!("  1.0s  WARN x: w{i}\n  1.0s  INFO x: i{i}")).collect::<Vec<_>>().join("\n");
        let found = errors(&log);
        assert_eq!(found.len(), ERRORS_KEPT);
        assert!(found[0].ends_with("w5") && found.iter().all(|l| l.contains("WARN")));
    }

    #[test]
    fn the_log_tail_resumes_and_restarts_after_a_roll() {
        let dir = crate::log::tests::temp("tail");
        let path = dir.join("log.txt");
        fs::write(&path, "one\ntwo\nthree\n").unwrap();
        let (text, len, reset) = tail(&path, 0, 1000);
        assert_eq!((text.as_str(), len, reset), ("one\ntwo\nthree\n", 14, false));
        let (text, _, reset) = tail(&path, 0, 8);
        assert_eq!((text.as_str(), reset), ("three\n", true));
        fs::write(&path, "one\ntwo\nthree\nfour\n").unwrap();
        assert_eq!(tail(&path, 14, 1000).0, "four\n");
        fs::write(&path, "new\n").unwrap();
        assert_eq!(tail(&path, 19, 1000), ("new\n".to_string(), 4, true));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_flow_text_names_each_route_and_hides_hosts_unless_asked() {
        let flow = json!({
            "members": [{"id": "abcdefghijklmnop", "path": "direct", "relay": null}, {"id": "qrstuvwxyz012345", "path": "relay", "relay": "relay.example.net"}, {"id": "0123456789abcdef", "path": null, "relay": null}],
            "relay": "relay.example.net",
            "downloads": [{"key": "ffmpeg", "hosts": ["github.com"]}],
            "update": {"host": "github.com", "channel": "beta"},
            "api": {"on": true, "lan": false},
        });
        let hidden = flow_text(&flow, false);
        assert!(!hidden.contains("relay.example.net"), "{hidden}");
        assert!(hidden.contains("member abcdefghij: direct") && hidden.contains("member qrstuvwxyz: through the relay <host>") && hidden.contains("member 0123456789: offline"), "{hidden}");
        assert!(hidden.contains("update check: github.com, beta channel") && hidden.contains("downloads: ffmpeg from github.com") && hidden.contains("local API: on, this PC only"), "{hidden}");
        assert!(flow_text(&flow, true).contains("address lookup: relay relay.example.net"));
        let quiet = flow_text(&json!({"members": [], "relay": null, "downloads": [], "update": {"host": "github.com", "channel": "stable"}, "api": {"on": false}}), true);
        assert!(quiet.contains("address lookup: relay none") && quiet.contains("downloads: none now") && quiet.contains("local API: off"), "{quiet}");
    }

    #[test]
    fn a_combined_report_names_each_member_by_short_id() {
        let out = combine("mine".into(), vec![("abc".into(), "theirs".into())]);
        assert!(out.starts_with("=== this PC ===\nmine\n") && out.contains("=== member abc ===\ntheirs"));
    }
}

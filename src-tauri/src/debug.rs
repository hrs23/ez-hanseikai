use std::collections::{BTreeMap, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use iroh::endpoint::Connection;
use iroh::Watcher;
use regex::Regex;
use serde_json::{json, Value};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::EnvFilter;

use crate::room::{self, SERIES};
use crate::ops::arg;
use crate::{check, data, ffmpeg, hw, net, now_us, relay, short, Core, NO_WINDOW};

const MAX_LOG: u64 = 10 << 20;
pub const LOG_FILES: [&str; 3] = ["log.txt", "log.old.txt", "log.older.txt"];
pub const CRASH: &str = "crash.txt";
const EVENTS_KEPT: usize = 500;
const TRANSFERS_KEPT: usize = 1500;
const CRASH_TAIL: u64 = 256 << 10;
const REPORT_LINES: usize = 400;
const ERRORS_KEPT: usize = 50;
const JOB_LINES: usize = 40;
const NAME_MIN_CHARS: usize = 2;
pub const UPDATE_HOST: &str = "github.com";

pub const TAKEN: usize = 0;
pub const ENCODED: usize = 1;
pub const DROPPED: usize = 2;
pub static FRAMES: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

static TRANSFERS: Mutex<VecDeque<Value>> = Mutex::new(VecDeque::new());
static ACTIVE: Mutex<Vec<(u64, Value)>> = Mutex::new(Vec::new());
static MESSAGES: Mutex<BTreeMap<(String, String), (u64, u64)>> = Mutex::new(BTreeMap::new());
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn push(rows: &mut VecDeque<Value>, row: Value) {
    rows.push_back(row);
    while rows.len() > TRANSFERS_KEPT {
        rows.pop_front();
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
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(format!("warn,{}=debug", env!("CARGO_CRATE_NAME"))));
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
}

pub fn what(peer: impl std::fmt::Display, req: &Value) -> What {
    let text = |k: &str| req[k].as_str().unwrap_or_default().to_string();
    let row = json!({"peer": short(peer), "owner": short(text("owner")), "capture": text("capture"), "series": text("series")});
    What { row, live: req["live"] == true, series: text("series") }
}

pub struct Transfer {
    id: u64,
    row: Value,
    done: bool,
}

pub fn start(dir: &str, what: &What, n: u32) -> Transfer {
    let mut row = what.row.clone();
    row["dir"] = json!(dir);
    row["n"] = json!(n);
    row["kind"] = json!(kind(what.live, &what.series, n));
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    ACTIVE.lock().unwrap().push((id, row.clone()));
    Transfer { id, row, done: false }
}

impl Transfer {
    pub fn finish(mut self) {
        self.done = true;
    }
}

impl Drop for Transfer {
    fn drop(&mut self) {
        ACTIVE.lock().unwrap().retain(|(id, _)| *id != self.id);
        record(std::mem::take(&mut self.row), self.done);
    }
}

fn same_piece(a: &Value, b: &Value) -> bool {
    ["dir", "owner", "capture", "series", "n"].iter().all(|k| a[k] == b[k])
}

pub fn result(rows: &VecDeque<Value>, row: &Value, done: bool) -> &'static str {
    if !done {
        return "partial";
    }
    let failed_before = rows.iter().rev().find(|r| same_piece(r, row)).is_some_and(|r| r["result"] != "ok");
    if failed_before { "retry" } else { "ok" }
}

fn record(mut row: Value, done: bool) {
    let mut rows = TRANSFERS.lock().unwrap();
    row["result"] = json!(result(&rows, &row, done));
    push(&mut rows, row);
}

pub fn mark_bad(rows: &mut VecDeque<Value>, owner: &str, capture: &str, series: &str, n: u32) {
    let wanted = json!({"dir": "in", "owner": short(owner), "capture": capture, "series": series, "n": n});
    if let Some(row) = rows.iter_mut().rev().find(|r| same_piece(r, &wanted)) {
        row["result"] = json!("hash fail");
        return;
    }
    let mut row = wanted;
    row["kind"] = json!(kind(false, series, n));
    row["result"] = json!("hash fail");
    push(rows, row);
}

pub fn hash_fail(owner: &str, capture: &str, series: &str, n: u32) {
    mark_bad(&mut TRANSFERS.lock().unwrap(), owner, capture, series, n);
}

pub fn queued_out(peer: &str) -> usize {
    ACTIVE.lock().unwrap().iter().filter(|(_, row)| row["dir"] == "out" && row["peer"] == peer).count()
}

pub fn slow_send(what: &What, took: std::time::Duration) {
    let peer = what.row["peer"].as_str().unwrap_or_default();
    if took >= hw::SLOW_SEND {
        hw::slow_send(peer, took, queued_out(peer));
    }
}

pub fn log_transfer(dir: &str, peer: impl std::fmt::Display, of: &str, bytes: usize) {
    if bytes > 0 {
        record(json!({"dir": dir, "peer": short(peer), "owner": short(of), "kind": "log"}), true);
    }
}

pub fn message(dir: &str, kind: Option<&str>, bytes: usize) {
    let mut all = MESSAGES.lock().unwrap();
    let entry = all.entry((dir.to_string(), kind.unwrap_or("?").to_string())).or_default();
    entry.0 += 1;
    entry.1 += bytes as u64;
}

pub fn frame(which: usize) {
    FRAMES[which].fetch_add(1, Ordering::Relaxed);
}

pub fn jobs(logs: &[(String, &[Value])]) -> Vec<Value> {
    let mut found: BTreeMap<String, Value> = BTreeMap::new();
    for (owner, records) in logs {
        for r in records.iter().filter(|r| r["k"] == "job") {
            let Some(id) = r["job"].as_str().map(String::from) else { continue };
            let t = r["t_us"].as_i64().unwrap_or(0);
            let job = found.entry(id).or_insert_with(|| json!({"t_us": i64::MIN}));
            for k in ["plugin", "set"] {
                if !r[k].is_null() {
                    job[k] = r[k].clone();
                }
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

fn path_json(conn: &Connection) -> Vec<Value> {
    conn.paths()
        .iter()
        .map(|path| {
            let stats = path.stats();
            json!({
                "kind": relay::kind(path.remote_addr()),
                "selected": path.is_selected(),
                "rtt_ms": stats.rtt.as_secs_f64() * 1000.0,
                "tx": stats.udp_tx.bytes,
                "rx": stats.udp_rx.bytes,
                "mtu": stats.current_mtu,
            })
        })
        .collect()
}

fn endpoint_json(core: &Core) -> Value {
    let report = core.ep.net_report().get();
    let home = relay::home(core);
    json!({
        "id": core.me(),
        "relay_connected": home.is_some(),
        "relay": home.map(|u| u.to_string()).unwrap_or_default(),
        "sockets": format!("{:?}", core.ep.bound_sockets()),
        "addr": format!("{:?}", core.ep.addr()),
        "relay_only": std::env::var_os("EZH_RELAY_ONLY").is_some(),
        "net": report.map(|r| json!({
            "udp_v4": r.udp_v4,
            "udp_v6": r.udp_v6,
            "mapping_varies_v4": r.mapping_varies_by_dest_ipv4,
            "mapping_varies_v6": r.mapping_varies_by_dest_ipv6,
            "captive_portal": r.captive_portal,
            "preferred_relay": r.preferred_relay.map(|u| u.to_string()).unwrap_or_default(),
            "global_v4": r.global_v4.map(|a| a.to_string()).unwrap_or_default(),
            "global_v6": r.global_v6.map(|a| a.to_string()).unwrap_or_default(),
        })),
    })
}

pub fn flow_json(core: &Core) -> Value {
    let members: Vec<Value> = core
        .peers
        .lock()
        .unwrap()
        .iter()
        .map(|p| {
            let kind = p.alive().map(relay::path_kind);
            json!({"id": p.id.to_string(), "path": kind.as_ref().map(|k| k.0).filter(|k| *k != "none"), "relay": kind.and_then(|k| k.2)})
        })
        .collect();
    json!({
        "members": members,
        "relay": relay::home(core).and_then(|u| u.host_str().map(String::from)),
        "download": ffmpeg::download_host(),
        "update": {"host": UPDATE_HOST},
        "time": {"host": crate::clock::HOST, "ok": crate::clock::status()["ok"] == true},
        "api": {"on": *core.api_on.lock().unwrap()},
    })
}

pub fn flow_text(flow: &Value) -> String {
    let host = |h: &Value| if h.is_string() { "<host>" } else { "none" };
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
    out.push_str(&format!("  update check: {}\n", flow["update"]["host"].as_str().unwrap_or("?")));
    out.push_str(&format!("  time check: {}, {}\n", flow["time"]["host"].as_str().unwrap_or("?"), if flow["time"]["ok"] == true { "ok" } else { "unreachable" }));
    let download = flow["download"].as_str().map_or("none now".to_string(), |host| format!("ffmpeg from {host}"));
    out.push_str(&format!("  downloads: {download}\n"));
    let api = if flow["api"]["on"] == true { "on, this PC only" } else { "off" };
    out.push_str(&format!("  local API: {api}\n"));
    out
}

pub fn tail(path: &Path, max: u64) -> String {
    let Ok(mut file) = File::open(path) else { return String::new() };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(max);
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(start)).is_ok() {
        let _ = file.take(len - start).read_to_end(&mut bytes);
    }
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 { text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default() } else { text }
}

pub fn debug_flow(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    Ok(flow_json(core))
}

pub fn debug_report(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let text = diagnostics(core, false)?;
    if !arg::<bool>(&args, "save")? {
        return Ok(json!(text));
    }
    let path = core.dir.join("report.txt");
    fs::write(&path, text).map_err(|e| e.to_string())?;
    Ok(json!(path.display().to_string()))
}

pub fn diagnostics(core: &Arc<Core>, everyone: bool) -> Result<String, String> {
    crate::ops::warm(core);
    let own = report_text(core, Some(&check::run(core).join("\n")));
    Ok(if everyone { combine(own, tauri::async_runtime::block_on(team(core))) } else { own })
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

fn describe(conn: &Connection, out: &mut String) {
    out.push_str(&format!("connection to {}\n", conn.remote_id().fmt_short()));
    for p in path_json(conn) {
        out.push_str(&format!(
            "  path {}{} rtt {:.1} ms sent {} B received {} B mtu {}\n",
            p["kind"].as_str().unwrap_or_default(),
            if p["selected"] == true { " (selected)" } else { "" },
            p["rtt_ms"].as_f64().unwrap_or(0.0),
            p["tx"],
            p["rx"],
            p["mtu"]
        ));
    }
    let stats = conn.stats();
    out.push_str(&format!("  total sent {} B received {} B lost packets {}\n", stats.udp_tx.bytes, stats.udp_rx.bytes, stats.lost_packets));
}

pub fn text(core: &Core) -> String {
    let mut out = String::new();
    out.push_str(&format!("encoder {} ({})\n", ffmpeg::encoder(), ffmpeg::path().display()));
    out.push_str(&format!("encoder probe {:?}\n", ffmpeg::encoder_results()));
    out.push_str(&format!("endpoint {}\n", endpoint_json(core)));
    let peers: Vec<(String, Option<Connection>, Option<i64>)> = core.peers.lock().unwrap().iter().map(|p| (p.id.to_string(), p.conn.clone(), p.offset_us)).collect();
    for (id, conn, offset) in peers {
        out.push_str(&format!("peer {id} clock offset {offset:?} us\n"));
        match conn {
            Some(conn) => describe(&conn, &mut out),
            None => out.push_str("connection: none\n"),
        }
    }
    if let Some(room) = core.room_id() {
        let logs = room::logs(core, &room);
        for set in room::sets_json(core, &room, &logs, &[]) {
            out.push_str(&format!("set {}\n", set["id"].as_str().unwrap_or_default()));
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

pub fn transfer_summary(rows: &VecDeque<Value>) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    for r in rows {
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

fn last<T>(mut items: Vec<T>, count: usize) -> Vec<T> {
    items.drain(..items.len().saturating_sub(count));
    items
}

pub fn errors(log: &str) -> Vec<&str> {
    last(log.lines().filter(|l| l.contains(" WARN ") || l.contains(" ERROR ")).collect(), ERRORS_KEPT)
}

pub fn job_line(job: &Value) -> String {
    let text = |k: &str| job[k].as_str().unwrap_or("?");
    let progress = job["progress"].as_f64().map(|p| format!(" {}%", (p * 100.0).floor())).unwrap_or_default();
    format!("  {} {} {}{} by {}", text("plugin"), text("set"), text("state"), progress, job["by"].as_str().map(String::from).unwrap_or_else(|| "?".into()))
}

fn jobs_text(logs: &[room::Log]) -> String {
    let pairs: Vec<(String, &[Value])> = logs.iter().map(|l| (short(&l.owner), l.records.as_slice())).collect();
    let lines: Vec<String> = jobs(&pairs).iter().take(JOB_LINES).map(job_line).collect();
    if lines.is_empty() { "  none".into() } else { lines.join("\n") }
}

pub fn report_text(core: &Core, check: Option<&str>) -> String {
    let log = fs::read_to_string(core.dir.join("log.txt")).unwrap_or_default();
    let recent = last(log.lines().collect(), REPORT_LINES);
    let mut out = String::new();
    out.push_str(&format!("{} {} on {} {}\n", env!("CARGO_PKG_NAME"), crate::VERSION, std::env::consts::OS, command_line("cmd", &["/c", "ver"])));
    out.push_str(&format!("previous version {}\n", if core.previous_version.is_empty() { "none" } else { &core.previous_version }));
    out.push_str(&format!("ffmpeg {}\n\n", command_line(&ffmpeg::path().to_string_lossy(), &["-version"]).lines().next().unwrap_or("")));
    let infos: Vec<String> = core.peers.lock().unwrap().iter().map(|p| format!("{} {:?}", short(p.id), p.info)).collect();
    out.push_str(&format!("peers {}\n\nenvironment check\n{}\n\n", infos.join(", "), check.unwrap_or("not run")));
    out.push_str(&text(core));
    out.push_str(&format!("\n{}\n\n{}", hw::report(core), flow_text(&flow_json(core))));
    out.push_str("\nrecent errors\n");
    out.push_str(&errors(&log).join("\n"));
    out.push_str("\n\njobs\n");
    out.push_str(&jobs_text(&core.room_id().map(|room| room::logs(core, &room)).unwrap_or_default()));
    out.push_str("\n\nrecent crashes\n");
    out.push_str(&tail(&core.dir.join(CRASH), CRASH_TAIL));
    out.push_str("\n\nrecent log\n");
    out.push_str(&recent.join("\n"));
    redact(&out, &home(), &known_names(core))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(dir: &str, n: u32, result: &str) -> Value {
        json!({"dir": dir, "owner": "a", "capture": "1", "series": "v", "n": n, "result": result})
    }

    #[test]
    fn the_transfer_list_keeps_the_newest() {
        let mut rows = VecDeque::new();
        for n in 0..TRANSFERS_KEPT + 2 {
            push(&mut rows, json!({"n": n}));
        }
        assert_eq!((rows.len(), rows[0]["n"].as_u64()), (TRANSFERS_KEPT, Some(2)));
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
        let mut ring = VecDeque::new();
        assert_eq!(result(&ring, &row("in", 1, ""), true), "ok");
        assert_eq!(result(&ring, &row("in", 1, ""), false), "partial");
        push(&mut ring, row("in", 1, "partial"));
        push(&mut ring, row("in", 2, "ok"));
        assert_eq!(result(&ring, &row("in", 1, ""), true), "retry");
        assert_eq!(result(&ring, &row("out", 1, ""), true), "ok");
        assert_eq!(result(&ring, &row("in", 2, ""), true), "ok");
    }

    #[test]
    fn a_hash_failure_marks_the_last_receipt_or_adds_a_row() {
        let mut ring = VecDeque::new();
        push(&mut ring, json!({"dir": "in", "owner": short("abcdefghijklmnop"), "capture": "1", "series": "v", "n": 4, "result": "ok"}));
        mark_bad(&mut ring, "abcdefghijklmnop", "1", "v", 4);
        assert_eq!(ring[0]["result"], "hash fail");
        mark_bad(&mut ring, "abcdefghijklmnop", "1", "m", 5);
        assert_eq!(ring.len(), 2);
        assert_eq!(ring[1]["kind"], "history voice");
        assert_eq!(result(&ring, &ring[1], true), "retry");
    }

    #[test]
    fn jobs_are_grouped_with_their_latest_state() {
        let a = [json!({"k": "job", "state": "claim", "job": "j1", "plugin": "p", "file": "f", "t_us": 10}), json!({"k": "job", "state": "claim", "job": "j1", "progress": 0.5, "t_us": 20})];
        let b = [json!({"k": "job", "state": "done", "job": "j1", "t_us": 30}), json!({"k": "job", "state": "claim", "job": "j2", "t_us": 5}), json!({"k": "event", "job": "x"})];
        let jobs = jobs(&[("a".into(), &a[..]), ("b".into(), &b[..])]);
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0]["state"], "done");
        assert_eq!(jobs[0]["by"], "b");
        assert_eq!(jobs[0]["progress"], 0.5);
        assert_eq!(jobs[0]["plugin"], "p");
        assert_eq!(jobs[1]["state"], "claim");
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
        let job = json!({"plugin": "apex-matches", "set": "s1", "state": "running", "progress": 0.426, "by": "ab12"});
        assert_eq!(job_line(&job), "  apex-matches s1 running 42% by ab12");
        assert_eq!(job_line(&json!({})), "  ? ? ? by ?");
    }

    #[test]
    fn addresses_are_hidden_and_the_rest_kept() {
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
    fn the_tail_starts_at_a_whole_line() {
        let dir = crate::log::tests::temp("tail");
        let path = dir.join("crash.txt");
        fs::write(&path, "one\ntwo\nthree\n").unwrap();
        assert_eq!(tail(&path, 1000), "one\ntwo\nthree\n");
        assert_eq!(tail(&path, 8), "three\n");
        assert_eq!(tail(&dir.join("none.txt"), 8), "");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_flow_text_names_each_route_and_hides_hosts() {
        let flow = json!({
            "members": [{"id": "abcdefghijklmnop", "path": "direct", "relay": null}, {"id": "qrstuvwxyz012345", "path": "relay", "relay": "relay.example.net"}, {"id": "0123456789abcdef", "path": null, "relay": null}],
            "relay": "relay.example.net",
            "download": "github.com",
            "update": {"host": "github.com"},
            "time": {"host": "time.cloudflare.com", "ok": true},
            "api": {"on": true},
        });
        let hidden = flow_text(&flow);
        assert!(!hidden.contains("relay.example.net"), "{hidden}");
        assert!(hidden.contains("member abcdefghij: direct") && hidden.contains("member qrstuvwxyz: through the relay <host>") && hidden.contains("member 0123456789: offline"), "{hidden}");
        assert!(hidden.contains("update check: github.com\n") && hidden.contains("downloads: ffmpeg from github.com") && hidden.contains("local API: on, this PC only"), "{hidden}");
        assert!(hidden.contains("address lookup: relay <host>") && hidden.contains("time check: time.cloudflare.com, ok"), "{hidden}");
        let quiet = flow_text(&json!({"members": [], "relay": null, "download": null, "update": {"host": "github.com"}, "time": {"host": "time.cloudflare.com", "ok": false}, "api": {"on": false}}));
        assert!(quiet.contains("address lookup: relay none") && quiet.contains("downloads: none now") && quiet.contains("local API: off") && quiet.contains("time check: time.cloudflare.com, unreachable"), "{quiet}");
    }

    #[test]
    fn a_combined_report_names_each_member_by_short_id() {
        let out = combine("mine".into(), vec![("abc".into(), "theirs".into())]);
        assert!(out.starts_with("=== this PC ===\nmine\n") && out.contains("=== member abc ===\ntheirs"));
    }
}

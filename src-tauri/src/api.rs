use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream, ToSocketAddrs};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{json, Value};

use crate::room::{self, Log, Set};
use crate::{party, tenth, Core};

pub const DEFAULT_PORT: u16 = 47625;
pub const TOKEN_BYTES: usize = 24;
const INFO_FILE: &str = "api.json";
const HEAD_MAX: usize = 16 << 10;
const BODY_MAX: usize = 64 << 10;
const CHUNK: usize = 4 << 10;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(50);
const CONNECTIONS_MAX: usize = 8;
const TAGS_MAX: usize = 16;
const AT_MAX_S: f64 = 7.0 * 24.0 * 3600.0;
const LLMS: &str = include_str!("../../docs/llms.txt");
const METHODS: [&str; 3] = ["GET ", "POST ", "PUT "];
const DEFAULT_HIDDEN: [&str; 3] = ["speech", "drawing", "board"];
static PORT: LazyLock<u16> = LazyLock::new(|| std::env::var("EZH_API_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(DEFAULT_PORT));

pub fn port() -> u16 {
    *PORT
}

pub fn address() -> String {
    format!("127.0.0.1:{}", port())
}

pub fn requested() -> bool {
    std::env::args().any(|a| a == "--api") || std::env::var("EZH_API").is_ok_and(|v| v == "1")
}

fn computer_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
}

fn own_addresses() -> Vec<String> {
    let name = computer_name();
    if name.is_empty() {
        return Vec::new();
    }
    let found = (name.as_str(), 0).to_socket_addrs().into_iter().flatten();
    let mut out: Vec<String> = found.filter_map(|a| if let IpAddr::V4(ip) = a.ip() { (!ip.is_loopback()).then(|| ip.to_string()) } else { None }).collect();
    out.dedup();
    out
}

pub fn local_hosts() -> Vec<String> {
    let mut out = own_addresses();
    let name = computer_name();
    if !name.is_empty() {
        out.push(name);
    }
    out
}

pub fn lan_addresses() -> Vec<String> {
    local_hosts().into_iter().map(|h| format!("{h}:{}", port())).collect()
}

fn restrict(path: &Path) -> std::io::Result<()> {
    let user = std::env::var("USERNAME").map_err(std::io::Error::other)?;
    let status = Command::new("icacls").arg(path).args(["/inheritance:r", "/grant:r"]).arg(format!("{user}:F")).args(["/remove:g", "*S-1-5-18", "*S-1-5-32-544"]).creation_flags(crate::proc::NO_WINDOW).stdout(Stdio::null()).stderr(Stdio::null()).status()?;
    if status.success() { Ok(()) } else { Err(std::io::Error::other("icacls failed")) }
}

fn write_info(dir: &Path, token: &str) -> std::io::Result<()> {
    let path = dir.join(INFO_FILE);
    let made = std::fs::write(&path, b"").and_then(|_| restrict(&path)).and_then(|_| std::fs::write(&path, json!({"address": address(), "token": token}).to_string()));
    if made.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    made
}

pub fn forget(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(INFO_FILE));
}

pub fn refresh(core: &Core) {
    if SERVER.lock().unwrap().is_some() {
        if let Err(e) = write_info(&core.dir, &core.api_token.lock().unwrap()) {
            tracing::warn!("the local API address file was not written: {e}");
        }
    }
}

pub fn token_valid(token: &str) -> bool {
    token.len() == TOKEN_BYTES * 2 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    fn query(&self, name: &str) -> Option<&str> {
        self.query.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

pub struct Response {
    pub status: u16,
    pub kind: &'static str,
    pub body: Vec<u8>,
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    }
}

fn reply(status: u16, value: Value) -> Response {
    Response { status, kind: "application/json; charset=utf-8", body: value.to_string().into_bytes() }
}

fn error(status: u16, text: &str) -> Response {
    reply(status, json!({"error": text}))
}

pub fn host_ok(host: Option<&str>, port: u16, own: &[String]) -> bool {
    let Some((name, given)) = host.and_then(|h| h.rsplit_once(':')) else { return false };
    given.parse::<u16>().ok() == Some(port) && (["127.0.0.1", "localhost"].contains(&name) || own.iter().any(|h| h.eq_ignore_ascii_case(name)))
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

pub fn token_ok(header: Option<&str>, token: &str) -> bool {
    let Some((scheme, given)) = header.and_then(|h| h.split_once(' ')) else { return false };
    !token.is_empty() && scheme.eq_ignore_ascii_case("bearer") && same(given.trim().as_bytes(), token.as_bytes())
}

pub fn decode(text: &str, plus: bool) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let digits = text.get(i + 1..i + 3).filter(|d| d.bytes().all(|b| b.is_ascii_hexdigit()))?;
                out.push(u8::from_str_radix(digits, 16).ok()?);
                i += 3;
            }
            b'+' if plus => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

pub fn parse_head(head: &str) -> Result<Request, Response> {
    let bad = || error(400, "Bad request.");
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or_default().split(' ');
    let (Some(method), Some(target), Some(version)) = (first.next(), first.next(), first.next()) else { return Err(bad()) };
    if !version.starts_with("HTTP/1.") || !target.starts_with('/') {
        return Err(bad());
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut headers = HashMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or_else(bad)?;
        let name = name.trim().to_ascii_lowercase();
        let single = ["host", "authorization", "content-length"].contains(&name.as_str());
        if headers.insert(name, value.trim().to_string()).is_some() && single {
            return Err(bad());
        }
    }
    let query = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            Some((decode(k, true)?, decode(v, true)?))
        })
        .collect::<Option<Vec<_>>>()
        .ok_or_else(bad)?;
    Ok(Request { method: method.to_string(), path: path.to_string(), query, headers, body: Vec::new() })
}

fn read_more(stream: &mut impl Read, buf: &mut Vec<u8>) -> Result<(), Response> {
    let mut chunk = [0u8; CHUNK];
    match stream.read(&mut chunk) {
        Ok(0) | Err(_) => Err(error(400, "Bad request.")),
        Ok(n) => Ok(buf.extend_from_slice(&chunk[..n])),
    }
}

pub fn read_request(stream: &mut impl Read) -> Result<Request, Response> {
    let mut buf: Vec<u8> = Vec::new();
    let end = loop {
        if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            if at > HEAD_MAX {
                return Err(error(431, "The request head is too large."));
            }
            break at;
        }
        if buf.len() > HEAD_MAX {
            return Err(error(431, "The request head is too large."));
        }
        read_more(stream, &mut buf)?;
    };
    let head = std::str::from_utf8(&buf[..end]).map_err(|_| error(400, "Bad request."))?;
    let mut request = parse_head(head)?;
    if request.header("transfer-encoding").is_some() {
        return Err(error(400, "Send the body with a Content-Length."));
    }
    let length = match request.header("content-length") {
        Some(text) => text.parse::<usize>().map_err(|_| error(400, "Bad Content-Length."))?,
        None => 0,
    };
    if length > BODY_MAX {
        return Err(error(413, "The request body is too large."));
    }
    let mut body = buf[end + 4..].to_vec();
    while body.len() < length {
        read_more(stream, &mut body)?;
    }
    body.truncate(length);
    request.body = body;
    Ok(request)
}

#[derive(Clone)]
pub struct Head {
    pub id: String,
    pub name: String,
    pub active: bool,
    pub members: Vec<(String, bool)>,
    pub names: HashMap<String, String>,
    pub present: HashSet<String>,
}

pub struct Row {
    pub id: String,
    pub online: bool,
    pub sharing: bool,
    pub recording: bool,
}

fn state_of(row: &Row) -> &'static str {
    if !row.online {
        "offline"
    } else if row.recording {
        "recording"
    } else if row.sharing {
        "standby"
    } else {
        "idle"
    }
}

pub trait Backend {
    fn app(&self) -> Value;
    fn status(&self) -> Vec<Row>;
    fn me(&self) -> String;
    fn now_us(&self) -> i64;
    fn heads(&self) -> Vec<Head>;
    fn logs(&self, room: &str) -> Vec<Log>;
    fn plugins(&self, room: &str) -> Vec<Value>;
    fn analysis(&self, room: &str) -> HashMap<String, Vec<Value>>;
    fn voice(&self, room: &str, set: &str, member: &str) -> Option<Vec<u8>>;
    fn add(&self, room: &str, record: Value) -> Result<(), String>;
    fn call(&self, name: &str, args: Value) -> Option<Result<Value, String>>;
    fn diagnostics(&self, everyone: bool) -> Result<String, String>;
}

struct Loaded {
    head: Head,
    logs: Vec<Log>,
}

impl Loaded {
    fn new(be: &dyn Backend, head: Head) -> Self {
        let logs = be.logs(&head.id);
        Self { head, logs }
    }

    fn sets(&self) -> Vec<Set> {
        room::with_presence(room::derive_sets(&self.head.id, &self.logs), |owner| self.head.present.contains(owner))
    }

    fn name(&self, id: &str) -> String {
        name_of(&self.head.names, id)
    }
}

fn name_of(names: &HashMap<String, String>, id: &str) -> String {
    names.get(id).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| crate::short(id))
}

fn seconds(set: &Set, us: Option<i64>) -> Value {
    us.map_or(Value::Null, |us| json!(tenth((us - set.start()) as f64 / 1e6)))
}

fn rooms_json(heads: &[Head]) -> Value {
    heads
        .iter()
        .filter(|h| !h.id.starts_with("pending-"))
        .map(|h| json!({"id": h.id, "name": h.name, "active": h.active, "members": h.members.iter().map(|(id, online)| json!({"id": id, "name": name_of(&h.names, id), "online": online})).collect::<Vec<_>>()}))
        .collect()
}

fn status_reply(be: &dyn Backend) -> Response {
    let app = be.app();
    let me = be.me();
    let rows = be.status();
    let names = be.heads().into_iter().find(|h| h.active).map(|h| h.names).unwrap_or_default();
    let members: Vec<Value> = rows.iter().map(|r| json!({"id": r.id, "name": name_of(&names, &r.id), "online": r.online, "state": state_of(r)})).collect();
    let own = rows.iter().find(|r| r.id == me).map_or("idle", state_of);
    reply(200, json!({"room": app["room"], "recording": app["recording"], "state": own, "members": members}))
}

struct Extras {
    watch: HashMap<String, Vec<Value>>,
    analysis: HashMap<String, Vec<Value>>,
}

impl Extras {
    fn new(be: &dyn Backend, l: &Loaded) -> Self {
        Self { watch: party::records(&l.logs), analysis: be.analysis(&l.head.id) }
    }
}

fn recording_json(l: &Loaded, set: &Set, extras: &Extras, now_us: i64) -> Value {
    let parent = room::debrief_of(&set.id, &l.logs);
    let start = set.start();
    let end = set.end_shown();
    let members: Vec<Value> = set
        .files
        .iter()
        .map(|f| json!({"id": f.owner, "name": l.name(&f.owner), "voice_only": f.series(room::SERIES[0]).is_empty(), "imported": f.view.is_some(), "start_ms": (f.start_us - f.offset_us) / 1000, "end_ms": f.end_us.map(|e| (e - f.offset_us) / 1000)}))
        .collect();
    let imported = set.files.iter().find_map(|f| f.view.as_ref());
    let mut out = json!({
        "id": set.id, "kind": if parent.is_some() { "debrief" } else { "play" }, "recording": set.recording(),
        "title": imported.and_then(|v| v.title.clone()), "imported_by": imported.map(|v| json!({"id": v.by, "name": l.name(&v.by)})),
        "start_ms": start / 1000, "end_ms": end.map(|e| e / 1000), "length_s": tenth((end.unwrap_or(now_us) - start).max(0) as f64 / 1e6),
        "members": members, "analysis": extras.analysis.get(&set.id).cloned().unwrap_or_default(),
    });
    if let Some(parent) = parent {
        out["of"] = json!(parent);
        out["watch"] = party::watch_json(extras.watch.get(&set.id).map(Vec::as_slice).unwrap_or(&[]));
    }
    out
}

fn find_set(l: &Loaded, id: &str) -> Option<Set> {
    l.sets().into_iter().find(|s| s.id == id)
}

fn event_json(l: &Loaded, set: &Set, e: &Value) -> Value {
    let member = e["file"].as_str();
    json!({
        "key": e["key"], "id": e["id"], "author": e["author"], "author_name": l.name(e["author"].as_str().unwrap_or_default()),
        "plugin": e["plugin"]["name"], "kind": e["kind"], "at_s": seconds(set, e["local_us"].as_i64()), "to_s": seconds(set, e["end_us"].as_i64()),
        "member": member, "member_name": member.map(|m| l.name(m)), "text": e["text"], "tags": e["tags"], "values": e["values"],
    })
}

fn covered(l: &Loaded, set: &Set) -> Vec<Value> {
    room::events(&l.logs, None).into_iter().filter(|e| room::covers(set, e)).collect()
}

fn events_reply(l: &Loaded, set: &Set, req: &Request) -> Response {
    let kind = req.query("kind");
    let tag = req.query("tag").map(|t| t.trim_start_matches('#').to_lowercase());
    let list: Vec<Value> = covered(l, set)
        .iter()
        .filter(|e| match kind {
            Some(kind) => e["kind"] == kind,
            None => !DEFAULT_HIDDEN.iter().any(|k| e["kind"] == *k),
        })
        .filter(|e| tag.as_ref().is_none_or(|t| e["tags"].as_array().into_iter().flatten().any(|x| x.as_str().is_some_and(|x| x.to_lowercase() == *t))))
        .map(|e| event_json(l, set, e))
        .collect();
    reply(200, json!({"events": list}))
}

fn chapters_reply(l: &Loaded, set: &Set, now_us: i64) -> Response {
    let mut ends: Vec<f64> = covered(l, set)
        .iter()
        .filter_map(|e| room::match_end(e))
        .map(|us| (us - set.start()) as f64 / 1e6)
        .collect();
    ends.sort_by(|a, b| a.total_cmp(b));
    let mut chapters = Vec::new();
    let mut from = 0.0;
    for (i, end) in ends.iter().enumerate() {
        chapters.push(json!({"n": i + 1, "name": format!("Match {}", i + 1), "from_s": tenth(from), "to_s": tenth(*end)}));
        from = *end;
    }
    let total = (set.end_shown().unwrap_or(now_us) - set.start()) as f64 / 1e6;
    if set.recording() && !ends.is_empty() && total > from {
        chapters.push(json!({"n": ends.len() + 1, "name": format!("Match {}", ends.len() + 1), "from_s": tenth(from), "to_s": null}));
    }
    reply(200, json!({"chapters": chapters}))
}

fn transcript_reply(l: &Loaded, set: &Set, req: &Request) -> Response {
    let member = req.query("member");
    let mut lines: Vec<(f64, Value)> = covered(l, set)
        .iter()
        .filter(|e| e["kind"] == "speech" && e["file"].is_string() && member.is_none_or(|m| e["file"] == m))
        .filter_map(|e| {
            let (from, to) = (e["local_us"].as_i64()?, e["end_us"].as_i64().unwrap_or(e["local_us"].as_i64()?));
            let who = e["file"].as_str()?;
            Some((from as f64, json!({"member": who, "member_name": l.name(who), "from_s": seconds(set, Some(from)), "to_s": seconds(set, Some(to)), "text": e["text"]})))
        })
        .collect();
    lines.sort_by(|a, b| a.0.total_cmp(&b.0));
    reply(200, json!({"lines": lines.into_iter().map(|(_, v)| v).collect::<Vec<_>>()}))
}

fn board_json(l: &Loaded) -> Value {
    let cards = room::board_cards(&room::events(&l.logs, None), &[]);
    let team = cards.iter().find(|c| c["card"] == "team").unwrap_or(&Value::Null);
    let author = team["by"].as_str();
    json!({"text": team["text"], "author": author, "author_name": author.map(|a| l.name(a)), "updated_ms": team["at_us"].as_i64().map(|us| us / 1000)})
}

fn pick(value: &Value, keys: &[&str]) -> Value {
    keys.iter().map(|k| (k.to_string(), value[*k].clone())).collect::<serde_json::Map<String, Value>>().into()
}

fn plugins_json(l: &Loaded, list: Vec<Value>) -> Value {
    let name = |id: &Value| id.as_str().map_or(Value::Null, |id| json!(l.name(id)));
    let plugins: Vec<Value> = list
        .iter()
        .map(|p| {
            let mut out = pick(p, &["id", "name", "version", "on", "settings", "ready", "runner", "left", "failed", "waiting_for"]);
            out["runner_name"] = name(&p["runner"]);
            out["waiting_for_name"] = name(&p["waiting_for"]);
            out
        })
        .collect();
    json!({"plugins": plugins})
}

fn text_of(body: &serde_json::Map<String, Value>, key: &str) -> Result<String, Response> {
    match body.get(key) {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(text)) => Ok(text.clone()),
        Some(_) => Err(error(400, &format!("{key} must be a string."))),
    }
}

fn tags_of(body: &serde_json::Map<String, Value>) -> Result<Vec<String>, Response> {
    let bad = || error(400, "tags must be a list of words.");
    let list = match body.get("tags") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(list)) => list,
        Some(_) => return Err(bad()),
    };
    let mut out = Vec::new();
    for tag in list {
        let tag: String = tag.as_str().ok_or_else(bad)?.trim_start_matches('#').chars().filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-').collect();
        if !tag.is_empty() && !out.iter().any(|t: &String| t.to_lowercase() == tag.to_lowercase()) {
            out.push(tag);
        }
    }
    if out.len() > TAGS_MAX {
        return Err(error(400, "Too many tags."));
    }
    Ok(out)
}

fn body_of(req: &Request) -> Result<serde_json::Map<String, Value>, Response> {
    match serde_json::from_slice::<Value>(&req.body) {
        Ok(Value::Object(map)) => Ok(map),
        _ => Err(error(400, "Send a JSON object.")),
    }
}

fn add_event(be: &dyn Backend, l: &Loaded, set: &Set, req: &Request) -> Response {
    let built = (|| {
        let body = body_of(req)?;
        let kind = match body.get("kind").and_then(Value::as_str) {
            None | Some("note") => "note",
            Some("mark") => "mark",
            Some(_) => return Err(error(400, "kind must be note or mark.")),
        };
        let at = match body.get("at_s") {
            None | Some(Value::Null) => None,
            Some(v) => Some(v.as_f64().filter(|s| s.is_finite() && (0.0..=AT_MAX_S).contains(s)).ok_or_else(|| error(400, "at_s must be seconds from the start of the recording."))?),
        };
        if kind == "mark" && at.is_none() {
            return Err(error(400, "A mark needs at_s."));
        }
        let text = crate::clean_note(&text_of(&body, "text")?, usize::MAX).trim().to_string();
        let mut words = vec![text];
        for tag in tags_of(&body)? {
            if !room::tags(&words[0]).iter().any(|t| t.to_lowercase() == tag.to_lowercase()) {
                words.push(format!("#{tag}"));
            }
        }
        let text = words.into_iter().filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
        if kind == "note" && text.is_empty() {
            return Err(error(400, "Write something first."));
        }
        if text.chars().count() > crate::NOTE_MAX {
            return Err(error(400, &format!("A note is at most {} characters.", crate::NOTE_MAX)));
        }
        let target = match at {
            Some(s) => json!({"t_us": set.start() + (s * 1e6).round() as i64, "set": set.id}),
            None => json!({"set": set.id}),
        };
        Ok((kind, text, target))
    })();
    let (kind, text, target) = match built {
        Ok(parts) => parts,
        Err(response) => return response,
    };
    let id = be.now_us();
    let mut record = room::user_event(id, kind, &text, target, Value::Null);
    record["source"] = json!("ai");
    if let Err(e) = be.add(&l.head.id, record) {
        return error(409, &e);
    }
    let key = format!("{}/{id}", be.me());
    let again = Loaded::new(be, l.head.clone());
    match covered(&again, set).iter().find(|e| e["key"] == key.as_str()) {
        Some(e) => reply(201, event_json(&again, set, e)),
        None => error(500, "The event was not found after writing it."),
    }
}

fn put_board(be: &dyn Backend, l: &Loaded, req: &Request) -> Response {
    let text = match body_of(req).and_then(|b| text_of(&b, "text")) {
        Ok(text) => crate::clean_note(&text, usize::MAX),
        Err(response) => return response,
    };
    if text.trim().is_empty() {
        return error(400, "Write something first.");
    }
    if text.chars().count() > crate::BOARD_MAX {
        return error(400, &format!("The game plan is at most {} characters.", crate::BOARD_MAX));
    }
    if let Err(e) = be.add(&l.head.id, room::user_event(be.now_us(), "board", &text, json!({}), json!({"card": "team"}))) {
        return error(409, &e);
    }
    reply(200, board_json(&Loaded::new(be, l.head.clone())))
}

fn operation(be: &dyn Backend, name: &str, req: &Request) -> Response {
    let args = if req.body.is_empty() { Ok(serde_json::Map::new()) } else { body_of(req) };
    match args.map(|map| be.call(name, Value::Object(map))) {
        Err(response) => response,
        Ok(None) => error(404, "No such operation."),
        Ok(Some(Ok(value))) => reply(200, value),
        Ok(Some(Err(e))) => error(400, &e),
    }
}

fn segments(path: &str) -> Option<Vec<String>> {
    path.split('/').filter(|s| !s.is_empty()).map(|s| decode(s, false)).collect()
}

fn route(req: &Request, be: &dyn Backend, parts: &[&str]) -> Response {
    let allowed: &[&str] = match parts {
        ["v1", "ops", _] => &["POST"],
        ["v1"] | ["v1", "status"] | ["v1", "diagnostics"] | ["v1", "rooms"] | ["v1", "rooms", _, "recordings"] | ["v1", "rooms", _, "recordings", _] => &["GET"],
        ["v1", "rooms", _, "recordings", _, "chapters" | "transcript"] | ["v1", "rooms", _, "recordings", _, "voices", _] | ["v1", "rooms", _, "plugins"] => &["GET"],
        ["v1", "rooms", _, "recordings", _, "events"] => &["GET", "POST"],
        ["v1", "rooms", _, "board"] => &["GET", "PUT"],
        _ => return error(404, "No such endpoint."),
    };
    if !allowed.contains(&req.method.as_str()) {
        return error(405, "Method not allowed.");
    }
    let ["v1", "rooms", room, rest @ ..] = parts else {
        return match parts {
            ["v1", "rooms"] => reply(200, json!({"rooms": rooms_json(&be.heads())})),
            ["v1", "status"] => status_reply(be),
            ["v1", "diagnostics"] => match be.diagnostics(matches!(req.query("everyone"), Some("true" | "1"))) {
                Ok(text) => reply(200, json!({"text": text})),
                Err(e) => error(500, &e),
            },
            ["v1", "ops", name] => operation(be, name, req),
            _ => reply(200, be.app()),
        };
    };
    let Some(head) = be.heads().into_iter().find(|h| h.id == *room && !h.id.starts_with("pending-")) else { return error(404, "No such room.") };
    let l = Loaded::new(be, head);
    match rest {
        ["board"] if req.method == "PUT" => put_board(be, &l, req),
        ["board"] => reply(200, board_json(&l)),
        ["plugins"] => reply(200, plugins_json(&l, be.plugins(room))),
        ["recordings"] => {
            let extras = Extras::new(be, &l);
            let now = be.now_us();
            reply(200, json!({"recordings": l.sets().iter().map(|s| recording_json(&l, s, &extras, now)).collect::<Vec<_>>()}))
        }
        ["recordings", id, tail @ ..] => {
            let Some(set) = find_set(&l, id) else { return error(404, "No such recording.") };
            match tail {
                [] => reply(200, recording_json(&l, &set, &Extras::new(be, &l), be.now_us())),
                ["voices", member] => match be.voice(room, &set.id, member) {
                    Some(body) => Response { status: 200, kind: "audio/mp4", body },
                    None => error(404, "No voice of that member in this recording."),
                },
                ["events"] if req.method == "POST" => add_event(be, &l, &set, req),
                ["events"] => events_reply(&l, &set, req),
                ["chapters"] => chapters_reply(&l, &set, be.now_us()),
                _ => transcript_reply(&l, &set, req),
            }
        }
        _ => error(404, "No such endpoint."),
    }
}

pub fn handle(req: &Request, be: &dyn Backend, token: &str, port: u16, own: &[String]) -> Response {
    if !host_ok(req.header("host"), port, own) {
        return error(403, "The Host header must be an address of this PC and the port.");
    }
    if req.method == "GET" && req.path == "/llms.txt" {
        return Response { status: 200, kind: "text/plain; charset=utf-8", body: LLMS.as_bytes().to_vec() };
    }
    if !token_ok(req.header("authorization"), token) {
        return error(401, "Send the token as: Authorization: Bearer <token>.");
    }
    let Some(owned) = segments(&req.path) else { return error(400, "Bad path.") };
    let parts: Vec<&str> = owned.iter().map(String::as_str).collect();
    route(req, be, &parts)
}

struct Live(Arc<Core>);

impl Backend for Live {
    fn app(&self) -> Value {
        json!({"app": env!("CARGO_PKG_NAME"), "version": crate::VERSION, "id": self.me(), "name": *self.0.name.lock().unwrap(), "room": self.0.room_id(), "recording": self.0.any_recording()})
    }

    fn me(&self) -> String {
        self.0.me()
    }

    fn status(&self) -> Vec<Row> {
        let core = &self.0;
        let (Some(room), Some(active)) = (core.room_id(), core.active_room()) else { return Vec::new() };
        let me = core.me();
        let sets = room::sets(core, &room, &room::logs(core, &room));
        let recording: HashSet<&str> = sets.iter().flat_map(|s| &s.files).filter(|f| f.recording() && f.view.is_none()).map(|f| f.owner.as_str()).collect();
        let peers = core.peers.lock().unwrap();
        active
            .members
            .iter()
            .map(|id| {
                if *id == me {
                    return Row { id: id.clone(), online: true, sharing: !core.engine.source.lock().unwrap().is_null(), recording: recording.contains(id.as_str()) && core.engine.working() };
                }
                let peer = peers.iter().find(|p| p.id.to_string() == *id);
                Row { id: id.clone(), online: peer.is_some_and(|p| p.alive().is_some()), sharing: peer.is_some_and(|p| p.live.is_some()), recording: recording.contains(id.as_str()) }
            })
            .collect()
    }

    fn now_us(&self) -> i64 {
        crate::now_us()
    }

    fn heads(&self) -> Vec<Head> {
        let core = &self.0;
        let (list, active) = {
            let rooms = core.rooms.lock().unwrap();
            (rooms.list.clone(), rooms.active.clone())
        };
        let me = core.me();
        let online: HashSet<String> = core.peers.lock().unwrap().iter().filter(|p| p.alive().is_some()).map(|p| p.id.to_string()).collect();
        list.into_iter()
            .map(|r| {
                let people = r.people();
                let names = room::names(core, Some(&r), true);
                let present = people.iter().filter(|m| core.present(m)).cloned().collect();
                Head { active: active.as_deref() == Some(&r.id), members: r.members.iter().map(|m| (m.clone(), *m == me || online.contains(m))).collect(), id: r.id, name: r.name, names, present }
            })
            .collect()
    }

    fn logs(&self, room: &str) -> Vec<Log> {
        room::logs(&self.0, room)
    }

    fn plugins(&self, room: &str) -> Vec<Value> {
        crate::jobs::plugins_json(&self.0, room)
    }

    fn analysis(&self, room: &str) -> HashMap<String, Vec<Value>> {
        crate::jobs::room_analysis(&self.0, room, &room::logs(&self.0, room))
    }

    fn voice(&self, room: &str, set: &str, member: &str) -> Option<Vec<u8>> {
        let core = &self.0;
        let found = room::sets(core, room, &room::logs(core, room)).into_iter().find(|s| s.id == set)?;
        let file = found.files.iter().find(|f| f.owner == member)?;
        let mut out = Vec::new();
        for capture in file.captures() {
            let pieces = crate::stream::set_pieces(core, room, file, capture, room::SERIES[1]);
            if pieces.len() > 1 {
                out.extend(crate::stream::read(&pieces, 0, crate::stream::size(&pieces)).ok()?);
            }
        }
        (!out.is_empty()).then_some(out)
    }

    fn add(&self, room: &str, record: Value) -> Result<(), String> {
        if self.0.room_id().as_deref() != Some(room) {
            return Err("Enter the room first: the app writes only to the room that is open.".into());
        }
        self.0.append_log(room, &record);
        party::send(&self.0);
        Ok(())
    }

    fn call(&self, name: &str, args: Value) -> Option<Result<Value, String>> {
        crate::ops::find(name).map(|op| op(&self.0, args))
    }

    fn diagnostics(&self, everyone: bool) -> Result<String, String> {
        crate::debug::diagnostics(&self.0, everyone, None)
    }
}

fn answer(stream: &mut TcpStream, response: &Response) {
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n",
        response.status,
        reason(response.status),
        response.kind,
        response.body.len()
    );
    let _ = stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(&response.body)).and_then(|_| stream.flush());
}

fn serve_one(mut stream: TcpStream, core: &Arc<Core>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let response = match read_request(&mut stream) {
        Ok(request) => {
            let token = core.api_token.lock().unwrap().clone();
            let own = if *core.api_lan.lock().unwrap() { local_hosts() } else { Vec::new() };
            handle(&request, &Live(core.clone()), &token, port(), &own)
        }
        Err(response) => response,
    };
    answer(&mut stream, &response);
}

fn serve(listener: TcpListener, core: Arc<Core>, stop: Arc<AtomicBool>) {
    let open = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                if open.load(Ordering::Relaxed) >= CONNECTIONS_MAX {
                    answer(&mut stream, &error(503, "Too many requests."));
                    continue;
                }
                open.fetch_add(1, Ordering::Relaxed);
                let (core, open) = (core.clone(), open.clone());
                std::thread::spawn(move || {
                    serve_one(stream, &core);
                    open.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(_) => std::thread::sleep(POLL),
        }
    }
}

pub fn reference() -> Vec<Value> {
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    let mut inside = false;
    for line in LLMS.lines() {
        if METHODS.iter().any(|m| line.starts_with(m)) {
            out.push((line.to_string(), Vec::new()));
            inside = true;
        } else if inside && line.starts_with("  ") {
            out.last_mut().unwrap().1.push(line.trim().to_string());
        } else {
            inside = false;
        }
    }
    out.into_iter().map(|(title, lines)| json!({"title": title, "lines": lines})).collect()
}

struct Running {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

static SERVER: Mutex<Option<Running>> = Mutex::new(None);

pub fn apply(core: &Arc<Core>) {
    let mut server = SERVER.lock().unwrap();
    if let Some(running) = server.take() {
        running.stop.store(true, Ordering::Relaxed);
        let _ = running.thread.join();
    }
    forget(&core.dir);
    core.api_error.lock().unwrap().clear();
    if !*core.api_on.lock().unwrap() {
        return;
    }
    let bind = if *core.api_lan.lock().unwrap() { Ipv4Addr::UNSPECIFIED } else { Ipv4Addr::LOCALHOST };
    match TcpListener::bind((bind, port())).and_then(|l| l.set_nonblocking(true).map(|_| l)) {
        Ok(listener) => {
            tracing::info!("the local API listens on {bind}:{}", port());
            let stop = Arc::new(AtomicBool::new(false));
            let (c, s) = (core.clone(), stop.clone());
            *server = Some(Running { stop, thread: std::thread::spawn(move || serve(listener, c, s)) });
            drop(server);
            refresh(core);
        }
        Err(e) => {
            tracing::warn!("the local API cannot listen on {}: {e}", address());
            *core.api_error.lock().unwrap() = "The port is in use".into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORT_FOR_TESTS: u16 = 4000;
    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";
    const S: i64 = 1_000_000;
    const START: i64 = 1_000 * S;

    struct Fake {
        heads: Vec<(String, bool)>,
        logs: Mutex<Vec<Log>>,
        clock: std::sync::atomic::AtomicI64,
        calls: Mutex<Vec<String>>,
        report_fails: AtomicBool,
    }

    impl Fake {
        fn new(records_a: Vec<Value>, records_b: Vec<Value>) -> Self {
            let log = |owner: &str, offset_us: i64, records: Vec<Value>| Log { owner: owner.into(), offset_us, records: Arc::new(records) };
            Self { heads: vec![("r1".into(), true), ("r2".into(), false)], logs: Mutex::new(vec![log("a", 0, records_a), log("b", 0, records_b)]), clock: std::sync::atomic::AtomicI64::new(START + 100 * S), calls: Mutex::new(Vec::new()), report_fails: Default::default() }
        }
    }

    impl Backend for Fake {
        fn app(&self) -> Value {
            json!({"app": "ez-hanseikai", "version": "0", "id": "a"})
        }
        fn me(&self) -> String {
            "a".into()
        }
        fn status(&self) -> Vec<Row> {
            vec![Row { id: "a".into(), online: true, sharing: true, recording: false }, Row { id: "b".into(), online: true, sharing: true, recording: true }, Row { id: "c".into(), online: false, sharing: true, recording: true }, Row { id: "d".into(), online: true, sharing: false, recording: false }]
        }
        fn now_us(&self) -> i64 {
            self.clock.fetch_add(1, Ordering::Relaxed)
        }
        fn heads(&self) -> Vec<Head> {
            let names: HashMap<String, String> = [("a".to_string(), "Ann".to_string()), ("b".to_string(), "Ben".to_string())].into();
            self.heads.iter().map(|(id, active)| Head { id: id.clone(), name: format!("Room {id}"), active: *active, members: vec![("a".into(), true), ("b".into(), false)], names: names.clone(), present: HashSet::from(["a".to_string()]) }).chain([Head { id: "pending-x".into(), name: "".into(), active: false, members: vec![], names: HashMap::new(), present: HashSet::new() }]).collect()
        }
        fn logs(&self, _: &str) -> Vec<Log> {
            self.logs.lock().unwrap().iter().map(|l| Log { owner: l.owner.clone(), offset_us: l.offset_us, records: l.records.clone() }).collect()
        }
        fn plugins(&self, _: &str) -> Vec<Value> {
            vec![json!({"id": "transcribe", "name": "Transcription", "summary": "x", "version": "1", "on": true, "settings": {"model": "small"}, "ready": true, "runner": "a", "left": 1, "failed": 0, "waiting_for": "b"})]
        }
        fn analysis(&self, _: &str) -> HashMap<String, Vec<Value>> {
            HashMap::from([("s1".to_string(), vec![json!({"plugin": "transcribe", "state": "left", "reason": "waiting", "waiting_for": "b"})])])
        }
        fn voice(&self, _: &str, set: &str, member: &str) -> Option<Vec<u8>> {
            (set == "s2" && member == "b").then(|| b"\0\0\0\x18ftypmp42".to_vec())
        }
        fn add(&self, room: &str, record: Value) -> Result<(), String> {
            if room != "r1" {
                return Err("Enter the room first.".into());
            }
            Arc::make_mut(&mut self.logs.lock().unwrap()[0].records).push(record);
            Ok(())
        }
        fn call(&self, name: &str, args: Value) -> Option<Result<Value, String>> {
            if name == "nothing" {
                return None;
            }
            self.calls.lock().unwrap().push(format!("{name} {args}"));
            Some(if args["bad"] == true { Err("Refused.".into()) } else { Ok(json!({"did": name})) })
        }
        fn diagnostics(&self, everyone: bool) -> Result<String, String> {
            if self.report_fails.load(Ordering::Relaxed) {
                return Err("task panicked".into());
            }
            Ok(format!("report everyone={everyone}"))
        }
    }

    fn plugin_records(id: &str, owner: &str) -> Vec<Value> {
        let p = crate::plugins::find(id).unwrap();
        let config = crate::jobs::Config::new(p, true, &Value::Null, Some(owner.to_string()));
        let key = format!("{}s1/{owner}", crate::jobs::prefix(id, &config));
        vec![
            json!({"k": "plugin", "plugin": id, "on": true, "version": p.version, "settings": config.settings, "t_us": 1}),
            json!({"k": "job", "state": "done", "job": key, "run": 7, "t_us": 2, "tags": {}}),
        ]
    }

    fn plugin_event(id: &str, owner: &str, kind: &str, from: i64, to: Option<i64>, text: &str, tags: Value) -> Value {
        let p = crate::plugins::find(id).unwrap();
        let mut target = json!({"set": "s1", "file": owner, "t_us": START + from * S});
        if let Some(to) = to {
            target = json!({"set": "s1", "file": owner, "from_us": START + from * S, "to_us": START + to * S});
        }
        json!({"k": "event", "id": START + from * S + 1, "t_us": 1, "kind": kind, "target": target, "text": text, "values": {}, "tags": tags, "plugin": {"id": id, "name": p.name, "version": p.version}, "job": "j", "run": 7})
    }

    fn fixture() -> Fake {
        let mut a = vec![
            json!({"k": "rec", "set": "s1", "start_us": START}),
            json!({"k": "seg", "capture": "111", "series": "v", "n": 1, "set": "s1", "t_us": START, "d_us": 4 * S, "size": 1, "sha256": "x"}),
            json!({"k": "rec_end", "set": "s1", "t_us": START + 90 * S}),
            json!({"k": "rec", "set": "s2", "start_us": START + 200 * S, "voice": true}),
            json!({"k": "rec_end", "set": "s2", "t_us": START + 260 * S}),
            json!({"k": "party", "set": "s2", "t_us": START + 210 * S, "shared": {"set": "s1"}}),
            room::user_event(START + 10 * S, "note", "Late rotate #rotate", json!({"t_us": START + 30 * S, "set": "s1"}), Value::Null),
            room::user_event(START + 11 * S, "mark", "", json!({"t_us": START + 40 * S}), Value::Null),
            room::user_event(START + 12 * S, "note", "Whole recording", json!({"set": "s1"}), Value::Null),
            room::user_event(START + 13 * S, "board", "# Plan", json!({}), json!({"card": "team"})),
            room::user_event(START + 14 * S, "board", "# Other card", json!({}), json!({"card": "other"})),
        ];
        a.extend(plugin_records("apex-matches", "a"));
        a.extend(plugin_events_a());
        let voice = |capture: &str| json!({"k": "seg", "capture": capture, "series": "m", "n": 1, "set": "s2", "t_us": START + 200 * S, "d_us": 4 * S, "size": 1, "sha256": "x"});
        let mut b = vec![json!({"k": "rec", "set": "s1", "start_us": START + 2 * S}), json!({"k": "rec_end", "set": "s1", "t_us": START + 88 * S}), json!({"k": "rec", "set": "s2", "start_us": START + 200 * S, "voice": true}), voice("111"), voice("222")];
        b.extend(plugin_records("transcribe", "b"));
        b.push(plugin_event("transcribe", "b", "speech", 20, Some(24), "Drop north", json!([])));
        b.push(plugin_event("transcribe", "b", "speech", 5, Some(8), "Ready", json!([])));
        Fake::new(a, b)
    }

    fn plugin_events_a() -> Vec<Value> {
        vec![
            plugin_event("apex-matches", "a", "match_end", 60, None, "", json!(["match"])),
            plugin_event("apex-matches", "a", "match_end", 30, None, "", json!(["match"])),
        ]
    }

    fn call(be: &dyn Backend, method: &str, path: &str, body: &str) -> (u16, Value) {
        let text = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{PORT_FOR_TESTS}\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: {}\r\n\r\n{body}", body.len());
        let request = read_request(&mut text.as_bytes()).ok().unwrap();
        let response = handle(&request, be, TOKEN, PORT_FOR_TESTS, &[]);
        (response.status, serde_json::from_slice(&response.body).unwrap_or(Value::Null))
    }

    fn get(be: &dyn Backend, path: &str) -> (u16, Value) {
        call(be, "GET", path, "")
    }

    #[test]
    fn only_the_loopback_address_and_port_are_a_valid_host() {
        assert!(host_ok(Some("127.0.0.1:4000"), 4000, &[]));
        assert!(host_ok(Some("localhost:4000"), 4000, &[]));
        for bad in [None, Some("127.0.0.1"), Some("127.0.0.1:4001"), Some("evil.example:4000"), Some("127.0.0.1.evil.example:4000"), Some("localhost.evil.example:4000"), Some("[::1]:4000"), Some("0.0.0.0:4000"), Some("")] {
            assert!(!host_ok(bad, 4000, &[]), "{bad:?}");
        }
    }

    #[test]
    fn a_pcs_own_addresses_and_name_are_valid_hosts_only_when_given() {
        let own = vec!["192.0.2.20".to_string(), "198.51.100.7".to_string(), "Gaming-PC".to_string()];
        for good in ["192.0.2.20:4000", "198.51.100.7:4000", "gaming-pc:4000", "GAMING-PC:4000", "127.0.0.1:4000", "localhost:4000"] {
            assert!(host_ok(Some(good), 4000, &own), "{good}");
        }
        for bad in ["192.0.2.21:4000", "192.0.2.20:4001", "192.0.2.20", "gaming-pc.evil.example:4000", "evil.example:4000", "gaming-pc:80", "0.0.0.0:4000", ""] {
            assert!(!host_ok(Some(bad), 4000, &own), "{bad}");
        }
        assert!(!host_ok(Some("192.0.2.20:4000"), 4000, &[]));
    }

    #[test]
    fn the_whole_handler_uses_the_hosts_it_is_given() {
        let fake = fixture();
        let text = format!("GET /v1 HTTP/1.1\r\nHost: 192.0.2.20:4000\r\nAuthorization: Bearer {TOKEN}\r\n\r\n");
        let request = read_request(&mut text.as_bytes()).ok().unwrap();
        assert_eq!(handle(&request, &fake, TOKEN, 4000, &[]).status, 403);
        assert_eq!(handle(&request, &fake, TOKEN, 4000, &["192.0.2.20".to_string()]).status, 200);
        let wrong = text.replace(TOKEN, "nope");
        let request = read_request(&mut wrong.as_bytes()).ok().unwrap();
        assert_eq!(handle(&request, &fake, TOKEN, 4000, &["192.0.2.20".to_string()]).status, 401);
    }

    #[test]
    fn the_token_must_match_exactly() {
        assert!(token_ok(Some(&format!("Bearer {TOKEN}")), TOKEN));
        assert!(token_ok(Some(&format!("bearer {TOKEN}")), TOKEN));
        for bad in [None, Some(""), Some(TOKEN), Some("Bearer "), Some("Bearer x"), Some(&format!("Bearer {}x", TOKEN)[..]), Some(&format!("Basic {TOKEN}")[..]), Some(&format!("Bearer {}", &TOKEN[1..])[..])] {
            assert!(!token_ok(bad, TOKEN), "{bad:?}");
        }
        assert!(!token_ok(Some("Bearer "), ""));
        assert!(!token_ok(Some("Bearer x"), ""));
        assert!(token_valid(TOKEN) && !token_valid("short") && !token_valid(&"g".repeat(TOKEN_BYTES * 2)));
    }

    #[test]
    fn requests_are_read_with_their_query_and_body() {
        let text = "POST /v1/rooms/a%20b/board?tag=%23x+y&kind=note HTTP/1.1\r\nHost: h\r\nContent-Length: 5\r\n\r\nhello and more";
        let r = read_request(&mut text.as_bytes()).ok().unwrap();
        assert_eq!((r.method.as_str(), r.path.as_str()), ("POST", "/v1/rooms/a%20b/board"));
        assert_eq!((r.query("tag"), r.query("kind")), (Some("#x y"), Some("note")));
        assert_eq!(r.body, b"hello");
        assert_eq!(segments(&r.path).unwrap(), ["v1", "rooms", "a b", "board"]);
        assert!(segments("/v1/%zz").is_none());
        assert!(segments("/v1/%ff").is_none());
        assert_eq!(decode("set%2Fr%2F2026-01-02%2012-00-00%2Fa", false).as_deref(), Some("set/r/2026-01-02 12-00-00/a"));
        assert_eq!((decode("a%zz", false), decode("a+b", false).as_deref(), decode("a+b", true).as_deref()), (None, Some("a+b"), Some("a b")));
    }

    #[test]
    fn malformed_and_oversized_requests_are_refused() {
        let status = |text: &str| read_request(&mut text.as_bytes()).err().map(|r| r.status);
        assert_eq!(status("GET /x HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n"), Some(400));
        assert_eq!(status("GET x HTTP/1.1\r\n\r\n"), Some(400));
        assert_eq!(status("GET / FTP/1.1\r\n\r\n"), Some(400));
        assert_eq!(status("GET / HTTP/1.1\r\nbroken\r\n\r\n"), Some(400));
        assert_eq!(status("GET / HTTP/1.1\r\n"), Some(400));
        assert_eq!(status(&format!("POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n", BODY_MAX + 1)), Some(413));
        assert_eq!(status("POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"), Some(400));
        assert_eq!(status("POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\nshort"), Some(400));
        assert_eq!(status(&format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "a".repeat(HEAD_MAX + 100))), Some(431));
    }

    #[test]
    fn a_wrong_host_or_token_gets_nothing_and_llms_txt_needs_no_token() {
        let fake = fixture();
        let send = |host: &str, auth: &str, path: &str| {
            let text = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\n{auth}\r\n");
            handle(&read_request(&mut text.as_bytes()).ok().unwrap(), &fake, TOKEN, PORT_FOR_TESTS, &[])
        };
        let ok_auth = format!("Authorization: Bearer {TOKEN}\r\n");
        assert_eq!(send("evil.example:4000", &ok_auth, "/v1/rooms").status, 403);
        assert_eq!(send("evil.example:4000", &ok_auth, "/llms.txt").status, 403);
        assert_eq!(send("127.0.0.1:4000", "", "/v1/rooms").status, 401);
        assert_eq!(send("127.0.0.1:4000", "Authorization: Bearer nope\r\n", "/v1/rooms").status, 401);
        let llms = send("127.0.0.1:4000", "", "/llms.txt");
        assert_eq!(llms.status, 200);
        assert!(llms.kind.starts_with("text/plain"));
        assert!(String::from_utf8(llms.body).unwrap().contains("Authorization: Bearer"));
        assert_eq!(send("127.0.0.1:4000", &ok_auth, "/v1").status, 200);
    }

    #[test]
    fn rooms_list_their_members_and_skip_pending_ones() {
        let (status, body) = get(&fixture(), "/v1/rooms");
        assert_eq!(status, 200);
        assert_eq!(body["rooms"].as_array().unwrap().len(), 2);
        assert_eq!(body["rooms"][0]["name"], "Room r1");
        assert_eq!(body["rooms"][0]["active"], true);
        assert_eq!(body["rooms"][0]["members"], json!([{"id": "a", "name": "Ann", "online": true}, {"id": "b", "name": "Ben", "online": false}]));
    }

    #[test]
    fn a_recording_whose_open_file_belongs_to_an_absent_member_reads_as_ended() {
        let seg = |n: i64| json!({"k": "seg", "capture": "222", "series": "m", "n": n, "set": "s1", "t_us": START + n * 4 * S, "d_us": 4 * S, "size": 1, "sha256": "x"});
        let fake = Fake::new(vec![json!({"k": "rec", "set": "s1", "start_us": START}), json!({"k": "rec_end", "set": "s1", "t_us": START + 10 * S})], vec![json!({"k": "rec", "set": "s1", "start_us": START}), seg(1), seg(2)]);
        let (_, body) = get(&fake, "/v1/rooms/r1/recordings/s1");
        assert_eq!((body["recording"].as_bool(), body["end_ms"].as_i64()), (Some(false), Some((START + 12 * S) / 1000)));
    }

    #[test]
    fn recordings_come_newest_first_with_members_length_and_kind() {
        let (status, body) = get(&fixture(), "/v1/rooms/r1/recordings");
        assert_eq!(status, 200);
        let list = body["recordings"].as_array().unwrap();
        assert_eq!(list.iter().map(|r| r["id"].as_str().unwrap()).collect::<Vec<_>>(), ["s2", "s1"]);
        let s1 = &list[1];
        assert_eq!((s1["kind"].as_str(), s1["recording"].as_bool()), (Some("play"), Some(false)));
        assert_eq!((s1["start_ms"].as_i64(), s1["end_ms"].as_i64(), s1["length_s"].as_f64()), (Some(1_000_000), Some(1_090_000), Some(90.0)));
        assert_eq!(s1["members"].as_array().unwrap().len(), 2);
        assert_eq!((s1["members"][0]["id"].as_str(), s1["members"][0]["name"].as_str(), s1["members"][0]["voice_only"].as_bool()), (Some("a"), Some("Ann"), Some(false)));
        assert_eq!(s1["members"][1]["voice_only"], true);
        let s2 = &list[0];
        assert_eq!((s2["kind"].as_str(), s2["length_s"].as_f64()), (Some("debrief"), Some(60.0)));
        let (status, one) = get(&fixture(), "/v1/rooms/r1/recordings/s1");
        assert_eq!((status, one["id"].as_str()), (200, Some("s1")));
        assert_eq!(get(&fixture(), "/v1/rooms/r1/recordings/nope").0, 404);
        assert_eq!(get(&fixture(), "/v1/rooms/nope/recordings").0, 404);
        assert_eq!(get(&fixture(), "/v1/rooms/pending-x/recordings").0, 404);
    }

    #[test]
    fn events_have_times_from_the_recording_start_tags_and_authors() {
        let fake = fixture();
        let (status, body) = get(&fake, "/v1/rooms/r1/recordings/s1/events");
        assert_eq!(status, 200);
        let list = body["events"].as_array().unwrap();
        let kinds: Vec<(&str, Option<f64>)> = list.iter().map(|e| (e["kind"].as_str().unwrap(), e["at_s"].as_f64())).collect();
        assert_eq!(kinds, [("note", None), ("note", Some(30.0)), ("match_end", Some(30.0)), ("mark", Some(40.0)), ("match_end", Some(60.0))]);
        let note = list.iter().find(|e| e["text"] == "Late rotate #rotate").unwrap();
        assert_eq!((note["tags"].clone(), note["author"].as_str(), note["author_name"].as_str(), note["plugin"].clone()), (json!(["rotate"]), Some("a"), Some("Ann"), Value::Null));
        let apex = list.iter().find(|e| e["kind"] == "match_end").unwrap();
        assert_eq!((apex["plugin"].as_str(), apex["tags"].clone(), apex["member"].as_str()), (Some("Apex matches"), json!(["match"]), Some("a")));
        let (_, tagged) = get(&fake, "/v1/rooms/r1/recordings/s1/events?tag=%23ROTATE");
        assert_eq!(tagged["events"].as_array().unwrap().len(), 1);
        let (_, marks) = get(&fake, "/v1/rooms/r1/recordings/s1/events?kind=mark");
        assert_eq!(marks["events"].as_array().unwrap().len(), 1);
        let (_, speech) = get(&fake, "/v1/rooms/r1/recordings/s1/events?kind=speech");
        assert_eq!(speech["events"].as_array().unwrap().len(), 2);
        let (_, board) = get(&fake, "/v1/rooms/r1/recordings/s1/events?kind=board");
        assert_eq!(board["events"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn chapters_are_the_matches_between_match_ends() {
        let fake = fixture();
        let (status, body) = get(&fake, "/v1/rooms/r1/recordings/s1/chapters");
        assert_eq!(status, 200);
        assert_eq!(body["chapters"], json!([{"n": 1, "name": "Match 1", "from_s": 0.0, "to_s": 30.0}, {"n": 2, "name": "Match 2", "from_s": 30.0, "to_s": 60.0}]));
        assert_eq!(get(&fake, "/v1/rooms/r1/recordings/s2/chapters").1["chapters"], json!([]));
    }

    #[test]
    fn the_transcript_lists_spoken_lines_in_time_order_with_the_speaker() {
        let fake = fixture();
        let (status, body) = get(&fake, "/v1/rooms/r1/recordings/s1/transcript");
        assert_eq!(status, 200);
        assert_eq!(body["lines"], json!([
            {"member": "b", "member_name": "Ben", "from_s": 5.0, "to_s": 8.0, "text": "Ready"},
            {"member": "b", "member_name": "Ben", "from_s": 20.0, "to_s": 24.0, "text": "Drop north"},
        ]));
        assert_eq!(get(&fake, "/v1/rooms/r1/recordings/s1/transcript?member=a").1["lines"], json!([]));
    }

    #[test]
    fn the_board_is_the_newest_team_card_and_can_be_replaced() {
        let fake = fixture();
        let (status, board) = get(&fake, "/v1/rooms/r1/board");
        assert_eq!((status, board["text"].as_str(), board["author_name"].as_str(), board["updated_ms"].as_i64()), (200, Some("# Plan"), Some("Ann"), Some((START + 13 * S) / 1000)));
        let (status, put) = call(&fake, "PUT", "/v1/rooms/r1/board", r##"{"text": "# New plan\n**Goal**"}"##);
        assert_eq!((status, put["text"].as_str()), (200, Some("# New plan\n**Goal**")));
        assert_eq!(get(&fake, "/v1/rooms/r1/board").1["text"], "# New plan\n**Goal**");
        assert_eq!(call(&fake, "PUT", "/v1/rooms/r1/board", r#"{"text": "  "}"#).0, 400);
        assert_eq!(call(&fake, "PUT", "/v1/rooms/r1/board", r#"{"text": 5}"#).0, 400);
        assert_eq!(call(&fake, "PUT", "/v1/rooms/r1/board", "nonsense").0, 400);
        assert_eq!(call(&fake, "PUT", "/v1/rooms/r1/board", &json!({"text": "x".repeat(crate::BOARD_MAX + 1)}).to_string()).0, 400);
        assert_eq!(call(&fake, "PUT", "/v1/rooms/r2/board", r#"{"text": "x"}"#).0, 409);
        let empty = Fake::new(vec![], vec![]);
        assert_eq!(get(&empty, "/v1/rooms/r1/board").1["text"], "");
    }

    #[test]
    fn a_note_added_through_the_api_is_an_ordinary_event_with_its_tags() {
        let fake = fixture();
        let (status, made) = call(&fake, "POST", "/v1/rooms/r1/recordings/s1/events", r##"{"text": "Check the ring #ring", "at_s": 12.34, "tags": ["Mistake", "#ring", "bad tag!"]}"##);
        assert_eq!(status, 201);
        assert_eq!((made["kind"].as_str(), made["at_s"].as_f64(), made["author_name"].as_str()), (Some("note"), Some(12.3), Some("Ann")));
        assert_eq!(made["text"], "Check the ring #ring #Mistake #badtag");
        assert_eq!(made["tags"], json!(["ring", "Mistake", "badtag"]));
        assert_eq!(fake.logs.lock().unwrap()[0].records.last().unwrap()["source"], "ai");
        let (_, list) = get(&fake, "/v1/rooms/r1/recordings/s1/events?tag=mistake");
        assert_eq!(list["events"].as_array().unwrap().len(), 1);
        assert_eq!(list["events"][0]["key"], made["key"]);
        let (status, mark) = call(&fake, "POST", "/v1/rooms/r1/recordings/s1/events", r#"{"kind": "mark", "at_s": 5, "tags": ["good"]}"#);
        assert_eq!((status, mark["kind"].as_str(), mark["text"].as_str(), mark["at_s"].as_f64()), (201, Some("mark"), Some("#good"), Some(5.0)));
        let (status, whole) = call(&fake, "POST", "/v1/rooms/r1/recordings/s1/events", r#"{"text": "Overall fine"}"#);
        assert_eq!((status, whole["at_s"].clone()), (201, Value::Null));
    }

    #[test]
    fn bad_events_are_refused_without_writing() {
        let fake = fixture();
        let before = fake.logs.lock().unwrap()[0].records.len();
        let post = |body: &str| call(&fake, "POST", "/v1/rooms/r1/recordings/s1/events", body).0;
        assert_eq!(post("[]"), 400);
        assert_eq!(post("{}"), 400);
        assert_eq!(post(r#"{"text": "  "}"#), 400);
        assert_eq!(post(r#"{"text": "x", "kind": "drawing"}"#), 400);
        assert_eq!(post(r#"{"text": "x", "at_s": -1}"#), 400);
        assert_eq!(post(r#"{"text": "x", "at_s": "soon"}"#), 400);
        assert_eq!(post(r#"{"text": "x", "at_s": 1e12}"#), 400);
        assert_eq!(post(r#"{"kind": "mark"}"#), 400);
        assert_eq!(post(r#"{"text": "x", "tags": "a"}"#), 400);
        assert_eq!(post(r#"{"text": "x", "tags": [1]}"#), 400);
        assert_eq!(post(&json!({"text": "x".repeat(crate::NOTE_MAX + 1)}).to_string()), 400);
        assert_eq!(post(&json!({"text": "x", "tags": (0..=TAGS_MAX).map(|i| format!("t{i}")).collect::<Vec<_>>()}).to_string()), 400);
        assert_eq!(call(&fake, "POST", "/v1/rooms/r1/recordings/nope/events", r#"{"text": "x"}"#).0, 404);
        assert_eq!(call(&fake, "POST", "/v1/rooms/r2/recordings/s1/events", r#"{"text": "x"}"#).0, 409);
        assert_eq!(fake.logs.lock().unwrap()[0].records.len(), before);
    }

    #[test]
    fn an_imported_recording_shows_its_title_importer_and_views() {
        let importer = iroh::SecretKey::generate().public().to_string();
        let view = iroh::SecretKey::generate().public().to_string();
        let rec = json!({"k": "rec", "set": "s7", "start_us": START, "imported": importer, "view": "Walkthrough", "title": "What went wrong"});
        let end = json!({"k": "rec_end", "set": "s7", "t_us": START + 60 * S});
        let log = |owner: &str, records: Vec<Value>| Log { owner: owner.into(), offset_us: 0, records: Arc::new(records) };
        let fake = Fake::new(vec![], vec![]);
        *fake.logs.lock().unwrap() = vec![log(&importer, vec![]), log(&view, vec![rec, end])];
        let (status, one) = get(&fake, "/v1/rooms/r1/recordings/s7");
        assert_eq!(status, 200);
        assert_eq!((one["title"].as_str(), one["imported_by"]["id"].as_str(), one["members"][0]["imported"].as_bool(), one["members"][0]["id"].as_str()), (Some("What went wrong"), Some(importer.as_str()), Some(true), Some(view.as_str())));
        let (_, plain) = get(&fixture(), "/v1/rooms/r1/recordings/s1");
        assert_eq!((plain["title"].clone(), plain["imported_by"].clone(), plain["members"][0]["imported"].clone()), (Value::Null, Value::Null, json!(false)));
    }

    #[test]
    fn an_operation_is_called_by_name_with_its_json_arguments() {
        let fake = fixture();
        assert_eq!(call(&fake, "POST", "/v1/ops/create_room", r#"{"name": "Team", "n": 2}"#), (200, json!({"did": "create_room"})));
        assert_eq!(call(&fake, "POST", "/v1/ops/state", ""), (200, json!({"did": "state"})));
        assert_eq!(fake.calls.lock().unwrap().as_slice(), [r#"create_room {"n":2,"name":"Team"}"#, "state {}"]);
        assert_eq!(call(&fake, "POST", "/v1/ops/mark", r#"{"bad": true}"#), (400, json!({"error": "Refused."})));
        assert_eq!(call(&fake, "POST", "/v1/ops/nothing", "{}"), (404, json!({"error": "No such operation."})));
        assert_eq!(call(&fake, "GET", "/v1/ops/state", "").0, 405);
        for bad in ["[]", "null", "nonsense", "3"] {
            assert_eq!(call(&fake, "POST", "/v1/ops/state", bad), (400, json!({"error": "Send a JSON object."})), "{bad}");
        }
        assert_eq!(fake.calls.lock().unwrap().len(), 3);
    }

    #[test]
    fn unknown_paths_and_methods_are_refused() {
        let fake = fixture();
        assert_eq!(get(&fake, "/v1/nothing").0, 404);
        assert_eq!(get(&fake, "/").0, 404);
        assert_eq!(get(&fake, "/v1/rooms/r1/recordings/s1/clips").0, 404);
        assert_eq!(call(&fake, "DELETE", "/v1/rooms/r1/board", "").0, 405);
        assert_eq!(call(&fake, "POST", "/v1/rooms", "{}").0, 405);
        assert_eq!(call(&fake, "PUT", "/v1/rooms/r1/recordings/s1/events", "{}").0, 405);
        assert_eq!(call(&fake, "POST", "/v1/rooms/r1/plugins", "{}").0, 405);
        assert_eq!(get(&fake, "/v1/rooms/r1/recordings/%zz").0, 400);
    }

    #[test]
    fn plugins_show_their_runner_and_what_is_left_and_recordings_their_analysis() {
        let fake = fixture();
        let (status, plugins) = get(&fake, "/v1/rooms/r1/plugins");
        assert_eq!(status, 200);
        let first = &plugins["plugins"][0];
        assert_eq!((first["id"].as_str(), first["on"].as_bool(), first["runner_name"].as_str(), first["left"].as_u64(), first["waiting_for_name"].as_str()), (Some("transcribe"), Some(true), Some("Ann"), Some(1), Some("Ben")));
        assert!(first.get("summary").is_none() && first.get("workers").is_none());
        assert_eq!(get(&fake, "/v1/rooms/r1/jobs").0, 404);
        let (_, s1) = get(&fake, "/v1/rooms/r1/recordings/s1");
        assert_eq!(s1["analysis"][0], json!({"plugin": "transcribe", "state": "left", "reason": "waiting", "waiting_for": "b"}));
        assert_eq!(get(&fake, "/v1/rooms/r1/recordings/s2").1["analysis"], json!([]));
    }

    #[test]
    fn a_debrief_names_its_recording_carries_the_watch_and_serves_its_voices() {
        let fake = fixture();
        let (_, s2) = get(&fake, "/v1/rooms/r1/recordings/s2");
        assert_eq!((s2["kind"].as_str(), s2["of"].as_str()), (Some("debrief"), Some("s1")));
        assert_eq!(s2["watch"].as_array().unwrap().len(), 1);
        assert_eq!(s2["watch"][0]["set"], "s1");
        let (_, s1) = get(&fake, "/v1/rooms/r1/recordings/s1");
        assert!(s1.get("of").is_none() && s1.get("watch").is_none());
        let text = format!("GET /v1/rooms/r1/recordings/s2/voices/b HTTP/1.1\r\nHost: 127.0.0.1:{PORT_FOR_TESTS}\r\nAuthorization: Bearer {TOKEN}\r\n\r\n");
        let voice = handle(&read_request(&mut text.as_bytes()).ok().unwrap(), &fake, TOKEN, PORT_FOR_TESTS, &[]);
        assert_eq!((voice.status, voice.kind), (200, "audio/mp4"));
        assert_eq!(&voice.body[4..8], b"ftyp");
        assert_eq!(get(&fake, "/v1/rooms/r1/recordings/s2/voices/a").0, 404);
        assert_eq!(call(&fake, "POST", "/v1/rooms/r1/recordings/s2/voices/b", "{}").0, 405);
    }

    #[test]
    fn the_app_info_names_the_open_room() {
        let (status, info) = get(&fixture(), "/v1");
        assert_eq!((status, info["app"].as_str()), (200, Some("ez-hanseikai")));
    }

    #[test]
    fn status_tells_the_state_of_this_pc_and_every_member() {
        let (status, body) = get(&fixture(), "/v1/status");
        assert_eq!(status, 200);
        assert_eq!(body["state"], "standby");
        assert_eq!(body["recording"], Value::Null);
        let states: Vec<(&str, &str, bool)> = body["members"].as_array().unwrap().iter().map(|m| (m["name"].as_str().unwrap(), m["state"].as_str().unwrap(), m["online"].as_bool().unwrap())).collect();
        assert_eq!(states[..2], [("Ann", "standby", true), ("Ben", "recording", true)]);
        assert_eq!(states[2].1, "offline");
        assert!(!states[2].2);
        assert_eq!(states[3].1, "idle");
        assert_eq!(call(&fixture(), "POST", "/v1/status", "").0, 405);
    }

    #[test]
    fn diagnostics_give_this_pcs_report_or_everyones() {
        let fake = fixture();
        assert_eq!(get(&fake, "/v1/diagnostics"), (200, json!({"text": "report everyone=false"})));
        assert_eq!(get(&fake, "/v1/diagnostics?everyone=true").1["text"], "report everyone=true");
        assert_eq!(get(&fake, "/v1/diagnostics?everyone=1").1["text"], "report everyone=true");
        assert_eq!(get(&fake, "/v1/diagnostics?everyone=no").1["text"], "report everyone=false");
        assert_eq!(call(&fake, "POST", "/v1/diagnostics", "").0, 405);
        fake.report_fails.store(true, Ordering::Relaxed);
        assert_eq!(get(&fake, "/v1/diagnostics"), (500, json!({"error": "task panicked"})));
    }

    #[test]
    fn every_reference_entry_has_lines_and_the_main_endpoints_are_listed() {
        let list = reference();
        for entry in &list {
            assert!(!entry["lines"].as_array().unwrap().is_empty(), "{entry}");
        }
        assert!(list.iter().any(|e| e["title"] == "GET /v1/status") && list.iter().any(|e| e["title"] == "POST /v1/ops/{name}"));
    }

    #[test]
    fn the_address_file_holds_the_address_and_token_for_this_user_only() {
        let dir = crate::log::tests::temp("api-info");
        write_info(&dir, TOKEN).unwrap();
        let info: Value = serde_json::from_str(&std::fs::read_to_string(dir.join(INFO_FILE)).unwrap()).unwrap();
        assert_eq!((info["address"].as_str(), info["token"].as_str()), (Some(address().as_str()), Some(TOKEN)));
        let access = Command::new("icacls").arg(dir.join(INFO_FILE)).output().unwrap();
        let access = String::from_utf8_lossy(&access.stdout).to_string();
        assert!(!access.contains("Everyone") && !access.contains("BUILTIN\\Users") && !access.contains("Administrators") && !access.contains("SYSTEM"), "{access}");
        write_info(&dir, "0".repeat(48).as_str()).unwrap();
        assert!(std::fs::read_to_string(dir.join(INFO_FILE)).unwrap().contains(&"0".repeat(48)));
        forget(&dir);
        assert!(!dir.join(INFO_FILE).exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn llms_txt_describes_every_endpoint() {
        for path in ["/v1/rooms", "recordings", "events", "chapters", "transcript", "voices", "board", "plugins", "/v1/status", "/v1/diagnostics", "/v1/ops", "create_room", "start_recording", "api.json", "--api"] {
            assert!(LLMS.contains(path), "{path}");
        }
        assert!(LLMS.contains("Authorization: Bearer") && LLMS.contains("127.0.0.1"));
        assert!(!LLMS.contains(TOKEN));
    }
}

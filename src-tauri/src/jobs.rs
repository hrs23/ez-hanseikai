use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::{Emitter, State};

use crate::plugins::{self, Found, Media, Output, Plugin, Reply};
use crate::room::{self, File, Log, Seg, SERIES};
use crate::{ffmpeg, hex, now_us, party, stream, Core};

const CHECK: Duration = Duration::from_secs(2);
const EMIT_EVERY: Duration = Duration::from_millis(250);
const SETTINGS_HASH_BYTES: usize = 6;
const CLAIM_SETTLE: Duration = Duration::from_secs(2 * crate::net::LOG_SYNC.as_secs());
const STOP_WAIT: Duration = Duration::from_secs(10);
pub const UNFINISHED_MAX: u32 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub on: bool,
    pub version: String,
    pub settings: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Claimed(i64),
    Done(i64),
    Failed(String),
    Retry,
}

#[derive(Default, Debug)]
pub struct Track {
    pub done: Vec<(String, i64)>,
    pub last: BTreeMap<String, Status>,
    pub tags: Vec<((String, i64), Value)>,
    pub unfinished: BTreeMap<String, u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Part {
    pub start_us: i64,
    pub end_us: i64,
    pub picture: Media,
    pub sound: Media,
    pub voice: Media,
}

#[derive(Clone, Debug, Default)]
pub struct Input {
    pub file: String,
    pub set: String,
    pub owner: String,
    pub start_us: i64,
    pub here: bool,
    pub parts: Vec<Part>,
}

#[derive(Debug, PartialEq)]
pub struct Pick {
    pub plugin: String,
    pub key: String,
    pub input: usize,
}

pub struct Running {
    pub room: String,
    pub key: String,
    pub plugin: String,
    pub file: String,
    pub run: i64,
    pub progress: f64,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
static FIRST: Mutex<String> = Mutex::new(String::new());
static CANCEL: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static WORKING: AtomicBool = AtomicBool::new(false);

fn capped<'a>(logs: &'a [Log], caps: Option<&'a HashMap<String, u64>>) -> impl Iterator<Item = (&'a Log, &'a [Value])> {
    logs.iter().map(move |log| {
        let cap = caps.map_or(usize::MAX, |c| c.get(&log.owner).copied().unwrap_or(0) as usize);
        (log, &log.records[..cap.min(log.records.len())])
    })
}

pub fn configs(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> BTreeMap<String, Config> {
    let mut latest: BTreeMap<String, ((i64, String), Config)> = BTreeMap::new();
    for (log, records) in capped(logs, caps) {
        for r in records.iter().filter(|r| r["k"] == "plugin") {
            let (Some(id), Some(t)) = (r["plugin"].as_str(), r["t_us"].as_i64()) else { continue };
            let at = (t - log.offset_us, log.owner.clone());
            if latest.get(id).is_none_or(|(old, _)| at > *old) {
                let config = Config { on: r["on"] == true, version: r["version"].as_str().unwrap_or_default().to_string(), settings: r["settings"].clone() };
                latest.insert(id.to_string(), (at, config));
            }
        }
    }
    latest.into_iter().map(|(id, (_, c))| (id, c)).collect()
}

pub fn prefix(plugin: &str, config: &Config) -> String {
    let sorted: BTreeMap<&String, &Value> = config.settings.as_object().into_iter().flatten().collect();
    let digest = Sha256::digest(serde_json::to_string(&sorted).unwrap_or_default().as_bytes());
    format!("{plugin}@{}/{}/", config.version, hex(&digest[..SETTINGS_HASH_BYTES]))
}

pub fn tracks(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> HashMap<String, Track> {
    let mut out: HashMap<String, Track> = HashMap::new();
    for (log, records) in capped(logs, caps) {
        for r in records.iter().filter(|r| r["k"] == "job") {
            let Some(key) = r["job"].as_str() else { continue };
            let run = r["run"].as_i64().unwrap_or(0);
            let status = match r["state"].as_str() {
                Some("claim") => Status::Claimed(run),
                Some("done") => Status::Done(run),
                Some("fail") => Status::Failed(r["error"].as_str().unwrap_or_default().to_string()),
                Some("retry" | "yield" | "stop") => Status::Retry,
                _ => continue,
            };
            let track = out.entry(key.to_string()).or_default();
            let unfinished = track.unfinished.entry(log.owner.clone()).or_default();
            *unfinished = if matches!(status, Status::Claimed(_)) { *unfinished + 1 } else { 0 };
            if status == Status::Done(run) {
                track.done.push((log.owner.clone(), run));
                track.tags.push(((log.owner.clone(), run), r["tags"].clone()));
            }
            track.last.insert(log.owner.clone(), status);
        }
    }
    out
}

pub fn chosen(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> HashMap<String, String> {
    let mut counts: HashMap<(String, String), u64> = HashMap::new();
    for (log, records) in capped(logs, caps) {
        for r in records.iter().filter(|r| r["k"] == "seg" && r["series"] == SERIES[0]) {
            let Some(set) = r["set"].as_str() else { continue };
            *counts.entry((set.to_string(), format!("{set}/{}", log.owner))).or_default() += 1;
        }
    }
    let mut out: HashMap<String, (u64, String)> = HashMap::new();
    for ((set, file), n) in counts {
        let better = out.get(&set).is_none_or(|(m, f)| (n, Reverse(&file)) > (*m, Reverse(f)));
        if better {
            out.insert(set, (n, file));
        }
    }
    out.into_iter().map(|(set, (_, file))| (set, file)).collect()
}

fn once(plugin: &str) -> bool {
    plugins::find(plugin).is_some_and(|p| p.per_set)
}

pub fn counts(plugin: &str, file: &str, chosen: &HashMap<String, String>) -> bool {
    file.matches('/').count() == 1 && (!once(plugin) || file.split('/').next().is_some_and(|set| chosen.get(set).is_some_and(|f| f == file)))
}

fn current(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> Vec<String> {
    configs(logs, caps).iter().filter(|(_, c)| c.on).map(|(id, c)| prefix(id, c)).collect()
}

fn winners(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> Vec<((String, i64), Value)> {
    let prefixes = current(logs, caps);
    let chosen = chosen(logs, caps);
    tracks(logs, caps)
        .into_iter()
        .filter(|(key, _)| prefixes.iter().any(|p| key.strip_prefix(p.as_str()).is_some_and(|file| counts(p.split('@').next().unwrap_or_default(), file, &chosen))))
        .filter_map(|(_, t)| {
            let best = t.done.iter().min()?.clone();
            let tags = t.tags.into_iter().find(|(w, _)| *w == best).map(|(_, v)| v).unwrap_or(Value::Null);
            Some((best, tags))
        })
        .collect()
}

pub fn shown(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> HashSet<(String, i64)> {
    winners(logs, caps).into_iter().map(|(w, _)| w).collect()
}

pub fn keep_shown(events: &mut Vec<Value>, logs: &[Log], caps: Option<&HashMap<String, u64>>) {
    let shown = shown(logs, caps);
    events.retain(|e| e["plugin"].is_null() || shown.contains(&(e["author"].as_str().unwrap_or_default().to_string(), e["run"].as_i64().unwrap_or(0))));
}

pub fn tag_names(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (_, tags) in winners(logs, caps) {
        for (tag, name) in tags.as_object().into_iter().flatten() {
            if let Some(name) = name.as_str() {
                out.insert(tag.clone(), name.to_string());
            }
        }
    }
    out
}

fn taken(track: Option<&Track>, me: &str, online: &HashSet<String>) -> bool {
    track.is_some_and(|t| t.last.iter().any(|(owner, s)| owner != me && matches!(s, Status::Claimed(_)) && online.contains(owner)))
}

pub fn yields(track: Option<&Track>, me: &str, run: i64, online: &HashSet<String>) -> bool {
    let earlier = |owner: &String, s: &Status| owner != me && online.contains(owner) && matches!(s, Status::Claimed(r) if (*r, owner.as_str()) < (run, me));
    track.is_some_and(|t| t.done.iter().any(|(owner, _)| owner != me) || t.last.iter().any(|(owner, s)| earlier(owner, s)))
}

#[allow(clippy::too_many_arguments)]
pub fn next(
    me: &str,
    online: &HashSet<String>,
    recording: bool,
    bundled: &[(&str, &str, bool)],
    ready: impl Fn(&str, &Value) -> bool,
    configs: &BTreeMap<String, Config>,
    tracks: &HashMap<String, Track>,
    chosen: &HashMap<String, String>,
    inputs: &[Input],
    first: &str,
) -> Option<Pick> {
    let mut picks = Vec::new();
    for (order, (id, config)) in configs.iter().enumerate() {
        let usable = bundled.iter().any(|(b, version, live)| *b == id.as_str() && *version == config.version && (!recording || *live));
        if !config.on || !usable || !ready(id, &config.settings) {
            continue;
        }
        let prefix = prefix(id, config);
        for (i, input) in inputs.iter().enumerate().filter(|(_, input)| input.here && counts(id, &input.file, chosen)) {
            let key = format!("{prefix}{}", input.file);
            let track = tracks.get(&key);
            if track.is_some_and(|t| !t.done.is_empty()) {
                continue;
            }
            let mine = track.and_then(|t| t.last.get(me));
            if matches!(mine, Some(Status::Failed(_))) {
                continue;
            }
            let resumed = matches!(mine, Some(Status::Claimed(_)));
            if !resumed && taken(track, me, online) {
                continue;
            }
            let rank = (key != first, !resumed, input.owner != me, Reverse(input.start_us), order);
            picks.push((rank, Pick { plugin: id.clone(), key, input: i }));
        }
    }
    picks.into_iter().min_by(|a, b| a.0.cmp(&b.0)).map(|(_, p)| p)
}

pub fn unfinished(me: &str, tracks: &HashMap<String, Track>) -> Vec<(String, i64)> {
    tracks
        .iter()
        .filter_map(|(key, t)| match t.last.get(me) {
            Some(Status::Claimed(run)) if t.done.is_empty() && t.unfinished.get(me).copied().unwrap_or(0) >= UNFINISHED_MAX => Some((key.clone(), *run)),
            _ => None,
        })
        .collect()
}

pub fn state(track: Option<&Track>, me: &str, online: &HashSet<String>) -> (String, Value, Value) {
    let Some(t) = track else { return ("waiting".into(), Value::Null, Value::Null) };
    if let Some((owner, _)) = t.done.iter().min() {
        return ("done".into(), json!(owner), Value::Null);
    }
    let claimed = t.last.iter().find(|(owner, s)| matches!(s, Status::Claimed(_)) && (*owner == me || online.contains(*owner)));
    if let Some((owner, _)) = claimed {
        return ("claimed".into(), json!(owner), Value::Null);
    }
    match t.last.iter().find(|(_, s)| matches!(s, Status::Failed(_))) {
        Some((owner, Status::Failed(e))) => ("failed".into(), json!(owner), json!(e)),
        _ => ("waiting".into(), Value::Null, Value::Null),
    }
}

pub fn moment(start_us: i64, s: f64) -> i64 {
    start_us + (s * 1e6).round() as i64
}

pub fn target(found: &Found, input: &Input, start_us: i64) -> Value {
    let mut t = match (found.at_s, found.from_s, found.to_s) {
        (Some(at), _, _) => json!({"t_us": moment(start_us, at)}),
        (None, Some(from), Some(to)) => json!({"from_us": moment(start_us, from), "to_us": moment(start_us, to.max(from))}),
        _ => json!({}),
    };
    t["set"] = json!(input.set);
    t["file"] = json!(input.owner);
    t
}

pub fn event_record(plugin: &Plugin, key: &str, run: i64, input: &Input, start_us: i64, found: &Found, id: i64) -> Value {
    json!({
        "k": "event", "id": id, "t_us": id, "kind": found.kind, "target": target(found, input, start_us),
        "text": found.text, "values": found.values, "tags": found.tags,
        "plugin": {"id": plugin.id, "name": plugin.name, "version": plugin.version}, "job": key, "run": run,
    })
}

pub fn job_record(state: &str, key: &str, run: i64, extra: Value) -> Value {
    let mut r = json!({"k": "job", "state": state, "job": key, "run": run, "t_us": now_us()});
    crate::merge(&mut r, &extra);
    r
}

pub fn span(segs: &[&Seg], offset_us: i64) -> Option<(i64, i64)> {
    let (first, last) = (segs.first()?, segs.last()?);
    Some((first.t_us - offset_us, last.t_us + last.d_us - offset_us))
}

fn series(core: &Core, room: &str, f: &File, capture: &str, series: &str) -> Option<Vec<PathBuf>> {
    let count = f.segs.iter().filter(|s| s.capture == capture && s.series == series).count();
    if count == 0 {
        return Some(Vec::new());
    }
    let pieces = stream::set_pieces(core, room, f, capture, series);
    (pieces.len() == count + 1).then(|| pieces.into_iter().map(|p| p.path).collect())
}

pub fn inputs(core: &Core, room: &str, logs: &[Log], locate: impl Fn(&str) -> bool) -> Vec<Input> {
    let mut out = Vec::new();
    for set in room::sets(core, room, logs) {
        for f in set.files.iter().filter(|f| f.end_us.is_some()) {
            let file = format!("{}/{}", set.id, f.owner);
            let here = locate(&file);
            let mut parts = Vec::new();
            let mut complete = true;
            for capture in f.captures() {
                let segs: Vec<&Seg> = f.segs.iter().filter(|s| s.capture == capture && s.series == SERIES[0]).collect();
                let Some((start_us, end_us)) = span(&segs, f.offset_us) else { continue };
                let mut part = Part { start_us, end_us, ..Default::default() };
                if here {
                    let (v, m) = (series(core, room, f, capture, SERIES[0]), series(core, room, f, capture, SERIES[1]));
                    complete &= v.as_ref().is_some_and(|v| !v.is_empty()) && m.is_some();
                    let at = |parts: Option<Vec<PathBuf>>, stream: &str| Media { parts: parts.unwrap_or_default(), stream: stream.into() };
                    (part.picture, part.sound, part.voice) = (at(v.clone(), "v:0"), at(v, "a:0"), at(m, "a:0"));
                }
                parts.push(part);
            }
            parts.sort_by_key(|p| p.start_us);
            let Some(first) = parts.first() else { continue };
            out.push(Input { file, set: set.id.clone(), owner: f.owner.clone(), start_us: first.start_us, here: here && complete, parts });
        }
    }
    out
}

pub fn open(file: &str, me: &str, prefixes: &[String], tracks: &HashMap<String, Track>) -> bool {
    prefixes.iter().any(|p| tracks.get(&format!("{p}{file}")).is_none_or(|t| t.done.is_empty() && !matches!(t.last.get(me), Some(Status::Failed(_)))))
}

fn snapshot(core: &Core, room: &str) -> (Vec<Log>, BTreeMap<String, Config>, HashMap<String, Track>) {
    let logs = room::logs(core, room);
    let (configs, tracks) = (configs(&logs, None), tracks(&logs, None));
    (logs, configs, tracks)
}

fn plugin_dir(core: &Core, id: &str) -> PathBuf {
    core.dir.join("plugins").join(id)
}

fn work_root(core: &Core) -> PathBuf {
    core.dir.join("plugins").join("work")
}

fn online(core: &Core) -> HashSet<String> {
    core.peers.lock().unwrap().iter().filter(|p| p.alive().is_some()).map(|p| p.id.to_string()).collect()
}

fn bundled() -> Vec<(&'static str, &'static str, bool)> {
    plugins::ALL.iter().map(|p| (p.id, p.version, p.live)).collect()
}

fn present(core: &Core, id: &str, settings: &Value) -> bool {
    plugins::find(id).is_some_and(|p| plugins::missing(p, &plugin_dir(core, id), settings).is_empty()) && ffmpeg::present()
}

fn skipped(core: &Core, id: &str) -> bool {
    core.skip_plugins.lock().unwrap().contains(id)
}

fn ready(core: &Core, id: &str, settings: &Value) -> bool {
    present(core, id, settings) && !skipped(core, id)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Worker {
    pub on: bool,
    pub ready: bool,
}

pub fn workers(logs: &[Log]) -> HashMap<(String, String), Worker> {
    let mut out = HashMap::new();
    for log in logs {
        for r in log.records.iter().filter(|r| r["k"] == "worker") {
            if let Some(plugin) = r["plugin"].as_str() {
                out.insert((plugin.to_string(), log.owner.clone()), Worker { on: r["on"] != false, ready: r["ready"] == true });
            }
        }
    }
    out
}

fn publish(core: &Core, room: &str, logs: &[Log], configs: &BTreeMap<String, Config>) {
    if !core.active_room().is_some_and(|r| r.invite.is_none()) {
        return;
    }
    let me = core.me();
    let known = workers(logs);
    for p in plugins::ALL.iter() {
        let settings = configs.get(p.id).map_or_else(|| plugins::normalize(p, &Value::Null), |c| c.settings.clone());
        let now = Worker { on: !skipped(core, p.id), ready: present(core, p.id, &settings) };
        if known.get(&(p.id.to_string(), me.clone())) != Some(&now) {
            core.append_log(room, &json!({"k": "worker", "plugin": p.id, "on": now.on, "ready": now.ready, "t_us": now_us()}));
        }
    }
}

fn recording(core: &Core) -> bool {
    core.active_set.lock().unwrap().is_some()
}

fn running_json() -> Value {
    match RUNNING.lock().unwrap().as_ref() {
        Some(r) => json!({"room": r.room, "job": r.key, "plugin": r.plugin, "file": r.file, "run": r.run, "progress": r.progress}),
        None => Value::Null,
    }
}

fn emit(core: &Core) {
    let _ = core.app.emit("job", running_json());
}

pub fn worker(core: Arc<Core>) {
    let _ = std::fs::remove_dir_all(work_root(&core));
    loop {
        tick(&core);
        std::thread::sleep(CHECK);
    }
}

fn tick(core: &Arc<Core>) {
    let room = core.active_room_id();
    let (logs, configs, tracks) = snapshot(core, &room);
    publish(core, &room, &logs, &configs);
    let prefixes = current(&logs, None);
    if prefixes.is_empty() || STOP.load(Ordering::Relaxed) {
        return;
    }
    let me = core.me();
    for (key, run) in unfinished(&me, &tracks) {
        tracing::warn!("job {key} stopped twice without finishing, marking it failed");
        core.append_log(&room, &job_record("fail", &key, run, json!({"error": "Stopped twice without finishing."})));
    }
    let inputs = inputs(core, &room, &logs, |file| open(file, &me, &prefixes, &tracks));
    let first = FIRST.lock().unwrap().clone();
    let pick = next(&me, &online(core), recording(core), &bundled(), |id, s| ready(core, id, s), &configs, &tracks, &chosen(&logs, None), &inputs, &first);
    let Some(pick) = pick else { return };
    let (Some(plugin), Some(config)) = (plugins::find(&pick.plugin), configs.get(&pick.plugin)) else { return };
    WORKING.store(true, Ordering::Relaxed);
    work(core, &room, plugin, config, &pick.key, &inputs[pick.input]);
    WORKING.store(false, Ordering::Relaxed);
}

fn work(core: &Arc<Core>, room: &str, plugin: &'static Plugin, config: &Config, key: &str, input: &Input) {
    let run = now_us();
    let about = json!({"plugin": plugin.id, "version": plugin.version, "settings": config.settings, "file": input.file});
    core.append_log(room, &job_record("claim", key, run, about.clone()));
    std::thread::sleep(CLAIM_SETTLE);
    if yields(tracks(&room::logs(core, room), None).get(key), &core.me(), run, &online(core)) {
        core.append_log(room, &job_record("yield", key, run, about));
        return;
    }
    if STOP.load(Ordering::Relaxed) {
        core.append_log(room, &job_record("stop", key, run, about));
        return;
    }
    CANCEL.store(false, Ordering::Relaxed);
    tracing::info!("job {key} started on {}", input.file);
    *RUNNING.lock().unwrap() = Some(Running { room: room.into(), key: key.into(), plugin: plugin.id.into(), file: input.file.clone(), run, progress: 0.0 });
    emit(core);
    let work = work_root(core).join(run.to_string());
    let outcome = run_parts(core, room, plugin, config, key, input, &work);
    let about = json!({"plugin": plugin.id, "version": plugin.version, "file": input.file});
    match outcome {
        Some(Ok((found, tags))) => {
            tracing::info!("job {key} done with {} events", found.len());
            finish(core, room, plugin, key, run, input, &found, &tags)
        }
        Some(Err(error)) => {
            tracing::warn!("job {key} failed: {error}");
            let mut record = job_record("fail", key, run, about);
            record["error"] = json!(error);
            core.append_log(room, &record);
        }
        None => {
            if RUNNING.lock().unwrap().is_some() {
                tracing::info!("job {key} stopped");
                core.append_log(room, &job_record("stop", key, run, about));
            }
        }
    }
    let _ = std::fs::remove_dir_all(work);
    *RUNNING.lock().unwrap() = None;
    emit(core);
}

type Results = (Vec<(i64, Found)>, BTreeMap<String, String>);

fn run_parts(core: &Arc<Core>, room: &str, plugin: &'static Plugin, config: &Config, key: &str, input: &Input, work: &std::path::Path) -> Option<Result<Results, String>> {
    let mut found = Vec::new();
    let mut tags = BTreeMap::new();
    let count = input.parts.len().max(1) as f64;
    for (i, part) in input.parts.iter().enumerate() {
        let job = plugins::Job {
            plugin: plugin.id.into(),
            settings: config.settings.clone(),
            set: json!({"id": input.set, "start_us": part.start_us}),
            file: json!({"owner": input.owner, "name": room::display_name(core, &input.owner)}),
            duration_s: (part.end_us - part.start_us) as f64 / 1e6,
            picture: part.picture.clone(),
            sound: part.sound.clone(),
            voice: part.voice.clone(),
            ffmpeg: ffmpeg::path(),
            dir: plugin_dir(core, plugin.id),
            work: work.join(i.to_string()),
        };
        match watch(core, room, plugin, key, &job, (i as f64 / count, 1.0 / count))? {
            Ok(output) => {
                found.extend(output.events.into_iter().map(|e| (part.start_us, e)));
                tags.extend(output.tags);
            }
            Err(e) => return Some(Err(e)),
        }
    }
    Some(Ok((found, tags)))
}

fn stale(core: &Core, room: &str, plugin: &Plugin, key: &str) -> bool {
    let logs = room::logs(core, room);
    core.active_room_id() != room || skipped(core, plugin.id) || (recording(core) && !plugin.live) || !current(&logs, None).iter().any(|p| key.starts_with(p.as_str()))
}

fn watch(core: &Core, room: &str, plugin: &Plugin, key: &str, job: &plugins::Job, share: (f64, f64)) -> Option<Result<Output, String>> {
    let mut process = match plugins::spawn(job) {
        Ok(c) => c,
        Err(e) => return Some(Err(e.to_string())),
    };
    let rx = plugins::replies(process.child.stdout.take().unwrap());
    let (mut checked, mut emitted) = (Instant::now(), Instant::now());
    loop {
        if checked.elapsed() >= CHECK {
            checked = Instant::now();
            if CANCEL.swap(false, Ordering::Relaxed) {
                process.kill();
                return Some(Err("Cancelled".into()));
            }
            if STOP.load(Ordering::Relaxed) || stale(core, room, plugin, key) {
                process.kill();
                return None;
            }
        }
        match rx.recv_timeout(CHECK) {
            Ok(Reply::Progress(p)) => {
                if let Some(r) = RUNNING.lock().unwrap().as_mut() {
                    r.progress = share.0 + p * share.1;
                }
                if emitted.elapsed() >= EMIT_EVERY {
                    emitted = Instant::now();
                    emit(core);
                }
            }
            Ok(Reply::Done(output)) => {
                let _ = process.child.wait();
                return (!stale(core, room, plugin, key)).then_some(Ok(output));
            }
            Ok(Reply::Failed(e)) => {
                let _ = process.child.wait();
                return Some(Err(e));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                let _ = process.child.wait();
                return Some(Err("The plugin stopped without a result.".into()));
            }
        }
    }
}

fn finish(core: &Arc<Core>, room: &str, plugin: &Plugin, key: &str, run: i64, input: &Input, found: &[(i64, Found)], tags: &BTreeMap<String, String>) {
    let mut id = 0;
    for (start_us, one) in found {
        id = now_us().max(id + 1);
        core.append_log(room, &event_record(plugin, key, run, input, *start_us, one, id));
    }
    core.append_log(room, &job_record("done", key, run, json!({"plugin": plugin.id, "version": plugin.version, "file": input.file, "events": found.len(), "tags": tags})));
    party::send(core);
}

fn plugins_json(core: &Core) -> Vec<Value> {
    let room = core.active_room_id();
    let logs = room::logs(core, &room);
    let configs = configs(&logs, None);
    let known = workers(&logs);
    let members: Vec<String> = core.rooms.lock().unwrap().list.iter().find(|r| r.id == room).map(|r| r.members.iter().map(|m| m.to_string()).collect()).unwrap_or_default();
    let me = core.me();
    plugins::ALL
        .iter()
        .map(|p| {
            let config = configs.get(p.id);
            let settings = config.map_or_else(|| plugins::normalize(p, &Value::Null), |c| c.settings.clone());
            let missing = plugins::missing(p, &plugin_dir(core, p.id), &settings);
            let mut v = plugins::manifest(p);
            v["on"] = json!(config.is_some_and(|c| c.on));
            v["room_version"] = json!(config.map(|c| c.version.clone()));
            v["settings"] = settings;
            v["download_bytes"] = json!(missing.iter().map(|d| d.size).sum::<u64>());
            v["ready"] = json!(missing.is_empty());
            v["download"] = crate::download::progress(p.id);
            v["process_here"] = json!(!skipped(core, p.id));
            let others = members.iter().filter(|m| **m != me);
            v["workers"] = std::iter::once(&me).chain(others).map(|m| {
                let w = known.get(&(p.id.to_string(), m.clone()));
                json!({"id": m, "on": w.is_none_or(|w| w.on), "ready": w.map(|w| w.ready)})
            }).collect();
            v
        })
        .collect()
}

#[tauri::command]
pub async fn room_plugins(core: State<'_, Arc<Core>>) -> Result<Vec<Value>, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || plugins_json(&core)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_room_plugin(core: State<Arc<Core>>, id: String, on: bool, settings: Value) -> Result<(), String> {
    let plugin = plugins::find(&id).ok_or("Unknown plugin.")?;
    let record = json!({"k": "plugin", "plugin": plugin.id, "on": on, "version": plugin.version, "settings": plugins::normalize(plugin, &settings), "t_us": now_us()});
    core.append_log(&core.active_room_id(), &record);
    Ok(())
}

#[tauri::command]
pub fn set_process_plugin(core: State<Arc<Core>>, id: String, on: bool) -> Result<(), String> {
    let plugin = plugins::find(&id).ok_or("Unknown plugin.")?;
    {
        let mut skip = core.skip_plugins.lock().unwrap();
        if on { skip.remove(plugin.id) } else { skip.insert(plugin.id.to_string()) };
    }
    core.save_settings();
    let room = core.active_room_id();
    let logs = room::logs(&core, &room);
    publish(&core, &room, &logs, &configs(&logs, None));
    Ok(())
}

#[tauri::command]
pub async fn prepare_plugin(core: State<'_, Arc<Core>>, id: String) -> Result<(), String> {
    let plugin = plugins::find(&id).ok_or("Unknown plugin.")?;
    let settings = configs(&room::logs(&core, &core.active_room_id()), None).get(plugin.id).map_or_else(|| plugins::normalize(plugin, &Value::Null), |c| c.settings.clone());
    let dir = plugin_dir(&core, plugin.id);
    tauri::async_runtime::spawn_blocking(move || plugins::fetch_group(plugin.id, &dir, &plugins::missing(plugin, &dir, &settings))).await.map_err(|e| e.to_string())?
}

fn jobs_json(core: &Core) -> Value {
    let room = core.active_room_id();
    let (logs, configs, tracks) = snapshot(core, &room);
    let (me, online) = (core.me(), online(core));
    let on: Vec<(&String, &Config, String)> = configs.iter().filter(|(_, c)| c.on).map(|(id, c)| (id, c, prefix(id, c))).collect();
    let prefixes: Vec<String> = on.iter().map(|(_, _, p)| p.clone()).collect();
    let inputs = if on.is_empty() { Vec::new() } else { inputs(core, &room, &logs, |file| open(file, &me, &prefixes, &tracks)) };
    let chosen = chosen(&logs, None);
    let mut list = Vec::new();
    for (id, config, prefix) in &on {
        for input in inputs.iter().filter(|i| counts(id, &i.file, &chosen)) {
            let key = format!("{prefix}{}", input.file);
            let (state, by, error) = state(tracks.get(&key), &me, &online);
            let next = *FIRST.lock().unwrap() == key;
            list.push(json!({"job": key, "plugin": id, "version": config.version, "settings": config.settings, "set": input.set, "owner": input.owner, "whole": plugins::find(id).is_some_and(|p| p.per_set), "here": input.here, "state": state, "by": by, "error": error, "next": next}));
        }
    }
    json!({"jobs": list, "running": running_json(), "tags": tag_names(&logs, core.party.lock().unwrap().caps().as_ref())})
}

#[tauri::command]
pub async fn jobs(core: State<'_, Arc<Core>>) -> Result<Value, String> {
    let core = core.inner().clone();
    tauri::async_runtime::spawn_blocking(move || jobs_json(&core)).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn retry_job(core: State<Arc<Core>>, job: String) {
    core.append_log(&core.active_room_id(), &job_record("retry", &job, 0, Value::Null));
}

#[tauri::command]
pub fn run_job_next(job: String) {
    *FIRST.lock().unwrap() = job;
}

#[tauri::command]
pub fn cancel_job() {
    if RUNNING.lock().unwrap().is_some() {
        CANCEL.store(true, Ordering::Relaxed);
    }
}

pub fn stop_now(core: &Core) {
    STOP.store(true, Ordering::Relaxed);
    let Some(r) = RUNNING.lock().unwrap().take() else { return };
    tracing::info!("job {} stopped", r.key);
    core.append_log(&r.room, &job_record("stop", &r.key, r.run, json!({"plugin": r.plugin, "file": r.file})));
}

pub fn stop_and_wait(core: &Core) {
    stop_now(core);
    let started = Instant::now();
    while WORKING.load(Ordering::Relaxed) && started.elapsed() < STOP_WAIT {
        std::thread::sleep(EMIT_EVERY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(owner: &str, offset_us: i64, records: Vec<Value>) -> Log {
        Log { owner: owner.into(), offset_us, records: Arc::new(records) }
    }

    fn config_record(t: i64, on: bool, model: &str) -> Value {
        json!({"k": "plugin", "plugin": "transcribe", "on": on, "version": "1.0.0", "settings": {"model": model}, "t_us": t})
    }

    fn input(set: &str, owner: &str, start: i64, here: bool) -> Input {
        Input { file: format!("{set}/{owner}"), set: set.into(), owner: owner.into(), start_us: start, here, parts: vec![Part { start_us: start, end_us: start + 1000, ..Default::default() }] }
    }

    fn on() -> BTreeMap<String, Config> {
        BTreeMap::from([("transcribe".to_string(), Config { on: true, version: "1.0.0".into(), settings: json!({"model": "small"}) })])
    }

    fn key(file: &str) -> String {
        format!("{}{file}", prefix("transcribe", &on()["transcribe"]))
    }

    const BUNDLED: [(&str, &str, bool); 1] = [("transcribe", "1.0.0", false)];

    fn pick(me: &str, online: &[&str], recording: bool, logs: &[Log], inputs: &[Input]) -> Option<String> {
        let online: HashSet<String> = online.iter().map(|s| s.to_string()).collect();
        next(me, &online, recording, &BUNDLED, |_, _| true, &configs(logs, None), &tracks(logs, None), &HashMap::new(), inputs, "").map(|p| inputs[p.input].file.clone())
    }

    fn base_logs(extra_a: Vec<Value>, extra_b: Vec<Value>) -> Vec<Log> {
        let mut a = vec![config_record(1, true, "small")];
        a.extend(extra_a);
        vec![log("a", 0, a), log("b", 0, extra_b)]
    }

    #[test]
    fn the_latest_config_wins_on_the_shared_clock() {
        let logs = vec![log("a", 0, vec![config_record(100, true, "small")]), log("b", 50, vec![config_record(120, false, "base")])];
        let c = configs(&logs, None);
        assert!(c["transcribe"].on);
        let logs = vec![log("a", 0, vec![config_record(100, true, "small")]), log("b", -50, vec![config_record(120, false, "base")])];
        assert!(!configs(&logs, None)["transcribe"].on);
    }

    #[test]
    fn the_later_of_two_claims_yields() {
        let claim = |run: i64| json!({"k": "job", "state": "claim", "job": "j", "run": run});
        let logs = vec![log("a", 0, vec![claim(10)]), log("b", 0, vec![claim(20)])];
        let t = tracks(&logs, None);
        let online: HashSet<String> = ["a".to_string(), "b".to_string()].into();
        assert!(yields(t.get("j"), "b", 20, &online));
        assert!(!yields(t.get("j"), "a", 10, &online));
        assert!(!yields(t.get("j"), "b", 20, &HashSet::new()));
        let tie = tracks(&[log("a", 0, vec![claim(10)]), log("b", 0, vec![claim(10)])], None);
        assert!(yields(tie.get("j"), "b", 10, &online) && !yields(tie.get("j"), "a", 10, &online));
        let yielded = tracks(&[log("a", 0, vec![claim(10)]), log("b", 0, vec![claim(20), json!({"k": "job", "state": "yield", "job": "j", "run": 20})])], None);
        assert_eq!(yielded["j"].last["b"], Status::Retry);
        let finished = tracks(&[log("a", 0, vec![claim(10), json!({"k": "job", "state": "done", "job": "j", "run": 10})]), log("b", 0, vec![claim(20)])], None);
        assert!(yields(finished.get("j"), "b", 20, &HashSet::new()));
    }

    #[test]
    fn keys_change_with_version_and_settings() {
        let a = Config { on: true, version: "1.0.0".into(), settings: json!({"model": "small"}) };
        let b = Config { settings: json!({"model": "base"}), ..a.clone() };
        let c = Config { version: "1.1.0".into(), ..a.clone() };
        assert_ne!(prefix("transcribe", &a), prefix("transcribe", &b));
        assert_ne!(prefix("transcribe", &a), prefix("transcribe", &c));
        assert_eq!(prefix("transcribe", &a), prefix("transcribe", &a.clone()));
    }

    #[test]
    fn a_job_asked_for_runs_next() {
        let logs = base_logs(vec![], vec![]);
        let inputs = [input("s1", "b", 100, true), input("s2", "a", 200, true)];
        let online: HashSet<String> = HashSet::new();
        let first = key(&inputs[0].file);
        let pick = next("a", &online, false, &BUNDLED, |_, _| true, &configs(&logs, None), &tracks(&logs, None), &HashMap::new(), &inputs, &first).unwrap();
        assert_eq!(inputs[pick.input].file, inputs[0].file);
        assert_eq!(pick_file("a", &logs, &inputs), inputs[1].file);
    }

    fn pick_file(me: &str, logs: &[Log], inputs: &[Input]) -> String {
        pick(me, &[], false, logs, inputs).unwrap()
    }

    #[test]
    fn own_recordings_come_first_then_the_newest() {
        let logs = base_logs(vec![], vec![]);
        let inputs = [input("s1", "b", 10, true), input("s2", "b", 20, true), input("s1", "a", 10, true)];
        assert_eq!(pick("a", &[], false, &logs, &inputs).as_deref(), Some("s1/a"));
        assert_eq!(pick("a", &[], false, &logs, &inputs[..2]).as_deref(), Some("s2/b"));
        assert_eq!(pick("a", &[], false, &logs, &[input("s1", "b", 10, false)]), None);
    }

    #[test]
    fn nothing_runs_while_recording_unless_live_or_when_off_or_mismatched() {
        let inputs = [input("s1", "a", 10, true)];
        assert_eq!(pick("a", &[], true, &base_logs(vec![], vec![]), &inputs), None);
        let off = vec![log("a", 0, vec![config_record(1, true, "small"), config_record(2, false, "small")])];
        assert_eq!(pick("a", &[], false, &off, &inputs), None);
        let mut newer = config_record(3, true, "small");
        newer["version"] = json!("2.0.0");
        assert_eq!(pick("a", &[], false, &[log("a", 0, vec![newer])], &inputs), None);
        let online = HashSet::new();
        let live = [("transcribe", "1.0.0", true)];
        let logs = base_logs(vec![], vec![]);
        assert!(next("a", &online, true, &live, |_, _| true, &configs(&logs, None), &tracks(&logs, None), &HashMap::new(), &inputs, "").is_some());
        assert!(next("a", &online, false, &live, |_, _| false, &configs(&logs, None), &tracks(&logs, None), &HashMap::new(), &inputs, "").is_none());
    }

    #[test]
    fn claims_by_online_members_are_left_and_stale_ones_taken() {
        let claim = job_record("claim", &key("s1/b"), 7, Value::Null);
        let logs = base_logs(vec![], vec![claim]);
        let inputs = [input("s1", "b", 10, true)];
        assert_eq!(pick("a", &["b"], false, &logs, &inputs), None);
        assert_eq!(pick("a", &[], false, &logs, &inputs).as_deref(), Some("s1/b"));
    }

    #[test]
    fn interrupted_own_claims_rerun_first() {
        let mine = job_record("claim", &key("s0/b"), 5, Value::Null);
        let theirs = job_record("claim", &key("s0/b"), 6, Value::Null);
        let logs = base_logs(vec![mine], vec![theirs]);
        let inputs = [input("s1", "a", 10, true), input("s0", "b", 1, true)];
        assert_eq!(pick("a", &["b"], false, &logs, &inputs).as_deref(), Some("s0/b"));
    }

    #[test]
    fn done_jobs_are_skipped_and_own_failures_wait_for_a_retry() {
        let inputs = [input("s1", "a", 10, true)];
        let done = base_logs(vec![], vec![job_record("done", &key("s1/a"), 3, Value::Null)]);
        assert_eq!(pick("a", &[], false, &done, &inputs), None);
        let failed = base_logs(vec![job_record("claim", &key("s1/a"), 3, Value::Null), job_record("fail", &key("s1/a"), 3, json!({"error": "x"}))], vec![]);
        assert_eq!(pick("a", &[], false, &failed, &inputs), None);
        assert_eq!(pick("b", &[], false, &failed, &inputs).as_deref(), Some("s1/a"));
        let mut retried = failed;
        Arc::make_mut(&mut retried[0].records).push(job_record("retry", &key("s1/a"), 0, Value::Null));
        assert_eq!(pick("a", &[], false, &retried, &inputs).as_deref(), Some("s1/a"));
    }

    #[test]
    fn a_job_stopped_twice_without_finishing_fails_and_a_stop_on_purpose_does_not_count() {
        let claim = |run| job_record("claim", &key("s1/a"), run, Value::Null);
        let once = base_logs(vec![claim(1)], vec![]);
        assert!(unfinished("a", &tracks(&once, None)).is_empty());
        let twice = base_logs(vec![claim(1), claim(2)], vec![]);
        assert_eq!(unfinished("a", &tracks(&twice, None)), vec![(key("s1/a"), 2)]);
        assert!(unfinished("b", &tracks(&twice, None)).is_empty());
        let stopped = base_logs(vec![claim(1), job_record("stop", &key("s1/a"), 1, Value::Null), claim(2)], vec![]);
        assert!(unfinished("a", &tracks(&stopped, None)).is_empty());
        let inputs = [input("s1", "a", 10, true)];
        assert_eq!(pick("a", &[], false, &stopped, &inputs).as_deref(), Some("s1/a"));
    }

    #[test]
    fn a_plugin_for_the_whole_set_reads_the_longest_file_once() {
        let seg = |set: &str, n: u32, capture: &str| json!({"k": "seg", "series": "v", "set": set, "n": n, "capture": capture});
        let logs = vec![log("a", 0, vec![seg("s1", 1, "1"), seg("s1", 2, "1"), seg("s2", 1, "3")]), log("b", 0, vec![seg("s1", 1, "2"), seg("s1", 2, "2"), seg("s1", 3, "2"), seg("s2", 1, "4")])];
        let chosen = chosen(&logs, None);
        assert_eq!(chosen["s1"], "s1/b");
        assert_eq!(chosen["s2"], "s2/a");
        assert!(counts("apex-matches", "s1/b", &chosen) && !counts("apex-matches", "s1/a", &chosen));
        assert!(counts("transcribe", "s1/a", &chosen));
    }

    #[test]
    fn restarted_captures_of_one_person_are_one_file_and_old_keys_are_ignored() {
        let seg = |n: u32, capture: &str| json!({"k": "seg", "series": "v", "set": "s1", "n": n, "capture": capture});
        let logs = vec![log("a", 0, vec![seg(1, "1"), seg(2, "1"), seg(1, "2")]), log("b", 0, vec![seg(1, "3"), seg(2, "3")])];
        assert_eq!(chosen(&logs, None)["s1"], "s1/a");
        assert!(!counts("transcribe", "s1/a/1", &HashMap::new()));
        assert!(!counts("transcribe", "s1", &HashMap::new()));
    }

    fn plugin_event(run: i64, id: i64) -> Value {
        json!({"k": "event", "id": id, "kind": "speech", "plugin": {"id": "transcribe"}, "run": run})
    }

    #[test]
    fn a_job_done_twice_is_shown_once_and_only_for_the_current_config() {
        let k = key("s1/a");
        let a = vec![plugin_event(1, 10), job_record("done", &k, 1, json!({"tags": {"x": "X"}})), plugin_event(2, 11)];
        let b = vec![plugin_event(4, 20), job_record("done", &k, 4, Value::Null)];
        let logs = base_logs(a, b);
        assert_eq!(shown(&logs, None), HashSet::from([("a".to_string(), 1)]));
        assert_eq!(tag_names(&logs, None), BTreeMap::from([("x".to_string(), "X".to_string())]));
        let mut events = vec![
            json!({"author": "a", "run": 1, "plugin": {"id": "transcribe"}}),
            json!({"author": "a", "run": 2, "plugin": {"id": "transcribe"}}),
            json!({"author": "b", "run": 4, "plugin": {"id": "transcribe"}}),
            json!({"author": "b", "plugin": null}),
        ];
        keep_shown(&mut events, &logs, None);
        assert_eq!(events.len(), 2);
        let mut changed = logs;
        Arc::make_mut(&mut changed[1].records).push(config_record(9, true, "base"));
        assert!(shown(&changed, None).is_empty());
    }

    #[test]
    fn results_wait_for_the_done_record_within_caps() {
        let k = key("s1/a");
        let logs = base_logs(vec![plugin_event(1, 10), job_record("done", &k, 1, Value::Null)], vec![]);
        let caps = HashMap::from([("a".to_string(), 2u64)]);
        assert!(shown(&logs, Some(&caps)).is_empty());
        let caps = HashMap::from([("a".to_string(), 3u64)]);
        assert_eq!(shown(&logs, Some(&caps)).len(), 1);
    }

    #[test]
    fn each_member_publishes_whether_it_processes_a_plugin_and_has_it() {
        let worker = |plugin: &str, on: bool, ready: bool| json!({"k": "worker", "plugin": plugin, "on": on, "ready": ready});
        let logs = vec![log("a", 0, vec![worker("transcribe", true, false), worker("transcribe", false, true)]), log("b", 0, vec![worker("apex-matches", true, true)])];
        let w = workers(&logs);
        assert_eq!(w[&("transcribe".to_string(), "a".to_string())], Worker { on: false, ready: true });
        assert_eq!(w[&("apex-matches".to_string(), "b".to_string())], Worker { on: true, ready: true });
        assert!(!w.contains_key(&("transcribe".to_string(), "b".to_string())));
    }

    #[test]
    fn states_for_the_jobs_view() {
        let online = HashSet::from(["b".to_string()]);
        assert_eq!(state(None, "a", &online).0, "waiting");
        let logs = base_logs(vec![], vec![job_record("claim", &key("f"), 1, Value::Null)]);
        let t = tracks(&logs, None);
        assert_eq!(state(t.get(&key("f")), "a", &online), ("claimed".into(), json!("b"), Value::Null));
        assert_eq!(state(t.get(&key("f")), "a", &HashSet::new()).0, "waiting");
        let logs = base_logs(vec![], vec![job_record("fail", &key("f"), 1, json!({"error": "no"}))]);
        assert_eq!(state(tracks(&logs, None).get(&key("f")), "a", &online), ("failed".into(), json!("b"), json!("no")));
    }

    #[test]
    fn results_land_on_the_runners_clock() {
        let i = input("s1", "b", 1_000_000, true);
        let at = Found { kind: "match_end".into(), at_s: Some(2.5), ..Default::default() };
        assert_eq!(target(&at, &i, i.start_us), json!({"t_us": 3_500_000, "set": "s1", "file": "b"}));
        assert_eq!(target(&at, &i, 9_000_000), json!({"t_us": 11_500_000, "set": "s1", "file": "b"}));
        let range = Found { kind: "speech".into(), from_s: Some(1.0), to_s: Some(0.5), ..Default::default() };
        assert_eq!(target(&range, &i, i.start_us), json!({"from_us": 2_000_000, "to_us": 2_000_000, "set": "s1", "file": "b"}));
        let r = event_record(plugins::find("transcribe").unwrap(), "k", 7, &i, i.start_us, &range, 42);
        assert_eq!((r["k"].clone(), r["run"].clone(), r["plugin"]["name"].clone()), (json!("event"), json!(7), json!("Transcription")));
    }

    #[test]
    fn a_capture_spans_its_segments_on_the_local_clock() {
        let seg = |n, t_us| Seg { capture: "1".into(), series: "v".into(), n, t_us, d_us: 4, size: 1, sha256: String::new() };
        let (a, b) = (seg(1, 100), seg(2, 104));
        assert_eq!(span(&[&a, &b], 10), Some((90, 98)));
        assert_eq!(span(&[], 0), None);
    }

    #[test]
    fn room_events_hide_unfinished_plugin_runs() {
        let k = key("s1/a");
        let mut e = plugin_event(1, 10);
        e["target"] = json!({"t_us": 50});
        let logs = base_logs(vec![e.clone()], vec![]);
        assert!(room::events(&logs, None).is_empty());
        let logs = base_logs(vec![e, job_record("done", &k, 1, Value::Null)], vec![]);
        assert_eq!(room::events(&logs, None).len(), 1);
    }
}

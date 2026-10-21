use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::Emitter;

use crate::plugins::{self, Found, Media, Output, Plugin, Reply};
use crate::room::{self, File, Log, Seg, Set, SERIES};
use crate::ops::arg;
use crate::{data, ffmpeg, hex, now_us, party, stream, Core};

const CHECK: Duration = Duration::from_secs(2);
const EMIT_EVERY: Duration = Duration::from_millis(250);
const SETTINGS_HASH_BYTES: usize = 6;
const STOP_WAIT: Duration = Duration::from_secs(10);
pub const UNFINISHED_MAX: u32 = 2;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub on: bool,
    pub version: String,
    pub settings: Value,
    pub runner: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Claimed(i64),
    Done(i64),
    Failed(String),
    Retry,
}

#[derive(Clone, Debug)]
pub struct Done {
    pub at: i64,
    pub owner: String,
    pub run: i64,
    pub tags: Value,
}

#[derive(Clone, Debug)]
pub struct Latest {
    pub at: (i64, String),
    pub key: String,
    pub status: Status,
}

#[derive(Default, Debug)]
pub struct Track {
    pub done: Vec<Done>,
    pub latest: Option<Latest>,
    pub unfinished: BTreeMap<String, u32>,
}

pub type Tracks = HashMap<(String, String), Track>;

impl Track {
    fn winner(&self) -> Option<&Done> {
        self.done.iter().max_by(|a, b| a.at.cmp(&b.at).then_with(|| (&b.owner, b.run).cmp(&(&a.owner, a.run))))
    }

    fn open_for(&self, me: &str) -> Option<bool> {
        match self.latest.as_ref() {
            None => Some(false),
            Some(Latest { status: Status::Done(_) | Status::Failed(_), .. }) => None,
            Some(Latest { at: (_, owner), status: Status::Claimed(_), .. }) => Some(owner == me),
            Some(_) => Some(false),
        }
    }
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
static CANCEL: AtomicBool = AtomicBool::new(false);
static STOP: AtomicBool = AtomicBool::new(false);
static WORK: Mutex<Option<String>> = Mutex::new(None);

fn capped<'a>(logs: &'a [Log], caps: Option<&'a HashMap<String, u64>>) -> impl Iterator<Item = (&'a Log, &'a [Value])> {
    logs.iter().map(move |log| {
        let cap = caps.map_or(usize::MAX, |c| c.get(&log.owner).copied().unwrap_or(0) as usize);
        (log, &log.records[..cap.min(log.records.len())])
    })
}

fn newest(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> BTreeMap<String, ((i64, String), Config)> {
    let mut latest: BTreeMap<String, ((i64, String), Config)> = BTreeMap::new();
    for (log, records) in capped(logs, caps) {
        for r in records.iter().filter(|r| r["k"] == "plugin") {
            let (Some(id), Some(t)) = (r["plugin"].as_str(), r["t_us"].as_i64()) else { continue };
            let at = (t - log.offset_us, log.owner.clone());
            if latest.get(id).is_none_or(|(old, _)| at > *old) {
                let runner = r["runner"].as_str().filter(|m| !m.is_empty()).unwrap_or(&log.owner).to_string();
                let config = Config { on: r["on"] == true, version: r["version"].as_str().unwrap_or_default().to_string(), settings: r["settings"].clone(), runner: Some(runner) };
                latest.insert(id.to_string(), (at, config));
            }
        }
    }
    latest
}

pub fn configs(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> BTreeMap<String, Config> {
    newest(logs, caps).into_iter().map(|(id, (_, c))| (id, c)).collect()
}

pub fn change_time(now: i64, logs: &[Log], id: &str) -> i64 {
    newest(logs, None).get(id).map_or(now, |((at, _), _)| now.max(at + 1))
}

pub fn prefix(plugin: &str, config: &Config) -> String {
    let sorted: BTreeMap<&String, &Value> = config.settings.as_object().into_iter().flatten().collect();
    let digest = Sha256::digest(serde_json::to_string(&sorted).unwrap_or_default().as_bytes());
    format!("{plugin}@{}/{}/", config.version, hex(&digest[..SETTINGS_HASH_BYTES]))
}

fn split_key(key: &str) -> Option<(String, String)> {
    let (head, rest) = key.split_once('/')?;
    let file = rest.split_once('/')?.1;
    Some((head.split('@').next()?.to_string(), file.to_string()))
}

pub fn tracks(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> Tracks {
    let mut out: Tracks = HashMap::new();
    for (log, records) in capped(logs, caps) {
        for r in records.iter().filter(|r| r["k"] == "job") {
            let Some(key) = r["job"].as_str() else { continue };
            let Some(id) = split_key(key) else { continue };
            let run = r["run"].as_i64().unwrap_or(0);
            let status = match r["state"].as_str() {
                Some("claim") => Status::Claimed(run),
                Some("done") => Status::Done(run),
                Some("fail") => Status::Failed(r["error"].as_str().unwrap_or_default().to_string()),
                Some("retry" | "yield" | "stop") => Status::Retry,
                _ => continue,
            };
            let at = (r["t_us"].as_i64().unwrap_or(0) - log.offset_us, log.owner.clone());
            let track = out.entry(id).or_default();
            let unfinished = track.unfinished.entry(log.owner.clone()).or_default();
            *unfinished = if matches!(status, Status::Claimed(_)) { *unfinished + 1 } else { 0 };
            if status == Status::Done(run) {
                track.done.push(Done { at: at.0, owner: log.owner.clone(), run, tags: r["tags"].clone() });
            }
            if track.latest.as_ref().is_none_or(|l| at >= l.at) {
                track.latest = Some(Latest { at, key: key.to_string(), status });
            }
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

fn winners(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> Vec<((String, i64), Value)> {
    let on: HashSet<String> = configs(logs, caps).into_iter().filter(|(_, c)| c.on).map(|(id, _)| id).collect();
    let chosen = chosen(logs, caps);
    tracks(logs, caps)
        .into_iter()
        .filter(|((plugin, file), _)| on.contains(plugin) && counts(plugin, file, &chosen))
        .filter_map(|(_, t)| t.winner().map(|w| ((w.owner.clone(), w.run), w.tags.clone())))
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

#[allow(clippy::too_many_arguments)]
pub fn next(
    me: &str,
    recording: bool,
    bundled: &[(&str, &str, bool)],
    ready: impl Fn(&str, &Value) -> bool,
    configs: &BTreeMap<String, Config>,
    tracks: &Tracks,
    chosen: &HashMap<String, String>,
    inputs: &[Input],
) -> Option<Pick> {
    let mut picks = Vec::new();
    for (order, (id, config)) in configs.iter().enumerate() {
        let paused = recording && !bundled.iter().any(|(b, _, live)| *b == id.as_str() && *live);
        if paused || !runs_here(me, bundled, &ready, id, config) {
            continue;
        }
        let prefix = prefix(id, config);
        for (i, input) in inputs.iter().enumerate().filter(|(_, input)| input.here && counts(id, &input.file, chosen)) {
            let Some(resumed) = tracks.get(&(id.clone(), input.file.clone())).map_or(Some(false), |t| t.open_for(me)) else { continue };
            picks.push(((!resumed, Reverse(input.start_us), order), Pick { plugin: id.clone(), key: format!("{prefix}{}", input.file), input: i }));
        }
    }
    picks.into_iter().min_by(|a, b| a.0.cmp(&b.0)).map(|(_, p)| p)
}

fn runs_here(me: &str, bundled: &[(&str, &str, bool)], ready: impl Fn(&str, &Value) -> bool, id: &str, config: &Config) -> bool {
    config.on && bundled.iter().any(|(b, version, _)| *b == id && *version == config.version) && config.runner.as_deref() == Some(me) && ready(id, &config.settings)
}

pub fn runner_wants(me: &str, configs: &BTreeMap<String, Config>) -> bool {
    configs.values().any(|c| c.on && c.runner.as_deref() == Some(me))
}

pub fn abandoned(me: &str, tracks: &Tracks) -> Vec<Value> {
    tracks
        .values()
        .filter_map(|t| match t.latest.as_ref() {
            Some(Latest { at: (_, owner), key, status: Status::Claimed(run) }) if owner == me && t.unfinished.get(me).copied().unwrap_or(0) >= UNFINISHED_MAX => {
                tracing::warn!("job {key} stopped twice without finishing, marking it failed");
                Some(job_record("fail", key, *run, json!({"error": "Stopped twice without finishing."})))
            }
            _ => None,
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Holder {
    pub id: String,
    pub online: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Wait {
    Queue,
    Receiving,
    Member(String),
    NoRunner,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Done,
    Running,
    Failed(String),
    Missing,
    Left(Wait),
}

pub fn phase(track: Option<&Track>, runner: Option<&str>, me: &str, online: &HashSet<String>, holders: &[Holder]) -> Phase {
    let alive = |who: &str| who == me || online.contains(who);
    match (track.and_then(|t| t.latest.as_ref()), runner) {
        (Some(Latest { status: Status::Done(_), .. }), _) => Phase::Done,
        (Some(Latest { status: Status::Failed(e), .. }), _) => Phase::Failed(e.clone()),
        (Some(Latest { at: (_, owner), status: Status::Claimed(_), .. }), Some(runner)) if owner == runner && alive(owner) => Phase::Running,
        (_, None) => Phase::Left(Wait::NoRunner),
        (_, Some(runner)) if !alive(runner) => Phase::Left(Wait::Member(runner.to_string())),
        (_, Some(runner)) if holders.iter().any(|h| h.id == runner) => Phase::Left(Wait::Queue),
        _ if holders.iter().any(|h| h.online) => Phase::Left(Wait::Receiving),
        _ => Phase::Missing,
    }
}

fn rank(phase: &Phase) -> u8 {
    match phase {
        Phase::Done => 0,
        Phase::Left(_) => 1,
        Phase::Running => 2,
        Phase::Failed(_) | Phase::Missing => 3,
    }
}

fn fold(phases: Vec<Phase>) -> Phase {
    phases.into_iter().reduce(|a, b| if rank(&b) > rank(&a) { b } else { a }).unwrap_or(Phase::Done)
}

pub fn analysis(
    me: &str,
    online: &HashSet<String>,
    configs: &BTreeMap<String, Config>,
    tracks: &Tracks,
    chosen: &HashMap<String, String>,
    sets: &[Set],
    holders: impl Fn(&Set, &File) -> Vec<Holder>,
) -> HashMap<String, Vec<(String, Phase)>> {
    let mut out: HashMap<String, Vec<(String, Phase)>> = HashMap::new();
    for set in sets {
        let finished: Vec<&File> = set.files.iter().filter(|f| analyzable(f)).collect();
        for (id, config) in configs.iter().filter(|(_, c)| c.on) {
            let phases: Vec<Phase> = finished
                .iter()
                .filter(|f| counts(id, &format!("{}/{}", set.id, f.owner), chosen))
                .map(|f| phase(tracks.get(&(id.clone(), format!("{}/{}", set.id, f.owner))), config.runner.as_deref(), me, online, &holders(set, f)))
                .collect();
            if !phases.is_empty() {
                out.entry(set.id.clone()).or_default().push((id.clone(), fold(phases)));
            }
        }
    }
    out
}

pub fn analysis_json(phases: &[(String, Phase)], running: Option<&Running>, set: &str) -> Vec<Value> {
    phases
        .iter()
        .map(|(plugin, phase)| match phase {
            Phase::Done => json!({"plugin": plugin, "state": "done"}),
            Phase::Running => {
                let progress = running.filter(|r| r.plugin == *plugin && r.file.starts_with(&format!("{set}/"))).map(|r| r.progress);
                json!({"plugin": plugin, "state": "running", "progress": progress})
            }
            Phase::Failed(error) => json!({"plugin": plugin, "state": "failed", "error": error}),
            Phase::Missing => json!({"plugin": plugin, "state": "failed", "reason": "missing"}),
            Phase::Left(wait) => {
                let (reason, who) = match wait {
                    Wait::Queue => ("queue", None),
                    Wait::Receiving => ("receiving", None),
                    Wait::NoRunner => ("no_runner", None),
                    Wait::Member(id) => ("waiting", Some(id)),
                };
                json!({"plugin": plugin, "state": "left", "reason": reason, "waiting_for": who})
            }
        })
        .collect()
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

fn analyzable(f: &File) -> bool {
    f.end_us.is_some() && f.segs.iter().any(|s| s.series == SERIES[0])
}

pub fn inputs(core: &Core, room: &str, logs: &[Log], locate: impl Fn(&str) -> bool) -> Vec<Input> {
    let mut out = Vec::new();
    for set in room::sets(core, room, logs) {
        for f in set.files.iter().filter(|f| analyzable(f)) {
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

pub fn open(file: &str, me: &str, configs: &BTreeMap<String, Config>, tracks: &Tracks, chosen: &HashMap<String, String>) -> bool {
    configs.iter().any(|(id, c)| c.on && c.runner.as_deref() == Some(me) && counts(id, file, chosen) && tracks.get(&(id.clone(), file.to_string())).is_none_or(|t| t.open_for(me).is_some()))
}

fn snapshot(core: &Core, room: &str) -> (Vec<Log>, BTreeMap<String, Config>, Tracks) {
    let logs = room::logs(core, room);
    let (configs, tracks) = (room_configs(core, room, &logs), tracks(&logs, None));
    (logs, configs, tracks)
}

fn room_configs(core: &Core, room: &str, logs: &[Log]) -> BTreeMap<String, Config> {
    members_run(configs(logs, None), &room_members(core, room))
}

pub fn members_run(mut configs: BTreeMap<String, Config>, members: &[String]) -> BTreeMap<String, Config> {
    for c in configs.values_mut() {
        c.runner = c.runner.take().filter(|r| members.contains(r));
    }
    configs
}

pub fn wanted_sets(core: &Core, room: &str, logs: &[Log], sets: &[Set]) -> HashSet<String> {
    let (me, configs, tracks, chosen) = (core.me(), room_configs(core, room, logs), tracks(logs, None), chosen(logs, None));
    let wanted = |set: &Set, f: &File| analyzable(f) && open(&format!("{}/{}", set.id, f.owner), &me, &configs, &tracks, &chosen);
    sets.iter().filter(|set| set.files.iter().any(|f| wanted(set, f))).map(|set| set.id.clone()).collect()
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

fn room_members(core: &Core, room: &str) -> Vec<String> {
    let me = core.me();
    let others: Vec<String> = core.rooms.lock().unwrap().list.iter().find(|r| r.id == room).map(|r| r.members.iter().map(|m| m.to_string()).filter(|m| *m != me).collect()).unwrap_or_default();
    std::iter::once(me).chain(others).collect()
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
        let room = {
            let mut work = WORK.lock().unwrap();
            let room = core.active_room_id();
            *work = Some(room.clone());
            room
        };
        tick(&core, &room);
        *WORK.lock().unwrap() = None;
        std::thread::sleep(CHECK);
    }
}

fn tick(core: &Arc<Core>, room: &str) {
    let (logs, configs, tracks) = snapshot(core, room);
    let me = core.me();
    if !runner_wants(&me, &configs) || STOP.load(Ordering::Relaxed) {
        return;
    }
    for record in abandoned(&me, &tracks) {
        core.append_log(room, &record);
    }
    let chosen = chosen(&logs, None);
    let inputs = inputs(core, room, &logs, |file| open(file, &me, &configs, &tracks, &chosen));
    let pick = next(&me, recording(core), &bundled(), |id, s| present(core, id, s), &configs, &tracks, &chosen, &inputs);
    let Some(pick) = pick else { return };
    let (Some(plugin), Some(config)) = (plugins::find(&pick.plugin), configs.get(&pick.plugin)) else { return };
    work(core, room, plugin, config, &pick.key, &inputs[pick.input]);
}

fn work(core: &Arc<Core>, room: &str, plugin: &'static Plugin, config: &Config, key: &str, input: &Input) {
    let run = now_us();
    let about = json!({"plugin": plugin.id, "version": plugin.version, "settings": config.settings, "file": input.file});
    core.append_log(room, &job_record("claim", key, run, about));
    CANCEL.store(false, Ordering::Relaxed);
    tracing::info!("job {key} started on {}", input.file);
    *RUNNING.lock().unwrap() = Some(Running { room: room.into(), key: key.into(), plugin: plugin.id.into(), file: input.file.clone(), run, progress: 0.0 });
    emit(core);
    let work = work_root(core).join(run.to_string());
    let outcome = run_parts(core, room, plugin, config, input, &work);
    let about = json!({"plugin": plugin.id, "version": plugin.version, "file": input.file});
    match outcome {
        Some(Ok((found, tags, note))) => {
            tracing::info!("job {key} done with {} events{}", found.len(), if note.is_empty() { String::new() } else { format!(" ({note})") });
            finish(core, room, plugin, key, run, input, &found, &tags, &note)
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

type Results = (Vec<(i64, Found)>, BTreeMap<String, String>, String);

fn run_parts(core: &Arc<Core>, room: &str, plugin: &'static Plugin, config: &Config, input: &Input, work: &std::path::Path) -> Option<Result<Results, String>> {
    let mut found = Vec::new();
    let mut tags = BTreeMap::new();
    let mut notes = Vec::new();
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
        match watch(core, room, plugin, &job, (i as f64 / count, 1.0 / count))? {
            Ok(output) => {
                found.extend(output.events.into_iter().map(|e| (part.start_us, e)));
                tags.extend(output.tags);
                notes.extend(Some(output.note).filter(|n| !n.is_empty()));
            }
            Err(e) => return Some(Err(e)),
        }
    }
    Some(Ok((found, tags, notes.join("; "))))
}

fn stale(core: &Core, room: &str, plugin: &Plugin) -> bool {
    let logs = room::logs(core, room);
    let runs = room_configs(core, room, &logs).get(plugin.id).is_some_and(|c| c.on && c.runner.as_deref() == Some(core.me().as_str()));
    core.active_room_id() != room || (recording(core) && !plugin.live) || !runs
}

fn watch(core: &Core, room: &str, plugin: &Plugin, job: &plugins::Job, share: (f64, f64)) -> Option<Result<Output, String>> {
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
            if STOP.load(Ordering::Relaxed) || stale(core, room, plugin) {
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
                return (!stale(core, room, plugin)).then_some(Ok(output));
            }
            Ok(Reply::Failed(e)) => {
                let _ = process.child.wait();
                return Some(Err(e));
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                let _ = process.child.wait();
                return Some(Err("The extension stopped without a result.".into()));
            }
        }
    }
}

fn finish(core: &Arc<Core>, room: &str, plugin: &Plugin, key: &str, run: i64, input: &Input, found: &[(i64, Found)], tags: &BTreeMap<String, String>, note: &str) {
    let mut id = 0;
    for (start_us, one) in found {
        id = now_us().max(id + 1);
        core.append_log(room, &event_record(plugin, key, run, input, *start_us, one, id));
    }
    let mut done = json!({"plugin": plugin.id, "version": plugin.version, "file": input.file, "events": found.len(), "tags": tags});
    if !note.is_empty() {
        done["note"] = json!(note);
    }
    core.append_log(room, &job_record("done", key, run, done));
    party::send(core);
}

fn holders_of(core: &Core, mine: &HashMap<String, u64>) -> impl Fn(&Set, &File) -> Vec<Holder> {
    let me = core.me();
    let peers = room::others_have(core);
    let mine = mine.clone();
    move |set: &Set, f: &File| {
        let mut out: Vec<Holder> = room::full(&mine, set, f).then(|| Holder { id: me.clone(), online: true }).into_iter().collect();
        out.extend(peers.iter().filter(|(_, have, _)| room::full(have, set, f)).map(|(id, _, online)| Holder { id: id.clone(), online: *online }));
        out
    }
}

pub fn analysis_by_set(core: &Core, room: &str, logs: &[Log], sets: &[Set], mine: &HashMap<String, u64>) -> HashMap<String, Vec<Value>> {
    let configs = room_configs(core, room, logs);
    let phases = analysis(&core.me(), &online(core), &configs, &tracks(logs, None), &chosen(logs, None), sets, holders_of(core, mine));
    let running = RUNNING.lock().unwrap();
    phases.into_iter().map(|(set, list)| { let json = analysis_json(&list, running.as_ref(), &set); (set, json) }).collect()
}

pub fn plugins_json(core: &Core, room: &str) -> Vec<Value> {
    let logs = room::logs(core, room);
    let configs = room_configs(core, room, &logs);
    let sets = room::sets(core, room, &logs);
    let mine = data::have_counts(core, room, &sets);
    let by_set = analysis_by_set(core, room, &logs, &sets, &mine);
    let count = |id: &str, states: &[&str]| by_set.values().filter(|list| list.iter().any(|a| a["plugin"] == id && states.contains(&a["state"].as_str().unwrap_or_default()))).count();
    plugins::ALL
        .iter()
        .map(|p| {
            let config = configs.get(p.id);
            let settings = config.map_or_else(|| plugins::normalize(p, &Value::Null), |c| c.settings.clone());
            let missing = plugins::missing(p, &plugin_dir(core, p.id), &settings);
            let waiting_for = by_set.values().flatten().find(|a| a["plugin"] == p.id && !a["waiting_for"].is_null()).map(|a| a["waiting_for"].clone());
            let mut v = plugins::manifest(p);
            v["on"] = json!(config.is_some_and(|c| c.on));
            v["room_version"] = json!(config.map(|c| c.version.clone()));
            v["settings"] = settings;
            v["download_bytes"] = json!(missing.iter().map(|d| d.size).sum::<u64>());
            v["ready"] = json!(missing.is_empty());
            v["download"] = crate::download::progress(p.id);
            v["runner"] = json!(config.and_then(|c| c.runner.clone()));
            v["left"] = json!(count(p.id, &["left", "running"]));
            v["failed"] = json!(count(p.id, &["failed"]));
            v["waiting_for"] = waiting_for.unwrap_or(Value::Null);
            v
        })
        .collect()
}

pub fn room_plugins(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    Ok(json!(plugins_json(core, &core.active_room_id())))
}

pub fn set_room_plugin(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let plugin = plugins::find(&arg::<String>(&args, "id")?).ok_or("Unknown extension.")?;
    let room = core.active_room_id();
    let logs = room::logs(core, &room);
    let runner = room_configs(core, &room, &logs).remove(plugin.id).and_then(|c| c.runner).unwrap_or_else(|| core.me());
    let config = Config { on: arg(&args, "on")?, version: plugin.version.into(), settings: plugins::normalize(plugin, &args["settings"]), runner: Some(runner) };
    core.append_log(&room, &plugin_record(plugin.id, &config, change_time(now_us(), &logs, plugin.id)));
    Ok(Value::Null)
}

fn plugin_record(id: &str, config: &Config, t_us: i64) -> Value {
    let mut record = json!({"k": "plugin", "plugin": id, "on": config.on, "version": config.version, "settings": config.settings, "t_us": t_us});
    if let Some(runner) = &config.runner {
        record["runner"] = json!(runner);
    }
    record
}

fn runner_record(plugin: &Plugin, runner: String, logs: &[Log], now: i64) -> Value {
    let config = configs(logs, None).remove(plugin.id).unwrap_or_else(|| Config { on: false, version: plugin.version.into(), settings: plugins::normalize(plugin, &Value::Null), runner: None });
    plugin_record(plugin.id, &Config { runner: Some(runner), ..config }, change_time(now, logs, plugin.id))
}

pub fn set_plugin_runner(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let plugin = plugins::find(&arg::<String>(&args, "id")?).ok_or("Unknown extension.")?;
    let room = core.active_room_id();
    let logs = room::logs(core, &room);
    let runner: String = arg(&args, "runner")?;
    if !room_members(core, &room).contains(&runner) {
        return Err("Unknown member.".into());
    }
    core.append_log(&room, &runner_record(plugin, runner, &logs, now_us()));
    Ok(Value::Null)
}

pub fn prepare_plugin(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let plugin = plugins::find(&arg::<String>(&args, "id")?).ok_or("Unknown extension.")?;
    let settings = configs(&room::logs(core, &core.active_room_id()), None).get(plugin.id).map_or_else(|| plugins::normalize(plugin, &Value::Null), |c| c.settings.clone());
    let dir = plugin_dir(core, plugin.id);
    plugins::fetch_group(plugin.id, &dir, &plugins::missing(plugin, &dir, &settings)).map(|_| Value::Null)
}

pub fn retries(configs: &BTreeMap<String, Config>, chosen: &HashMap<String, String>, set: &Set, plugin: Option<&str>) -> Vec<Value> {
    configs
        .iter()
        .filter(|(id, c)| c.on && plugin.is_none_or(|p| p == id.as_str()))
        .flat_map(|(id, c)| {
            let prefix = prefix(id, c);
            set.files.iter().map(move |f| format!("{}/{}", set.id, f.owner)).filter(move |file| counts(id, file, chosen)).map(move |file| job_record("retry", &format!("{prefix}{file}"), 0, Value::Null))
        })
        .collect()
}

pub fn retry_job(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let (room, set): (String, String) = (arg(&args, "room")?, arg(&args, "set")?);
    let plugin: Option<String> = arg(&args, "plugin")?;
    if !room::valid_id(&room) || !room::valid_id(&set) || !core.rooms.lock().unwrap().list.iter().any(|r| r.id == room) {
        return Err("That recording is not on this PC.".into());
    }
    let logs = room::logs(core, &room);
    let found = room::sets(core, &room, &logs).into_iter().find(|s| s.id == set).ok_or("That recording is not on this PC.")?;
    for record in retries(&room_configs(core, &room, &logs), &chosen(&logs, None), &found, plugin.as_deref()) {
        core.append_log(&room, &record);
    }
    Ok(Value::Null)
}

pub fn cancel_set(room: &str, set: &str) {
    let file = format!("{set}/");
    if RUNNING.lock().unwrap().as_ref().is_some_and(|r| r.room == room && r.file.starts_with(&file)) {
        CANCEL.store(true, Ordering::Relaxed);
    }
}

pub fn stop_now(core: &Core) {
    STOP.store(true, Ordering::Relaxed);
    let Some(r) = RUNNING.lock().unwrap().take() else { return };
    tracing::info!("job {} stopped", r.key);
    core.append_log(&r.room, &job_record("stop", &r.key, r.run, json!({"plugin": r.plugin, "file": r.file})));
}

fn wait_idle(busy: impl Fn(Option<&str>) -> bool) {
    let started = Instant::now();
    while busy(WORK.lock().unwrap().as_deref()) && started.elapsed() < STOP_WAIT {
        std::thread::sleep(EMIT_EVERY);
    }
}

pub fn release_room(room: &str) {
    wait_idle(|work| work == Some(room));
}

pub fn stop_and_wait(core: &Core) {
    stop_now(core);
    wait_idle(|work| work.is_some());
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

    fn on(model: &str) -> Config {
        Config { on: true, version: "1.0.0".into(), settings: json!({"model": model}), runner: Some("a".into()) }
    }

    fn key(file: &str) -> String {
        format!("{}{file}", prefix("transcribe", &on("small")))
    }

    fn key_with(model: &str, file: &str) -> String {
        format!("{}{file}", prefix("transcribe", &on(model)))
    }

    const BUNDLED: [(&str, &str, bool); 1] = [("transcribe", "1.0.0", false)];

    fn pick(me: &str, recording: bool, logs: &[Log], inputs: &[Input]) -> Option<String> {
        next(me, recording, &BUNDLED, |_, _| true, &configs(logs, None), &tracks(logs, None), &HashMap::new(), inputs).map(|p| inputs[p.input].file.clone())
    }

    fn base_logs(extra_a: Vec<Value>, extra_b: Vec<Value>) -> Vec<Log> {
        let mut a = vec![config_record(1, true, "small")];
        a.extend(extra_a);
        vec![log("a", 0, a), log("b", 0, extra_b)]
    }

    fn append(logs: &mut [Log], i: usize, record: Value) {
        Arc::make_mut(&mut logs[i].records).push(record);
    }

    fn by_a(mut record: Value) -> Value {
        record["runner"] = json!("a");
        record
    }

    fn timed(mut record: Value, t: i64) -> Value {
        record["t_us"] = json!(t);
        record
    }

    #[test]
    fn changing_settings_keeps_old_results_and_reruns_nothing() {
        let done = timed(job_record("done", &key("s1/a"), 3, json!({"tags": {"x": "X"}})), 10);
        let event = json!({"k": "event", "id": 11, "kind": "speech", "plugin": {"id": "transcribe"}, "run": 3});
        let mut logs = base_logs(vec![event, done], vec![]);
        let inputs = [input("s1", "a", 10, true), input("s2", "a", 20, true)];
        assert_eq!(pick("a", false, &logs, &inputs).as_deref(), Some("s2/a"));
        append(&mut logs, 1, by_a(config_record(100, true, "base")));
        assert_eq!(configs(&logs, None)["transcribe"].settings["model"], "base");
        assert_eq!(shown(&logs, None), HashSet::from([("a".to_string(), 3)]));
        assert_eq!(tag_names(&logs, None), BTreeMap::from([("x".to_string(), "X".to_string())]));
        assert_eq!(pick("a", false, &logs, &inputs[..1]), None);
        assert_eq!(pick("a", false, &logs, &inputs).as_deref(), Some("s2/a"));
        append(&mut logs, 0, config_record(200, false, "base"));
        assert!(shown(&logs, None).is_empty());
    }

    #[test]
    fn the_latest_config_wins_on_the_shared_clock() {
        let logs = vec![log("a", 0, vec![config_record(100, true, "small")]), log("b", 50, vec![config_record(120, false, "base")])];
        assert!(configs(&logs, None)["transcribe"].on);
        let logs = vec![log("a", 0, vec![config_record(100, true, "small")]), log("b", -50, vec![config_record(120, false, "base")])];
        assert!(!configs(&logs, None)["transcribe"].on);
    }

    #[test]
    fn the_runner_defaults_to_who_turned_it_on() {
        let logs = vec![log("a", 0, vec![config_record(1, true, "small")]), log("b", 0, vec![])];
        assert_eq!(configs(&logs, None)["transcribe"].runner.as_deref(), Some("a"));
        let by_b = vec![log("a", 0, vec![]), log("b", 0, vec![config_record(1, true, "small")])];
        assert_eq!(configs(&by_b, None)["transcribe"].runner.as_deref(), Some("b"));
        let old_auto = json!({"k": "plugin", "plugin": "transcribe", "on": true, "version": "1.0.0", "settings": {}, "runner": "", "t_us": 2});
        let logs = vec![log("a", 0, vec![config_record(1, true, "small")]), log("b", 0, vec![old_auto])];
        assert_eq!(configs(&logs, None)["transcribe"].runner.as_deref(), Some("b"));
        let chosen = json!({"k": "plugin", "plugin": "transcribe", "on": true, "version": "1.0.0", "settings": {}, "runner": "a", "t_us": 3});
        let logs = vec![log("a", 0, vec![]), log("b", 0, vec![config_record(1, true, "small"), chosen])];
        assert_eq!(configs(&logs, None)["transcribe"].runner.as_deref(), Some("a"));
    }

    #[test]
    fn only_the_runner_processes_and_a_runner_who_left_is_no_runner() {
        let logs = base_logs(vec![], vec![]);
        let inputs = [input("s1", "a", 10, true)];
        assert_eq!(pick("a", false, &logs, &inputs).as_deref(), Some("s1/a"));
        assert_eq!(pick("b", false, &logs, &inputs), None);
        let members = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        assert_eq!(members_run(configs(&logs, None), &members(&["a", "b"]))["transcribe"].runner.as_deref(), Some("a"));
        assert_eq!(members_run(configs(&logs, None), &members(&["b", "c"]))["transcribe"].runner, None);
    }

    #[test]
    fn changing_the_runner_keeps_the_rooms_version_and_settings() {
        let plugin = plugins::find("transcribe").unwrap();
        let mut logs = base_logs(vec![], vec![]);
        let record = runner_record(plugin, "b".into(), &logs, 100);
        append(&mut logs, 1, record);
        let config = &configs(&logs, None)["transcribe"];
        assert_eq!(config.runner.as_deref(), Some("b"));
        assert_eq!(prefix("transcribe", config), prefix("transcribe", &on("small")));
        let fresh = runner_record(plugin, "b".into(), &[], 100);
        assert_eq!((fresh["on"].clone(), fresh["version"].clone()), (json!(false), json!(plugin.version)));
    }

    #[test]
    fn the_runner_wants_every_finished_file_of_an_enabled_extension() {
        let logs = base_logs(vec![], vec![]);
        let all = configs(&logs, None);
        assert!(runner_wants("a", &all));
        assert!(!runner_wants("b", &all));
        let off = vec![log("a", 0, vec![config_record(1, true, "small"), config_record(2, false, "small")])];
        assert!(!runner_wants("a", &configs(&off, None)));
        assert!(!runner_wants("a", &BTreeMap::new()));
    }

    #[test]
    fn the_runner_wants_only_files_that_still_need_analysis() {
        let done = timed(job_record("done", &key("s1/a"), 3, Value::Null), 10);
        let failed = timed(job_record("fail", &key("s2/a"), 4, Value::Null), 11);
        let logs = base_logs(vec![done, failed], vec![]);
        let (configs, tracks, chosen) = (configs(&logs, None), tracks(&logs, None), HashMap::new());
        let open = |file: &str, me: &str| open(file, me, &configs, &tracks, &chosen);
        assert!(!open("s1/a", "a"));
        assert!(!open("s2/a", "a"));
        assert!(open("s3/a", "a"));
        assert!(!open("s3/a", "b"));
    }

    #[test]
    fn a_recording_with_one_file_running_reads_running_and_a_failure_outranks_it() {
        let left = Phase::Left(Wait::Queue);
        assert_eq!(fold(vec![Phase::Running, left.clone(), left.clone()]), Phase::Running);
        assert_eq!(fold(vec![left.clone(), Phase::Running, Phase::Failed("x".into())]), Phase::Failed("x".into()));
        assert_eq!(fold(vec![Phase::Done, left.clone()]), left);
    }

    #[test]
    fn old_yield_and_stop_records_read_as_pending() {
        let inputs = [input("s1", "a", 10, true)];
        for state in ["yield", "stop", "retry"] {
            let logs = base_logs(vec![timed(job_record("claim", &key("s1/a"), 5, Value::Null), 5), timed(job_record(state, &key("s1/a"), 5, Value::Null), 6)], vec![]);
            assert_eq!(pick("a", false, &logs, &inputs).as_deref(), Some("s1/a"), "{state}");
            let t = tracks(&logs, None);
            assert_eq!(phase(t.get(&("transcribe".into(), "s1/a".into())), Some("a"), "a", &HashSet::new(), &[Holder { id: "a".into(), online: true }]), Phase::Left(Wait::Queue));
        }
    }

    #[test]
    fn a_file_nobody_online_holds_fails_as_missing_and_only_an_offline_runner_waits() {
        let t = Track::default();
        let online = HashSet::from(["a".to_string()]);
        let held_by_b = [Holder { id: "b".into(), online: false }];
        assert_eq!(phase(Some(&t), Some("a"), "a", &online, &held_by_b), Phase::Missing);
        assert_eq!(phase(None, Some("a"), "a", &online, &[]), Phase::Missing);
        let retried = base_logs(vec![timed(job_record("retry", &key("s1/b"), 5, Value::Null), 5)], vec![]);
        assert_eq!(phase(tracks(&retried, None).get(&("transcribe".to_string(), "s1/b".to_string())), Some("a"), "a", &online, &held_by_b), Phase::Missing);
        assert_eq!(phase(None, Some("c"), "a", &online, &[]), Phase::Left(Wait::Member("c".into())));
        let held_online = [Holder { id: "c".into(), online: true }, Holder { id: "b".into(), online: false }];
        assert_eq!(phase(None, Some("a"), "a", &online, &held_online), Phase::Left(Wait::Receiving));
        let held_here = [Holder { id: "a".into(), online: true }];
        assert_eq!(phase(None, Some("a"), "a", &online, &held_here), Phase::Left(Wait::Queue));
        assert_eq!(phase(None, Some("b"), "a", &online, &held_here), Phase::Left(Wait::Member("b".into())));
        assert_eq!(phase(None, None, "a", &online, &held_here), Phase::Left(Wait::NoRunner));
        let logs = base_logs(vec![timed(job_record("claim", &key("s1/b"), 5, Value::Null), 5)], vec![]);
        let t = tracks(&logs, None);
        let file = ("transcribe".to_string(), "s1/b".to_string());
        assert_eq!(phase(t.get(&file), Some("a"), "a", &online, &held_here), Phase::Running);
        assert_eq!(phase(t.get(&file), Some("a"), "b", &HashSet::new(), &held_here), Phase::Left(Wait::Member("a".into())));
        let failed = base_logs(vec![timed(job_record("fail", &key("s1/b"), 5, json!({"error": "x"})), 5)], vec![]);
        assert_eq!(phase(tracks(&failed, None).get(&file), Some("a"), "a", &online, &held_here), Phase::Failed("x".into()));
        let done = base_logs(vec![timed(job_record("done", &key("s1/b"), 5, Value::Null), 5)], vec![]);
        assert_eq!(phase(tracks(&done, None).get(&file), Some("a"), "a", &online, &held_here), Phase::Done);
    }

    #[test]
    fn a_recording_shows_its_worst_file_and_only_enabled_extensions() {
        let seg = |set: &str| json!({"k": "seg", "series": "v", "set": set, "n": 1, "capture": "1", "t_us": 1, "d_us": 1, "size": 1, "sha256": "x"});
        let rec = |set: &str| json!({"k": "rec", "set": set, "start_us": 1});
        let end = |set: &str| json!({"k": "rec_end", "set": set, "t_us": 9});
        let voice = json!({"k": "seg", "series": "m", "set": "d1", "n": 1, "capture": "1", "t_us": 1, "d_us": 1, "size": 1, "sha256": "x"});
        let logs = vec![log("a", 0, vec![config_record(1, true, "small"), rec("s1"), seg("s1"), end("s1"), timed(job_record("done", &key("s1/a"), 3, Value::Null), 5), rec("d1"), voice, end("d1")]), log("b", 0, vec![rec("s1"), seg("s1"), end("s1")])];
        let sets = room::derive_sets("r", &logs);
        let holders = |_: &Set, f: &File| vec![Holder { id: f.owner.clone(), online: true }];
        let run = |logs: &[Log]| analysis("a", &HashSet::from(["b".to_string()]), &configs(logs, None), &tracks(logs, None), &chosen(logs, None), &sets, &holders);
        let out = run(&logs);
        assert_eq!(out["s1"], vec![("transcribe".to_string(), Phase::Left(Wait::Receiving))]);
        assert!(!out.contains_key("d1"));
        let mut done = logs;
        append(&mut done, 0, timed(job_record("done", &key("s1/b"), 4, Value::Null), 6));
        assert_eq!(run(&done)["s1"], vec![("transcribe".to_string(), Phase::Done)]);
        append(&mut done, 0, config_record(10, false, "small"));
        assert!(run(&done).is_empty());
    }

    #[test]
    fn a_change_made_here_beats_a_record_from_a_fast_clock() {
        let ahead = log("b", 0, vec![config_record(5_000, true, "small")]);
        let t = change_time(1_000, &[ahead], "transcribe");
        assert_eq!(t, 5_001);
        let logs = vec![log("a", 0, vec![config_record(t, true, "base")]), log("b", 0, vec![config_record(5_000, true, "small")])];
        assert_eq!(configs(&logs, None)["transcribe"].settings["model"], "base");
        assert_eq!(change_time(9_000, &logs[1..], "transcribe"), 9_000);
        assert_eq!(change_time(7, &[], "transcribe"), 7);
    }

    #[test]
    fn the_download_size_follows_the_chosen_model_until_it_is_installed() {
        let dir = std::env::temp_dir().join(format!("ezh-test-size-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let plugin = plugins::find("transcribe").unwrap();
        let size = |model: &str| plugins::missing(plugin, &dir, &plugins::normalize(plugin, &json!({"model": model}))).iter().map(|d| d.size).sum::<u64>();
        let (small, base, medium) = (size("small"), size("base"), size("medium"));
        assert!(base < small && small < medium);
        std::fs::create_dir_all(dir.join("whisper")).unwrap();
        std::fs::write(dir.join("whisper").join("whisper-cli.exe"), b"").unwrap();
        std::fs::write(dir.join("ggml-base.bin"), b"").unwrap();
        assert_eq!(size("base"), 0);
        assert_eq!(size("small"), 487_601_967);
        assert_eq!(size("medium"), 1_533_763_059);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keys_change_with_version_and_settings() {
        let a = on("small");
        let b = Config { settings: json!({"model": "base"}), ..a.clone() };
        let c = Config { version: "1.1.0".into(), ..a.clone() };
        assert_ne!(prefix("transcribe", &a), prefix("transcribe", &b));
        assert_ne!(prefix("transcribe", &a), prefix("transcribe", &c));
        assert_eq!(prefix("transcribe", &a), prefix("transcribe", &a.clone()));
        assert_eq!(split_key(&key("s1/a")), Some(("transcribe".to_string(), "s1/a".to_string())));
        assert_eq!(split_key("s1/a"), None);
    }

    #[test]
    fn the_newest_recording_comes_first_and_an_interrupted_own_claim_before_it() {
        let logs = base_logs(vec![], vec![]);
        let inputs = [input("s1", "b", 10, true), input("s2", "b", 20, true), input("s0", "a", 1, true)];
        assert_eq!(pick("a", false, &logs, &inputs).as_deref(), Some("s2/b"));
        assert_eq!(pick("a", false, &logs, &[input("s1", "b", 10, false)]), None);
        let claimed = base_logs(vec![timed(job_record("claim", &key("s0/a"), 5, Value::Null), 5)], vec![]);
        assert_eq!(pick("a", false, &claimed, &inputs).as_deref(), Some("s0/a"));
    }

    #[test]
    fn nothing_runs_while_recording_unless_live_or_when_off_or_mismatched() {
        let inputs = [input("s1", "a", 10, true)];
        assert_eq!(pick("a", true, &base_logs(vec![], vec![]), &inputs), None);
        let off = vec![log("a", 0, vec![config_record(1, true, "small"), config_record(2, false, "small")])];
        assert_eq!(pick("a", false, &off, &inputs), None);
        let mut newer = config_record(3, true, "small");
        newer["version"] = json!("2.0.0");
        assert_eq!(pick("a", false, &[log("a", 0, vec![newer])], &inputs), None);
        let live = [("transcribe", "1.0.0", true)];
        let logs = base_logs(vec![], vec![]);
        assert!(next("a", true, &live, |_, _| true, &configs(&logs, None), &tracks(&logs, None), &HashMap::new(), &inputs).is_some());
        assert!(next("a", false, &live, |_, _| false, &configs(&logs, None), &tracks(&logs, None), &HashMap::new(), &inputs).is_none());
    }

    #[test]
    fn done_and_failed_files_wait_for_a_retry() {
        let inputs = [input("s1", "a", 10, true)];
        let done = base_logs(vec![timed(job_record("done", &key("s1/a"), 3, Value::Null), 5)], vec![]);
        assert_eq!(pick("a", false, &done, &inputs), None);
        let mut failed = base_logs(vec![timed(job_record("claim", &key("s1/a"), 3, Value::Null), 4), timed(job_record("fail", &key("s1/a"), 3, json!({"error": "x"})), 5)], vec![]);
        assert_eq!(pick("a", false, &failed, &inputs), None);
        append(&mut failed, 1, timed(job_record("retry", &key("s1/a"), 0, Value::Null), 6));
        assert_eq!(pick("a", false, &failed, &inputs).as_deref(), Some("s1/a"));
        append(&mut failed, 0, timed(job_record("done", &key_with("base", "s1/a"), 7, Value::Null), 7));
        assert_eq!(pick("a", false, &failed, &inputs), None);
        append(&mut failed, 1, timed(job_record("retry", &key("s1/a"), 0, Value::Null), 8));
        assert_eq!(pick("a", false, &failed, &inputs).as_deref(), Some("s1/a"));
        assert_eq!(shown(&failed, None).len(), 1);
    }

    #[test]
    fn a_job_stopped_twice_without_finishing_fails_and_a_stop_on_purpose_does_not_count() {
        let claim = |run: i64| timed(job_record("claim", &key("s1/a"), run, Value::Null), run);
        let ends = |me: &str, logs: &[Log]| abandoned(me, &tracks(logs, None)).iter().map(|r| (r["state"].as_str().unwrap_or_default().to_string(), r["job"].as_str().unwrap_or_default().to_string(), r["run"].as_i64().unwrap_or_default())).collect::<Vec<_>>();
        assert!(ends("a", &base_logs(vec![claim(1)], vec![])).is_empty());
        assert_eq!(ends("a", &base_logs(vec![claim(1), claim(2)], vec![])), vec![("fail".to_string(), key("s1/a"), 2)]);
        assert!(ends("b", &base_logs(vec![claim(1), claim(2)], vec![])).is_empty());
        let stopped = base_logs(vec![claim(1), timed(job_record("stop", &key("s1/a"), 1, Value::Null), 2), claim(3)], vec![]);
        assert!(ends("a", &stopped).is_empty());
        assert_eq!(pick("a", false, &stopped, &[input("s1", "a", 10, true)]).as_deref(), Some("s1/a"));
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
    fn a_file_done_twice_shows_the_newest_result_and_ties_go_to_the_lowest_owner() {
        let k = key("s1/a");
        let a = vec![plugin_event(1, 10), timed(job_record("done", &k, 1, json!({"tags": {"x": "X"}})), 50), plugin_event(2, 11)];
        let b = vec![plugin_event(4, 20), timed(job_record("done", &key_with("base", "s1/a"), 4, Value::Null), 60)];
        let logs = base_logs(a, b);
        assert_eq!(shown(&logs, None), HashSet::from([("b".to_string(), 4)]));
        let tie = base_logs(vec![timed(job_record("done", &k, 1, Value::Null), 50)], vec![timed(job_record("done", &k, 4, Value::Null), 50)]);
        assert_eq!(shown(&tie, None), HashSet::from([("a".to_string(), 1)]));
        let mut events = vec![
            json!({"author": "a", "run": 1, "plugin": {"id": "transcribe"}}),
            json!({"author": "b", "run": 4, "plugin": {"id": "transcribe"}}),
            json!({"author": "b", "plugin": null}),
        ];
        keep_shown(&mut events, &logs, None);
        assert_eq!(events.len(), 2);
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
    fn re_running_asks_every_enabled_extension_or_the_one_named_for_each_counted_file() {
        let seg = |owner_set: &str| json!({"k": "seg", "series": "v", "set": owner_set, "n": 1, "capture": "1", "t_us": 1, "d_us": 1, "size": 1, "sha256": "x"});
        let rec = |set: &str| json!({"k": "rec", "set": set, "start_us": 1});
        let logs = vec![log("a", 0, vec![config_record(1, true, "small"), rec("s1"), seg("s1")]), log("b", 0, vec![rec("s1"), seg("s1")])];
        let sets = room::derive_sets("r", &logs);
        let all = retries(&configs(&logs, None), &chosen(&logs, None), &sets[0], None);
        assert_eq!(all.iter().map(|r| r["job"].as_str().unwrap().to_string()).collect::<Vec<_>>(), vec![key("s1/a"), key("s1/b")]);
        assert!(all.iter().all(|r| r["state"] == "retry"));
        assert!(retries(&configs(&logs, None), &chosen(&logs, None), &sets[0], Some("apex-matches")).is_empty());
        assert_eq!(retries(&configs(&logs, None), &chosen(&logs, None), &sets[0], Some("transcribe")).len(), 2);
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

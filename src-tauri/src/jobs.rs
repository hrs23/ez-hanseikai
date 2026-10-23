use std::cmp::Reverse;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::plugins::{self, Found, Media, Output, Plugin, Reply};
use crate::room::{self, File, Log, Seg, Set, SERIES};
use crate::ops::{arg, recording_ids};
use crate::{data, ffmpeg, hex, now_us, party, stream, Core};

const CHECK: Duration = Duration::from_secs(2);
const WAIT_STEP: Duration = Duration::from_millis(250);
const SETTINGS_HASH_BYTES: usize = 6;
const STOP_WAIT: Duration = Duration::from_secs(10);
pub const UNFINISHED_MAX: u32 = 2;
pub const AUTO_ANALYSIS_US: i64 = 3_600_000_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub on: bool,
    pub runner: Option<String>,
    pub settings: BTreeMap<String, String>,
}

impl Config {
    fn runs_on(&self, me: &str) -> bool {
        self.on && self.runner.as_deref() == Some(me)
    }
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
static SKIPPED: Mutex<BTreeMap<String, &'static str>> = Mutex::new(BTreeMap::new());

pub fn busy() -> bool {
    RUNNING.lock().unwrap().is_some()
}

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
            let (Some(id), Some(t)) = (r["plugin"].as_str().filter(|id| plugins::find(id).is_some()), r["t_us"].as_i64()) else { continue };
            let at = (t - log.offset_us, log.owner.clone());
            if latest.get(id).is_none_or(|(old, _)| at > *old) {
                let runner = r["runner"].as_str().filter(|m| !m.is_empty()).unwrap_or(&log.owner).to_string();
                let settings = r["settings"].as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_str().filter(|v| !v.is_empty())?.to_string()))).collect();
                let config = Config { on: r["on"] == true, runner: Some(runner), settings };
                latest.insert(id.to_string(), (at, config));
            }
        }
    }
    latest
}

pub fn configs(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> BTreeMap<String, Config> {
    every_config(logs, caps).into_iter().filter(|(id, _)| plugins::find(id).is_some_and(Plugin::analyzes)).collect()
}

fn every_config(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> BTreeMap<String, Config> {
    newest(logs, caps).into_iter().map(|(id, (_, c))| (id, c)).collect()
}

pub fn change_time(now: i64, logs: &[Log], id: &str) -> i64 {
    newest(logs, None).get(id).map_or(now, |((at, _), _)| now.max(at + 1))
}

pub fn prefix(plugin: &str, version: &str) -> String {
    let digest = Sha256::digest(b"{}");
    format!("{plugin}@{version}/{}/", hex(&digest[..SETTINGS_HASH_BYTES]))
}

fn shipped(id: &str) -> String {
    prefix(id, plugins::find(id).map_or("", |p| p.version))
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
                Some("retry" | "stop") => Status::Retry,
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
    let mut counts: HashMap<(&str, &str), u64> = HashMap::new();
    for (log, records) in capped(logs, caps) {
        for r in records.iter().filter(|r| r["k"] == "seg" && r["series"] == SERIES[0]) {
            let Some(set) = r["set"].as_str() else { continue };
            *counts.entry((set, &log.owner)).or_default() += 1;
        }
    }
    let mut out: HashMap<String, (u64, String)> = HashMap::new();
    for ((set, owner), n) in counts {
        let file = format!("{set}/{owner}");
        let better = out.get(set).is_none_or(|(m, f)| (n, Reverse(&file)) > (*m, Reverse(f)));
        if better {
            out.insert(set.to_string(), (n, file));
        }
    }
    out.into_iter().map(|(set, (_, file))| (set, file)).collect()
}

pub fn counts(file: &str, chosen: &HashMap<String, String>) -> bool {
    file.split('/').next().is_some_and(|set| chosen.get(set).is_some_and(|f| f == file))
}

fn winners(logs: &[Log], caps: Option<&HashMap<String, u64>>) -> Vec<((String, i64), Value)> {
    let on: HashSet<String> = configs(logs, caps).into_iter().filter(|(_, c)| c.on).map(|(id, _)| id).collect();
    let chosen = chosen(logs, caps);
    tracks(logs, caps)
        .into_iter()
        .filter(|((plugin, file), _)| on.contains(plugin) && counts(file, &chosen))
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

pub fn next(me: &str, configs: &BTreeMap<String, Config>, tracks: &Tracks, chosen: &HashMap<String, String>, inputs: &[Input]) -> Option<Pick> {
    let mut picks = Vec::new();
    for (order, (id, config)) in configs.iter().enumerate() {
        if !config.runs_on(me) {
            continue;
        }
        let prefix = shipped(id);
        for (i, input) in inputs.iter().enumerate().filter(|(_, input)| input.here && counts(&input.file, chosen)) {
            let Some(resumed) = tracks.get(&(id.clone(), input.file.clone())).map_or(Some(false), |t| t.open_for(me)) else { continue };
            picks.push(((!resumed, Reverse(input.start_us), order), Pick { plugin: id.clone(), key: format!("{prefix}{}", input.file), input: i }));
        }
    }
    picks.into_iter().min_by(|a, b| a.0.cmp(&b.0)).map(|(_, p)| p)
}

pub fn runner_wants(me: &str, configs: &BTreeMap<String, Config>) -> bool {
    configs.values().any(|c| c.runs_on(me))
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
    Recording,
    Ffmpeg,
}

pub fn gate(recording: bool, ffmpeg: bool) -> Option<Wait> {
    if recording {
        Some(Wait::Recording)
    } else {
        (!ffmpeg).then_some(Wait::Ffmpeg)
    }
}

fn reason(wait: &Wait) -> &'static str {
    match wait {
        Wait::Queue => "queue",
        Wait::Receiving => "receiving",
        Wait::NoRunner => "no_runner",
        Wait::Member(_) => "waiting",
        Wait::Recording => "recording",
        Wait::Ffmpeg => "ffmpeg",
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Done,
    Running,
    Failed(String),
    Missing,
    Left(Wait),
    Idle,
}

pub fn phase(track: Option<&Track>, runner: Option<&str>, me: &str, online: &HashSet<String>, holders: &[Holder], local: Option<&Wait>) -> Phase {
    let alive = |who: &str| who == me || online.contains(who);
    match (track.and_then(|t| t.latest.as_ref()), runner) {
        (Some(Latest { status: Status::Done(_), .. }), _) => Phase::Done,
        (Some(Latest { status: Status::Failed(e), .. }), _) => Phase::Failed(e.clone()),
        (Some(Latest { at: (_, owner), status: Status::Claimed(_), .. }), Some(runner)) if owner == runner && alive(owner) => Phase::Running,
        (_, None) => Phase::Left(Wait::NoRunner),
        (_, Some(runner)) if !alive(runner) => Phase::Left(Wait::Member(runner.to_string())),
        (_, Some(runner)) => match local.filter(|_| runner == me) {
            Some(wait) => Phase::Left(wait.clone()),
            None if holders.iter().any(|h| h.id == runner) => Phase::Left(Wait::Queue),
            None if holders.iter().any(|h| h.online) => Phase::Left(Wait::Receiving),
            None => Phase::Missing,
        },
    }
}

fn rank(phase: &Phase) -> u8 {
    match phase {
        Phase::Done | Phase::Idle => 0,
        Phase::Left(_) => 1,
        Phase::Running => 2,
        Phase::Failed(_) | Phase::Missing => 3,
    }
}

fn fold(phases: Vec<Phase>) -> Phase {
    phases.into_iter().reduce(|a, b| if rank(&b) > rank(&a) { b } else { a }).unwrap_or(Phase::Done)
}

pub fn ended_recently(set: &Set, now: i64) -> bool {
    set.files.iter().filter_map(|f| f.end_us.map(|end| end - f.offset_us)).max().is_some_and(|end| now - end < AUTO_ANALYSIS_US)
}

pub fn due(set: &Set, plugin: &str, tracks: &Tracks, now: i64) -> bool {
    ended_recently(set, now) || set.files.iter().any(|f| tracks.contains_key(&(plugin.to_string(), format!("{}/{}", set.id, f.owner))))
}

pub fn analysis(
    me: &str,
    now: i64,
    online: &HashSet<String>,
    configs: &BTreeMap<String, Config>,
    tracks: &Tracks,
    chosen: &HashMap<String, String>,
    sets: &[Set],
    local: Option<&Wait>,
    holders: impl Fn(&Set, &File) -> Vec<Holder>,
) -> HashMap<String, Vec<(String, Phase)>> {
    let mut out: HashMap<String, Vec<(String, Phase)>> = HashMap::new();
    for set in sets {
        let finished: Vec<&File> = set.files.iter().filter(|f| analyzable(f)).collect();
        for (id, config) in configs.iter().filter(|(_, c)| c.on) {
            let phases: Vec<Phase> = finished
                .iter()
                .filter(|f| counts(&format!("{}/{}", set.id, f.owner), chosen))
                .map(|f| phase(tracks.get(&(id.clone(), format!("{}/{}", set.id, f.owner))), config.runner.as_deref(), me, online, &holders(set, f), local))
                .collect();
            if !phases.is_empty() {
                let phase = if due(set, id, tracks, now) { fold(phases) } else { Phase::Idle };
                out.entry(set.id.clone()).or_default().push((id.clone(), phase));
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
            Phase::Idle => json!({"plugin": plugin, "state": "idle"}),
            Phase::Left(wait) => {
                let who = match wait {
                    Wait::Member(id) => Some(id),
                    _ => None,
                };
                json!({"plugin": plugin, "state": "left", "reason": reason(wait), "waiting_for": who})
            }
        })
        .collect()
}

pub fn moment(start_us: i64, s: f64) -> i64 {
    start_us + (s * 1e6).round() as i64
}

pub fn target(found: &Found, input: &Input, start_us: i64) -> Value {
    let mut t = match found.at_s {
        Some(at) => json!({"t_us": moment(start_us, at)}),
        None => json!({}),
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
    let count = f.segs_of(capture, series).count();
    if count == 0 {
        return Some(Vec::new());
    }
    let pieces = stream::set_pieces(core, room, f, capture, series);
    (pieces.len() == count + 1).then(|| pieces.into_iter().map(|p| p.path).collect())
}

pub fn whole(picture: Option<&[PathBuf]>, voice: Option<&[PathBuf]>) -> bool {
    picture.is_some_and(|v| !v.is_empty()) && voice.is_some()
}

fn analyzable(f: &File) -> bool {
    f.end_us.is_some() && f.segs.iter().any(|s| s.series == SERIES[0])
}

pub fn inputs(core: &Core, room: &str, logs: &[Log], locate: impl Fn(&Set, &str) -> bool) -> Vec<Input> {
    let mut out = Vec::new();
    for set in room::sets(core, room, logs) {
        for f in set.files.iter().filter(|f| analyzable(f)) {
            let file = format!("{}/{}", set.id, f.owner);
            if !locate(&set, &file) {
                continue;
            }
            let mut parts = Vec::new();
            let mut complete = true;
            for capture in f.captures() {
                let segs: Vec<&Seg> = f.segs_of(capture, SERIES[0]).collect();
                let Some((start_us, end_us)) = span(&segs, f.offset_us) else { continue };
                let v = series(core, room, f, capture, SERIES[0]);
                let m = series(core, room, f, capture, SERIES[1]);
                complete &= whole(v.as_deref(), m.as_deref());
                let voice_start = span(&f.segs_of(capture, SERIES[1]).collect::<Vec<_>>(), f.offset_us).map_or(0.0, |(at, _)| (at - start_us) as f64 / 1e6);
                let picture = Media { parts: v.unwrap_or_default(), stream: "v:0".into(), start_s: 0.0 };
                parts.push(Part { start_us, end_us, picture, voice: Media { parts: m.unwrap_or_default(), stream: "a:0".into(), start_s: voice_start } });
            }
            parts.sort_by_key(|p| p.start_us);
            let Some(first) = parts.first() else { continue };
            out.push(Input { file, set: set.id.clone(), owner: f.owner.clone(), start_us: first.start_us, here: complete, parts });
        }
    }
    out
}

pub fn open(set: &Set, file: &str, me: &str, configs: &BTreeMap<String, Config>, tracks: &Tracks, chosen: &HashMap<String, String>, now: i64) -> bool {
    configs.iter().any(|(id, c)| c.runs_on(me) && counts(file, chosen) && due(set, id, tracks, now) && tracks.get(&(id.clone(), file.to_string())).is_none_or(|t| t.open_for(me).is_some()))
}

fn snapshot(core: &Core, room: &str) -> (Vec<Log>, BTreeMap<String, Config>, Tracks) {
    let logs = room::logs(core, room);
    let (configs, tracks) = (room_configs(core, room, &logs), tracks(&logs, None));
    (logs, configs, tracks)
}

fn room_configs(core: &Core, room: &str, logs: &[Log]) -> BTreeMap<String, Config> {
    runnable(configs(logs, None), &room_members(core, room))
}

pub fn runnable(mut configs: BTreeMap<String, Config>, members: &[String]) -> BTreeMap<String, Config> {
    for c in configs.values_mut() {
        c.runner = c.runner.take().filter(|r| members.contains(r));
    }
    configs
}

pub fn wanted_sets(core: &Core, room: &str, logs: &[Log], sets: &[Set]) -> HashSet<String> {
    let (me, configs, tracks, chosen, now) = (core.me(), room_configs(core, room, logs), tracks(logs, None), chosen(logs, None), now_us());
    let wanted = |set: &Set, f: &File| analyzable(f) && open(set, &format!("{}/{}", set.id, f.owner), &me, &configs, &tracks, &chosen, now);
    sets.iter().filter(|set| set.files.iter().any(|f| wanted(set, f))).map(|set| set.id.clone()).collect()
}

fn online(core: &Core) -> HashSet<String> {
    core.peers.lock().unwrap().iter().filter(|p| p.alive().is_some()).map(|p| p.id.to_string()).collect()
}

fn room_members(core: &Core, room: &str) -> Vec<String> {
    let me = core.me();
    let others: Vec<String> = core.room(room).map(|r| r.members.into_iter().filter(|m| *m != me).collect()).unwrap_or_default();
    std::iter::once(me).chain(others).collect()
}

pub fn worker(core: Arc<Core>) {
    loop {
        let room = {
            let mut work = WORK.lock().unwrap();
            let room = core.joined_room().ok();
            work.clone_from(&room);
            room
        };
        if let Some(room) = room {
            tick(&core, &room);
        }
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
    let (chosen, now) = (chosen(&logs, None), now_us());
    let due = |set: &Set, file: &str| open(set, file, &me, &configs, &tracks, &chosen, now);
    if let Some(wait) = gate(core.any_recording(), ffmpeg::present()) {
        let sets = room::sets(core, room, &logs);
        let files = sets.iter().flat_map(|set| set.files.iter().filter(|f| analyzable(f)).map(move |f| (set, format!("{}/{}", set.id, f.owner))));
        skipped(files.filter(|(set, file)| due(set, file)).map(|(_, file)| (file, reason(&wait))).collect());
        return;
    }
    let inputs = inputs(core, room, &logs, due);
    skipped(inputs.iter().filter(|i| !i.here).map(|i| (i.file.clone(), "files")).collect());
    let Some(pick) = next(&me, &configs, &tracks, &chosen, &inputs) else { return };
    let Some(plugin) = plugins::find(&pick.plugin) else { return };
    work(core, room, plugin, &pick.key, &inputs[pick.input]);
}

fn skipped(now: BTreeMap<String, &'static str>) {
    let mut before = SKIPPED.lock().unwrap();
    for (file, why) in now.iter().filter(|(file, why)| before.get(*file) != Some(*why)) {
        tracing::info!("analysis of {file} waits: {why}");
    }
    *before = now;
}

fn work(core: &Arc<Core>, room: &str, plugin: &'static Plugin, key: &str, input: &Input) {
    let run = now_us();
    let about = json!({"plugin": plugin.id, "version": plugin.version, "file": input.file});
    core.append_log(room, &job_record("claim", key, run, about.clone()));
    CANCEL.store(false, Ordering::Relaxed);
    tracing::info!("job {key} started on {}", input.file);
    *RUNNING.lock().unwrap() = Some(Running { room: room.into(), key: key.into(), plugin: plugin.id.into(), file: input.file.clone(), run, progress: 0.0 });
    match run_parts(core, room, plugin, input) {
        Some(Ok((found, tags))) => {
            tracing::info!("job {key} done with {} events", found.len());
            finish(core, room, plugin, key, run, input, &found, &tags)
        }
        Some(Err(error)) => {
            tracing::error!("job {key} failed: {error}");
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
    *RUNNING.lock().unwrap() = None;
}

type Results = (Vec<(i64, Found)>, BTreeMap<String, String>);

fn run_parts(core: &Arc<Core>, room: &str, plugin: &'static Plugin, input: &Input) -> Option<Result<Results, String>> {
    let mut found = Vec::new();
    let mut tags = BTreeMap::new();
    let count = input.parts.len().max(1) as f64;
    for (i, part) in input.parts.iter().enumerate() {
        let job = plugins::Job { plugin: plugin.id.into(), duration_s: (part.end_us - part.start_us) as f64 / 1e6, picture: part.picture.clone(), voice: part.voice.clone(), ffmpeg: ffmpeg::path() };
        match watch(core, room, plugin, &job, (i as f64 / count, 1.0 / count))? {
            Ok(output) => {
                found.extend(output.events.into_iter().map(|e| (part.start_us, e)));
                tags.extend(output.tags);
            }
            Err(e) => return Some(Err(e)),
        }
    }
    Some(Ok((found, tags)))
}

fn stale(core: &Core, room: &str, plugin: &Plugin) -> bool {
    let logs = room::logs(core, room);
    let runs = room_configs(core, room, &logs).get(plugin.id).is_some_and(|c| c.runs_on(&core.me()));
    core.check_joined(room).is_err() || core.any_recording() || !runs
}

fn watch(core: &Core, room: &str, plugin: &Plugin, job: &plugins::Job, share: (f64, f64)) -> Option<Result<Output, String>> {
    let mut process = match plugins::spawn(job) {
        Ok(c) => c,
        Err(e) => return Some(Err(e.to_string())),
    };
    let rx = plugins::replies(process.child.stdout.take().unwrap());
    let mut checked = Instant::now();
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
            }
            Ok(Reply::Done(output)) => {
                let _ = process.child.wait();
                return (!stale(core, room, plugin)).then_some(Ok(output));
            }
            Ok(Reply::Failed(e)) => {
                process.kill();
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

fn finish(core: &Arc<Core>, room: &str, plugin: &Plugin, key: &str, run: i64, input: &Input, found: &[(i64, Found)], tags: &BTreeMap<String, String>) {
    let mut id = 0;
    for (start_us, one) in found {
        id = now_us().max(id + 1);
        core.append_log(room, &event_record(plugin, key, run, input, *start_us, one, id));
    }
    let done = json!({"plugin": plugin.id, "version": plugin.version, "file": input.file, "events": found.len(), "tags": tags});
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
    let local = gate(core.any_recording(), ffmpeg::present());
    let tracks = tracks(logs, None);
    let phases = analysis(&core.me(), now_us(), &online(core), &configs, &tracks, &chosen(logs, None), sets, local.as_ref(), holders_of(core, mine));
    let running = RUNNING.lock().unwrap();
    phases
        .into_iter()
        .map(|(set, list)| {
            let mut json = analysis_json(&list, running.as_ref(), &set);
            if let Some(files) = sets.iter().find(|s| s.id == set).map(|s| &s.files) {
                for a in json.iter_mut().filter(|a| a["state"] == "done" || a["state"] == "failed") {
                    let plugin = a["plugin"].as_str().unwrap_or_default().to_string();
                    let at = files.iter().filter_map(|f| tracks.get(&(plugin.clone(), format!("{set}/{}", f.owner)))).filter_map(|t| t.latest.as_ref()).map(|l| l.at.0).max();
                    if let Some(at) = at {
                        a["at_us"] = json!(at);
                    }
                }
            }
            (set, json)
        })
        .collect()
}

pub fn room_analysis(core: &Core, room: &str, logs: &[Log]) -> HashMap<String, Vec<Value>> {
    let sets = room::sets(core, room, logs);
    analysis_by_set(core, room, logs, &sets, &data::have_counts(core, room, &sets))
}

pub fn plugins_json(core: &Core, room: &str) -> Vec<Value> {
    let logs = room::logs(core, room);
    let configs = runnable(every_config(&logs, None), &room_members(core, room));
    let by_set = room_analysis(core, room, &logs);
    let count = |id: &str, states: &[&str]| by_set.values().filter(|list| list.iter().any(|a| a["plugin"] == id && states.contains(&a["state"].as_str().unwrap_or_default()))).count();
    plugins::ALL
        .iter()
        .map(|p| {
            let config = configs.get(p.id);
            let waiting_for = by_set.values().flatten().find(|a| a["plugin"] == p.id && !a["waiting_for"].is_null()).map(|a| a["waiting_for"].clone());
            let mut v = plugins::manifest(p);
            v["on"] = json!(config.is_some_and(|c| c.on));
            v["runner"] = json!(config.and_then(|c| c.runner.clone()));
            v["settings"] = json!(config.map(|c| c.settings.clone()).unwrap_or_default());
            v["left"] = json!(count(p.id, &["left", "running"]));
            v["failed"] = json!(count(p.id, &["failed"]));
            v["waiting_for"] = waiting_for.unwrap_or(Value::Null);
            v
        })
        .collect()
}

pub fn room_plugins(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    Ok(json!(core.room_id().map(|room| plugins_json(core, &room)).unwrap_or_default()))
}

pub fn set_room_plugin(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let room = core.joined_room()?;
    let plugin = plugins::find(&arg::<String>(&args, "id")?).ok_or("Unknown extension.")?;
    let logs = room::logs(core, &room);
    let runner = plugin.analyzes().then(|| room_configs(core, &room, &logs).remove(plugin.id).and_then(|c| c.runner).unwrap_or_else(|| core.me()));
    let settings = every_config(&logs, None).remove(plugin.id).map(|c| c.settings).unwrap_or_default();
    let config = Config { on: arg(&args, "on")?, runner, settings };
    core.append_log(&room, &plugin_record(plugin, &config, change_time(now_us(), &logs, plugin.id)));
    Ok(Value::Null)
}

fn plugin_record(plugin: &Plugin, config: &Config, t_us: i64) -> Value {
    let mut record = json!({"k": "plugin", "plugin": plugin.id, "on": config.on, "version": plugin.version, "t_us": t_us});
    if let Some(runner) = &config.runner {
        record["runner"] = json!(runner);
    }
    if !config.settings.is_empty() {
        record["settings"] = json!(config.settings);
    }
    record
}

fn runner_record(plugin: &Plugin, runner: String, logs: &[Log], now: i64) -> Value {
    let (on, settings) = every_config(logs, None).remove(plugin.id).map_or((false, BTreeMap::new()), |c| (c.on, c.settings));
    plugin_record(plugin, &Config { on, runner: Some(runner), settings }, change_time(now, logs, plugin.id))
}

pub fn set_plugin_runner(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let room = core.joined_room()?;
    let plugin = plugins::find(&arg::<String>(&args, "id")?).filter(|p| p.analyzes()).ok_or("Unknown extension.")?;
    let logs = room::logs(core, &room);
    let runner: String = arg(&args, "runner")?;
    if !room_members(core, &room).contains(&runner) {
        return Err("Unknown member.".into());
    }
    core.append_log(&room, &runner_record(plugin, runner, &logs, now_us()));
    Ok(Value::Null)
}

pub fn set_plugin_setting(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let room = core.joined_room()?;
    let plugin = plugins::find(&arg::<String>(&args, "id")?).ok_or("Unknown extension.")?;
    let (setting, value): (String, String) = (arg(&args, "setting")?, arg(&args, "value")?);
    if !plugin.options.iter().any(|o| o.id == setting) {
        return Err("Unknown setting.".into());
    }
    if !value.is_empty() && !room_members(core, &room).contains(&value) {
        return Err("Unknown member.".into());
    }
    let logs = room::logs(core, &room);
    let mut config = every_config(&logs, None).remove(plugin.id).unwrap_or(Config { on: false, runner: None, settings: BTreeMap::new() });
    if value.is_empty() {
        config.settings.remove(&setting);
    } else {
        config.settings.insert(setting, value);
    }
    core.append_log(&room, &plugin_record(plugin, &config, change_time(now_us(), &logs, plugin.id)));
    Ok(Value::Null)
}

pub fn retries(configs: &BTreeMap<String, Config>, chosen: &HashMap<String, String>, set: &Set, plugin: Option<&str>) -> Vec<Value> {
    configs
        .iter()
        .filter(|(id, c)| c.on && plugin.is_none_or(|p| p == id.as_str()))
        .flat_map(|(id, _)| {
            let prefix = shipped(id);
            set.files.iter().map(move |f| format!("{}/{}", set.id, f.owner)).filter(move |file| counts(file, chosen)).map(move |file| job_record("retry", &format!("{prefix}{file}"), 0, Value::Null))
        })
        .collect()
}

pub fn retry_job(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let (room, set) = recording_ids(core, &args)?;
    let plugin: Option<String> = arg(&args, "plugin")?;
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
        std::thread::sleep(WAIT_STEP);
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

    const APEX: &str = "apex-matches";

    fn log(owner: &str, offset_us: i64, records: Vec<Value>) -> Log {
        Log { owner: owner.into(), offset_us, records: Arc::new(records) }
    }

    fn config_record(t: i64, on: bool) -> Value {
        json!({"k": "plugin", "plugin": APEX, "on": on, "version": "1.0.0", "t_us": t})
    }

    fn input(set: &str, owner: &str, start: i64, here: bool) -> Input {
        Input { file: format!("{set}/{owner}"), set: set.into(), owner: owner.into(), start_us: start, here, parts: vec![Part { start_us: start, end_us: start + 1000, ..Default::default() }] }
    }

    fn key(file: &str) -> String {
        format!("{}{file}", shipped(APEX))
    }

    fn older_key(file: &str) -> String {
        format!("{}{file}", prefix(APEX, "0.9.0"))
    }

    fn each_set(inputs: &[Input]) -> HashMap<String, String> {
        inputs.iter().map(|i| (i.set.clone(), i.file.clone())).collect()
    }

    fn pick(me: &str, logs: &[Log], inputs: &[Input]) -> Option<String> {
        next(me, &configs(logs, None), &tracks(logs, None), &each_set(inputs), inputs).map(|p| inputs[p.input].file.clone())
    }

    fn base_logs(extra_a: Vec<Value>, extra_b: Vec<Value>) -> Vec<Log> {
        let mut a = vec![config_record(1, true)];
        a.extend(extra_a);
        vec![log("a", 0, a), log("b", 0, extra_b)]
    }

    fn append(logs: &mut [Log], i: usize, record: Value) {
        Arc::make_mut(&mut logs[i].records).push(record);
    }

    fn timed(mut record: Value, t: i64) -> Value {
        record["t_us"] = json!(t);
        record
    }

    fn seg(set: &str) -> Value {
        json!({"k": "seg", "series": "v", "set": set, "n": 1, "capture": "1"})
    }

    fn ended(set: &str, owner: &str, end_us: Option<i64>) -> Set {
        let file = File { set: set.into(), owner: owner.into(), start_us: 0, end_us, offset_us: 0, segs: vec![Seg { capture: "1".into(), series: SERIES[0], n: 1, t_us: 0, d_us: 1, size: 1, sha256: String::new() }], caps: HashMap::new(), view: None, present: true };
        Set { id: set.into(), room: "r".into(), files: vec![file], stopped: true }
    }

    fn plugin_event(run: i64, id: i64) -> Value {
        json!({"k": "event", "id": id, "kind": "match_end", "plugin": {"id": APEX}, "run": run})
    }

    #[test]
    fn a_new_config_record_keeps_old_results_and_reruns_nothing() {
        let done = timed(job_record("done", &key("s1/a"), 3, json!({"tags": {"x": "X"}})), 10);
        let mut logs = base_logs(vec![seg("s1"), plugin_event(3, 11), done], vec![]);
        let inputs = [input("s1", "a", 10, true), input("s2", "a", 20, true)];
        assert_eq!(pick("a", &logs, &inputs).as_deref(), Some("s2/a"));
        let mut again = config_record(100, true);
        again["runner"] = json!("a");
        append(&mut logs, 1, again);
        assert_eq!(shown(&logs, None), HashSet::from([("a".to_string(), 3)]));
        assert_eq!(tag_names(&logs, None), BTreeMap::from([("x".to_string(), "X".to_string())]));
        assert_eq!(pick("a", &logs, &inputs[..1]), None);
        assert_eq!(pick("a", &logs, &inputs).as_deref(), Some("s2/a"));
        append(&mut logs, 0, config_record(200, false));
        assert!(shown(&logs, None).is_empty());
    }

    #[test]
    fn the_latest_config_wins_on_the_shared_clock() {
        let logs = vec![log("a", 0, vec![config_record(100, true)]), log("b", 50, vec![config_record(120, false)])];
        assert!(configs(&logs, None)[APEX].on);
        let logs = vec![log("a", 0, vec![config_record(100, true)]), log("b", -50, vec![config_record(120, false)])];
        assert!(!configs(&logs, None)[APEX].on);
    }

    #[test]
    fn a_view_extension_is_on_for_the_room_but_never_a_job() {
        let view = json!({"k": "plugin", "plugin": "apex-minimap", "on": true, "version": "1.0.0", "t_us": 1});
        let logs = vec![log("a", 0, vec![config_record(1, true), view])];
        assert!(every_config(&logs, None)["apex-minimap"].on);
        assert_eq!(configs(&logs, None).keys().collect::<Vec<_>>(), [APEX]);
        assert_eq!(plugins::manifest(plugins::find("apex-minimap").unwrap())["view"], "minimap");
        assert!(plugins::manifest(plugins::find(APEX).unwrap())["view"].is_null());
    }

    #[test]
    fn the_runner_defaults_to_who_turned_it_on() {
        let logs = vec![log("a", 0, vec![config_record(1, true)]), log("b", 0, vec![])];
        assert_eq!(configs(&logs, None)[APEX].runner.as_deref(), Some("a"));
        let by_b = vec![log("a", 0, vec![]), log("b", 0, vec![config_record(1, true)])];
        assert_eq!(configs(&by_b, None)[APEX].runner.as_deref(), Some("b"));
        let old_auto = json!({"k": "plugin", "plugin": APEX, "on": true, "version": "1.0.0", "settings": {}, "runner": "", "t_us": 2});
        let logs = vec![log("a", 0, vec![config_record(1, true)]), log("b", 0, vec![old_auto])];
        assert_eq!(configs(&logs, None)[APEX].runner.as_deref(), Some("b"));
        let chosen = json!({"k": "plugin", "plugin": APEX, "on": true, "version": "1.0.0", "runner": "a", "t_us": 3});
        let logs = vec![log("a", 0, vec![]), log("b", 0, vec![config_record(1, true), chosen])];
        assert_eq!(configs(&logs, None)[APEX].runner.as_deref(), Some("a"));
    }

    #[test]
    fn only_the_runner_processes_and_a_runner_who_left_is_no_runner() {
        let logs = base_logs(vec![], vec![]);
        let inputs = [input("s1", "a", 10, true)];
        assert_eq!(pick("a", &logs, &inputs).as_deref(), Some("s1/a"));
        assert_eq!(pick("b", &logs, &inputs), None);
        let members = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect::<Vec<_>>();
        assert_eq!(runnable(configs(&logs, None), &members(&["a", "b"]))[APEX].runner.as_deref(), Some("a"));
        assert_eq!(runnable(configs(&logs, None), &members(&["b", "c"]))[APEX].runner, None);
    }

    #[test]
    fn changing_the_runner_keeps_it_on_and_stamps_the_shipped_version() {
        let plugin = plugins::find(APEX).unwrap();
        let mut logs = base_logs(vec![], vec![]);
        let record = runner_record(plugin, "b".into(), &logs, 100);
        assert_eq!(record["version"], json!(plugin.version));
        append(&mut logs, 1, record);
        let config = &configs(&logs, None)[APEX];
        assert_eq!(config, &Config { on: true, runner: Some("b".into()), settings: BTreeMap::new() });
        let fresh = runner_record(plugin, "b".into(), &[], 100);
        assert_eq!((fresh["on"].clone(), fresh["version"].clone(), fresh["settings"].clone()), (json!(false), json!(plugin.version), Value::Null));
    }

    #[test]
    fn the_runner_wants_every_finished_file_of_an_enabled_extension() {
        let logs = base_logs(vec![], vec![]);
        let all = configs(&logs, None);
        assert!(runner_wants("a", &all));
        assert!(!runner_wants("b", &all));
        let off = vec![log("a", 0, vec![config_record(1, true), config_record(2, false)])];
        assert!(!runner_wants("a", &configs(&off, None)));
        assert!(!runner_wants("a", &BTreeMap::new()));
    }

    #[test]
    fn the_runner_wants_only_files_that_still_need_analysis() {
        let done = timed(job_record("done", &key("s1/a"), 3, Value::Null), 10);
        let failed = timed(job_record("fail", &key("s2/a"), 4, Value::Null), 11);
        let logs = base_logs(vec![done, failed], vec![]);
        let chosen = each_set(&[input("s1", "a", 1, true), input("s2", "a", 2, true), input("s3", "a", 3, true)]);
        let (configs, tracks) = (configs(&logs, None), tracks(&logs, None));
        let open = |file: &str, me: &str| open(&ended(&file[..2], &file[3..], Some(0)), file, me, &configs, &tracks, &chosen, 1);
        assert!(!open("s1/a", "a"));
        assert!(!open("s2/a", "a"));
        assert!(open("s3/a", "a"));
        assert!(!open("s3/a", "b"));
        assert!(!open("s3/b", "a"));
    }

    #[test]
    fn only_a_recording_that_ended_within_the_hour_or_was_already_started_is_analysed_on_its_own() {
        let logs = base_logs(vec![], vec![]);
        let (configs, chosen) = (configs(&logs, None), each_set(&[input("s1", "a", 1, true)]));
        let hour = AUTO_ANALYSIS_US;
        let set = ended("s1", "a", Some(10));
        assert!(ended_recently(&set, 10 + hour - 1) && !ended_recently(&set, 10 + hour));
        assert!(!ended_recently(&ended("s1", "a", None), 10));
        let open_at = |logs: &[Log], now: i64| open(&set, "s1/a", "a", &configs, &tracks(logs, None), &chosen, now);
        assert!(open_at(&logs, 10 + hour - 1));
        assert!(!open_at(&logs, 10 + hour));
        let asked = base_logs(vec![timed(job_record("retry", &key("s1/a"), 0, Value::Null), 5)], vec![]);
        assert!(open_at(&asked, 10 + hour));
        let started = base_logs(vec![timed(job_record("claim", &key("s1/a"), 5, Value::Null), 5)], vec![]);
        assert!(open_at(&started, 10 + hour));
        let holders = |_: &Set, f: &File| vec![Holder { id: f.owner.clone(), online: true }];
        let run = |logs: &[Log], now: i64| analysis("a", now, &HashSet::new(), &configs, &tracks(logs, None), &chosen, std::slice::from_ref(&set), None, holders)["s1"].clone();
        assert_eq!(run(&logs, 10 + hour), vec![(APEX.to_string(), Phase::Idle)]);
        assert_eq!(run(&logs, 10), vec![(APEX.to_string(), Phase::Left(Wait::Queue))]);
        assert_eq!(run(&asked, 10 + hour), vec![(APEX.to_string(), Phase::Left(Wait::Queue))]);
        assert_eq!(analysis_json(&[(APEX.to_string(), Phase::Idle)], None, "s1"), vec![json!({"plugin": APEX, "state": "idle"})]);
    }

    #[test]
    fn a_recording_with_one_file_running_reads_running_and_a_failure_outranks_it() {
        let left = Phase::Left(Wait::Queue);
        assert_eq!(fold(vec![Phase::Running, left.clone(), left.clone()]), Phase::Running);
        assert_eq!(fold(vec![left.clone(), Phase::Running, Phase::Failed("x".into())]), Phase::Failed("x".into()));
        assert_eq!(fold(vec![Phase::Done, left.clone()]), left);
    }

    #[test]
    fn stop_and_retry_records_read_as_pending() {
        let inputs = [input("s1", "a", 10, true)];
        for state in ["stop", "retry"] {
            let logs = base_logs(vec![timed(job_record("claim", &key("s1/a"), 5, Value::Null), 5), timed(job_record(state, &key("s1/a"), 5, Value::Null), 6)], vec![]);
            assert_eq!(pick("a", &logs, &inputs).as_deref(), Some("s1/a"), "{state}");
            let t = tracks(&logs, None);
            assert_eq!(phase(t.get(&(APEX.into(), "s1/a".into())), Some("a"), "a", &HashSet::new(), &[Holder { id: "a".into(), online: true }], None), Phase::Left(Wait::Queue));
        }
    }

    #[test]
    fn a_file_nobody_online_holds_fails_as_missing_and_only_an_offline_runner_waits() {
        let t = Track::default();
        let online = HashSet::from(["a".to_string()]);
        let held_by_b = [Holder { id: "b".into(), online: false }];
        let file = (APEX.to_string(), "s1/b".to_string());
        assert_eq!(phase(Some(&t), Some("a"), "a", &online, &held_by_b, None), Phase::Missing);
        assert_eq!(phase(None, Some("a"), "a", &online, &[], None), Phase::Missing);
        let retried = base_logs(vec![timed(job_record("retry", &key("s1/b"), 5, Value::Null), 5)], vec![]);
        assert_eq!(phase(tracks(&retried, None).get(&file), Some("a"), "a", &online, &held_by_b, None), Phase::Missing);
        assert_eq!(phase(None, Some("c"), "a", &online, &[], None), Phase::Left(Wait::Member("c".into())));
        let held_online = [Holder { id: "c".into(), online: true }, Holder { id: "b".into(), online: false }];
        assert_eq!(phase(None, Some("a"), "a", &online, &held_online, None), Phase::Left(Wait::Receiving));
        let held_here = [Holder { id: "a".into(), online: true }];
        assert_eq!(phase(None, Some("a"), "a", &online, &held_here, None), Phase::Left(Wait::Queue));
        assert_eq!(phase(None, Some("b"), "a", &online, &held_here, None), Phase::Left(Wait::Member("b".into())));
        assert_eq!(phase(None, None, "a", &online, &held_here, None), Phase::Left(Wait::NoRunner));
        let logs = base_logs(vec![timed(job_record("claim", &key("s1/b"), 5, Value::Null), 5)], vec![]);
        let t = tracks(&logs, None);
        assert_eq!(phase(t.get(&file), Some("a"), "a", &online, &held_here, None), Phase::Running);
        assert_eq!(phase(t.get(&file), Some("a"), "b", &HashSet::new(), &held_here, None), Phase::Left(Wait::Member("a".into())));
        let failed = base_logs(vec![timed(job_record("fail", &key("s1/b"), 5, json!({"error": "x"})), 5)], vec![]);
        assert_eq!(phase(tracks(&failed, None).get(&file), Some("a"), "a", &online, &held_here, None), Phase::Failed("x".into()));
        let done = base_logs(vec![timed(job_record("done", &key("s1/b"), 5, Value::Null), 5)], vec![]);
        assert_eq!(phase(tracks(&done, None).get(&file), Some("a"), "a", &online, &held_here, None), Phase::Done);
    }

    #[test]
    fn a_recording_shows_its_chosen_file_and_only_enabled_extensions() {
        let seg = |set: &str, n: u32| json!({"k": "seg", "series": "v", "set": set, "n": n, "capture": "1", "t_us": n, "d_us": 1, "size": 1, "sha256": "x"});
        let rec = |set: &str| json!({"k": "rec", "set": set, "start_us": 1});
        let end = |set: &str| json!({"k": "rec_end", "set": set, "t_us": 9});
        let voice = json!({"k": "seg", "series": "m", "set": "d1", "n": 1, "capture": "1", "t_us": 1, "d_us": 1, "size": 1, "sha256": "x"});
        let logs = vec![log("a", 0, vec![config_record(1, true), rec("s1"), seg("s1", 1), end("s1"), timed(job_record("done", &key("s1/a"), 3, Value::Null), 5), rec("d1"), voice, end("d1")]), log("b", 0, vec![rec("s1"), seg("s1", 1), seg("s1", 2), end("s1")])];
        let sets = room::derive_sets("r", &logs);
        let holders = |_: &Set, f: &File| vec![Holder { id: f.owner.clone(), online: true }];
        let run = |logs: &[Log]| analysis("a", 9, &HashSet::from(["b".to_string()]), &configs(logs, None), &tracks(logs, None), &chosen(logs, None), &sets, None, &holders);
        let out = run(&logs);
        assert_eq!(out["s1"], vec![(APEX.to_string(), Phase::Left(Wait::Receiving))]);
        assert!(!out.contains_key("d1"));
        let mut done = logs;
        append(&mut done, 0, timed(job_record("done", &key("s1/b"), 4, Value::Null), 6));
        assert_eq!(run(&done)["s1"], vec![(APEX.to_string(), Phase::Done)]);
        append(&mut done, 0, config_record(10, false));
        assert!(run(&done).is_empty());
    }

    #[test]
    fn a_change_made_here_beats_a_record_from_a_fast_clock() {
        let ahead = log("b", 0, vec![config_record(5_000, true)]);
        let t = change_time(1_000, &[ahead], APEX);
        assert_eq!(t, 5_001);
        let logs = vec![log("a", 0, vec![config_record(t, false)]), log("b", 0, vec![config_record(5_000, true)])];
        assert!(!configs(&logs, None)[APEX].on);
        assert_eq!(change_time(9_000, &logs[1..], APEX), 9_000);
        assert_eq!(change_time(7, &[], APEX), 7);
    }

    #[test]
    fn keys_hash_the_empty_settings_of_older_versions_and_change_with_the_version() {
        assert_eq!(prefix(APEX, "1.0.0"), "apex-matches@1.0.0/44136fa355b3/");
        assert_ne!(prefix(APEX, "1.0.0"), prefix(APEX, "1.1.0"));
        assert_eq!(split_key(&key("s1/a")), Some((APEX.to_string(), "s1/a".to_string())));
        assert_eq!(split_key("s1/a"), None);
    }

    #[test]
    fn the_newest_recording_comes_first_and_an_interrupted_own_claim_before_it() {
        let logs = base_logs(vec![], vec![]);
        let inputs = [input("s1", "b", 10, true), input("s2", "b", 20, true), input("s0", "a", 1, true)];
        assert_eq!(pick("a", &logs, &inputs).as_deref(), Some("s2/b"));
        assert_eq!(pick("a", &logs, &[input("s1", "b", 10, false)]), None);
        let claimed = base_logs(vec![timed(job_record("claim", &key("s0/a"), 5, Value::Null), 5)], vec![]);
        assert_eq!(pick("a", &claimed, &inputs).as_deref(), Some("s0/a"));
    }

    #[test]
    fn nothing_runs_when_off() {
        let inputs = [input("s1", "a", 10, true)];
        let off = vec![log("a", 0, vec![config_record(1, true), config_record(2, false)])];
        assert_eq!(pick("a", &off, &inputs), None);
    }

    #[test]
    fn a_built_in_extension_runs_the_shipped_version_whatever_the_room_recorded() {
        let shipped_version = plugins::find(APEX).unwrap().version;
        assert_ne!(shipped_version, "1.0.0");
        let inputs = [input("s1", "a", 10, true), input("s2", "a", 20, true)];
        for version in ["1.0.0", "2.0.0", ""] {
            let mut config = config_record(1, true);
            config["version"] = json!(version);
            let logs = vec![log("a", 0, vec![config]), log("b", 0, vec![])];
            let pick = next("a", &configs(&logs, None), &tracks(&logs, None), &each_set(&inputs), &inputs).unwrap();
            assert_eq!(pick.key, format!("apex-matches@{shipped_version}/44136fa355b3/s2/a"), "{version}");
        }
        let old_done = timed(job_record("done", &format!("{}s2/a", prefix(APEX, "1.0.0")), 3, Value::Null), 5);
        let logs = base_logs(vec![seg("s2"), old_done], vec![]);
        assert_eq!(pick("a", &logs, &inputs).as_deref(), Some("s1/a"));
        assert_eq!(shown(&logs, None).len(), 1);
    }

    #[test]
    fn an_extension_setting_is_read_from_the_log_and_kept_by_the_other_changes() {
        let plugin = plugins::find("apex-minimap").unwrap();
        let mut logs = base_logs(vec![], vec![]);
        let config = Config { on: true, runner: None, settings: BTreeMap::from([("source".to_string(), "b".to_string())]) };
        append(&mut logs, 1, plugin_record(plugin, &config, 100));
        assert_eq!(every_config(&logs, None)["apex-minimap"].settings, config.settings);
        append(&mut logs, 1, plugin_record(plugin, &Config { settings: BTreeMap::new(), ..config }, 200));
        assert!(every_config(&logs, None)["apex-minimap"].settings.is_empty());
        let moved = runner_record(plugin, "a".into(), &logs, 300);
        assert_eq!(moved.get("settings"), None);
    }

    #[test]
    fn turning_an_extension_on_stamps_the_shipped_version() {
        let plugin = plugins::find(APEX).unwrap();
        let record = plugin_record(plugin, &Config { on: true, runner: Some("a".into()), settings: BTreeMap::new() }, 1);
        assert_eq!(record, json!({"k": "plugin", "plugin": APEX, "on": true, "version": plugin.version, "runner": "a", "t_us": 1}));
    }

    #[test]
    fn a_waiting_file_tells_what_holds_it_with_the_checks_the_runner_uses() {
        assert_eq!(gate(true, true), Some(Wait::Recording));
        assert_eq!(gate(true, false), Some(Wait::Recording));
        assert_eq!(gate(false, false), Some(Wait::Ffmpeg));
        assert_eq!(gate(false, true), None);
        let online = HashSet::from(["b".to_string()]);
        let here = [Holder { id: "a".into(), online: true }];
        let at = |runner: &str, me: &str, holders: &[Holder], local: Option<Wait>| phase(None, Some(runner), me, &online, holders, local.as_ref());
        assert_eq!(at("a", "a", &here, None), Phase::Left(Wait::Queue));
        assert_eq!(at("a", "a", &here, Some(Wait::Recording)), Phase::Left(Wait::Recording));
        assert_eq!(at("a", "a", &here, Some(Wait::Ffmpeg)), Phase::Left(Wait::Ffmpeg));
        assert_eq!(at("a", "a", &[], Some(Wait::Recording)), Phase::Left(Wait::Recording));
        assert_eq!(at("b", "a", &here, Some(Wait::Recording)), Phase::Left(Wait::Receiving));
        assert_eq!(at("c", "a", &here, Some(Wait::Recording)), Phase::Left(Wait::Member("c".into())));
        let logs = base_logs(vec![timed(job_record("done", &key("s1/a"), 5, Value::Null), 5)], vec![]);
        assert_eq!(phase(tracks(&logs, None).get(&(APEX.into(), "s1/a".into())), Some("a"), "a", &online, &here, Some(&Wait::Recording)), Phase::Done);
        let json = |wait: Wait| analysis_json(&[(APEX.to_string(), Phase::Left(wait))], None, "s1")[0]["reason"].clone();
        assert_eq!([json(Wait::Queue), json(Wait::Recording), json(Wait::Ffmpeg), json(Wait::Receiving), json(Wait::NoRunner), json(Wait::Member("c".into()))], [json!("queue"), json!("recording"), json!("ffmpeg"), json!("receiving"), json!("no_runner"), json!("waiting")]);
    }

    #[test]
    fn an_input_is_ready_only_with_its_whole_picture_and_voice() {
        let piece = [PathBuf::from("000001.m4s")];
        assert!(whole(Some(&piece), Some(&piece)));
        assert!(whole(Some(&piece), Some(&[])));
        assert!(!whole(Some(&piece), None));
        assert!(!whole(Some(&[]), Some(&piece)));
        assert!(!whole(None, Some(&piece)));
    }

    #[test]
    fn done_and_failed_files_wait_for_a_retry() {
        let inputs = [input("s1", "a", 10, true)];
        let done = base_logs(vec![timed(job_record("done", &key("s1/a"), 3, Value::Null), 5)], vec![]);
        assert_eq!(pick("a", &done, &inputs), None);
        let mut failed = base_logs(vec![seg("s1"), timed(job_record("claim", &key("s1/a"), 3, Value::Null), 4), timed(job_record("fail", &key("s1/a"), 3, json!({"error": "x"})), 5)], vec![]);
        assert_eq!(pick("a", &failed, &inputs), None);
        append(&mut failed, 1, timed(job_record("retry", &key("s1/a"), 0, Value::Null), 6));
        assert_eq!(pick("a", &failed, &inputs).as_deref(), Some("s1/a"));
        append(&mut failed, 0, timed(job_record("done", &older_key("s1/a"), 7, Value::Null), 7));
        assert_eq!(pick("a", &failed, &inputs), None);
        append(&mut failed, 1, timed(job_record("retry", &key("s1/a"), 0, Value::Null), 8));
        assert_eq!(pick("a", &failed, &inputs).as_deref(), Some("s1/a"));
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
        assert_eq!(pick("a", &stopped, &[input("s1", "a", 10, true)]).as_deref(), Some("s1/a"));
    }

    #[test]
    fn a_plugin_reads_the_longest_file_once() {
        let seg = |set: &str, n: u32, capture: &str| json!({"k": "seg", "series": "v", "set": set, "n": n, "capture": capture});
        let logs = vec![log("a", 0, vec![seg("s1", 1, "1"), seg("s1", 2, "1"), seg("s2", 1, "3")]), log("b", 0, vec![seg("s1", 1, "2"), seg("s1", 2, "2"), seg("s1", 3, "2"), seg("s2", 1, "4")])];
        let chosen = chosen(&logs, None);
        assert_eq!(chosen["s1"], "s1/b");
        assert_eq!(chosen["s2"], "s2/a");
        assert!(counts("s1/b", &chosen) && !counts("s1/a", &chosen));
    }

    #[test]
    fn restarted_captures_of_one_person_are_one_file() {
        let seg = |n: u32, capture: &str| json!({"k": "seg", "series": "v", "set": "s1", "n": n, "capture": capture});
        let logs = vec![log("a", 0, vec![seg(1, "1"), seg(2, "1"), seg(1, "2")]), log("b", 0, vec![seg(1, "3"), seg(2, "3")])];
        assert_eq!(chosen(&logs, None)["s1"], "s1/a");
        assert!(!counts("s1/a/1", &HashMap::new()));
        assert!(!counts("s1", &HashMap::new()));
    }

    #[test]
    fn results_of_a_plugin_the_app_does_not_know_are_not_shown() {
        let seg = json!({"k": "seg", "series": "v", "set": "s1", "n": 1, "capture": "1"});
        let old = json!({"k": "plugin", "plugin": "gone", "on": true, "version": "1.1.0", "t_us": 1});
        let done = json!({"k": "job", "state": "done", "job": "gone@1.1.0/0123456789ab/s1/a", "run": 7, "t_us": 9});
        let logs = vec![log("a", 0, vec![old, seg, done])];
        let mut events = vec![json!({"author": "a", "run": 7, "plugin": {"id": "gone"}}), json!({"author": "a", "run": 0, "plugin": null})];
        keep_shown(&mut events, &logs, None);
        assert_eq!(events, vec![json!({"author": "a", "run": 0, "plugin": null})]);
        assert!(configs(&logs, None).is_empty());
    }

    #[test]
    fn a_plugin_the_app_does_not_know_runs_nothing_and_shows_no_analysis() {
        let old = json!({"k": "plugin", "plugin": "gone", "on": true, "version": "1.1.0", "runner": "a", "t_us": 1});
        let rec = json!({"k": "rec", "set": "s1", "start_us": 1});
        let seg = json!({"k": "seg", "series": "v", "set": "s1", "n": 1, "capture": "1", "t_us": 1, "d_us": 1, "size": 1, "sha256": "x"});
        let logs = vec![log("a", 0, vec![old, rec, seg])];
        let configs = runnable(configs(&logs, None), &["a".to_string()]);
        let (tracks, chosen) = (tracks(&logs, None), chosen(&logs, None));
        let sets = room::derive_sets("r", &logs);
        assert!(!open(&ended("s1", "a", Some(0)), "s1/a", "a", &configs, &tracks, &chosen, 1));
        assert!(analysis("a", 1, &HashSet::new(), &configs, &tracks, &chosen, &sets, None, |_, _| vec![]).is_empty());
    }

    #[test]
    fn a_file_done_twice_shows_the_newest_result_and_ties_go_to_the_lowest_owner() {
        let k = key("s1/a");
        let a = vec![seg("s1"), plugin_event(1, 10), timed(job_record("done", &k, 1, json!({"tags": {"x": "X"}})), 50), plugin_event(2, 11)];
        let b = vec![plugin_event(4, 20), timed(job_record("done", &older_key("s1/a"), 4, Value::Null), 60)];
        let logs = base_logs(a, b);
        assert_eq!(shown(&logs, None), HashSet::from([("b".to_string(), 4)]));
        let tie = base_logs(vec![seg("s1"), timed(job_record("done", &k, 1, Value::Null), 50)], vec![timed(job_record("done", &k, 4, Value::Null), 50)]);
        assert_eq!(shown(&tie, None), HashSet::from([("a".to_string(), 1)]));
    }

    #[test]
    fn results_wait_for_the_done_record_within_caps() {
        let logs = base_logs(vec![seg("s1"), plugin_event(1, 10), job_record("done", &key("s1/a"), 1, Value::Null)], vec![]);
        let caps = HashMap::from([("a".to_string(), 3u64)]);
        assert!(shown(&logs, Some(&caps)).is_empty());
        let caps = HashMap::from([("a".to_string(), 4u64)]);
        assert_eq!(shown(&logs, Some(&caps)).len(), 1);
    }

    #[test]
    fn re_running_asks_every_enabled_extension_or_the_one_named_for_each_counted_file() {
        let seg = |set: &str| json!({"k": "seg", "series": "v", "set": set, "n": 1, "capture": "1", "t_us": 1, "d_us": 1, "size": 1, "sha256": "x"});
        let rec = |set: &str| json!({"k": "rec", "set": set, "start_us": 1});
        let logs = vec![log("a", 0, vec![config_record(1, true), rec("s1"), seg("s1")]), log("b", 0, vec![rec("s1"), seg("s1")])];
        let sets = room::derive_sets("r", &logs);
        let all = retries(&configs(&logs, None), &chosen(&logs, None), &sets[0], None);
        assert_eq!(all.iter().map(|r| r["job"].as_str().unwrap().to_string()).collect::<Vec<_>>(), vec![key("s1/a")]);
        assert!(all.iter().all(|r| r["state"] == "retry"));
        assert!(retries(&configs(&logs, None), &chosen(&logs, None), &sets[0], Some("gone")).is_empty());
        assert_eq!(retries(&configs(&logs, None), &chosen(&logs, None), &sets[0], Some(APEX)).len(), 1);
    }

    #[test]
    fn a_retry_after_a_failure_on_an_older_version_runs_the_shipped_one() {
        let failed = timed(job_record("fail", &format!("{}s1/a", prefix(APEX, "1.0.0")), 3, json!({"error": "x"})), 5);
        let mut logs = base_logs(vec![seg("s1"), failed], vec![]);
        let inputs = [input("s1", "a", 10, true)];
        assert_eq!(pick("a", &logs, &inputs), None);
        let set = ended("s1", "a", Some(0));
        for record in retries(&configs(&logs, None), &each_set(&inputs), &set, Some(APEX)) {
            assert_eq!(record["job"], json!(key("s1/a")));
            append(&mut logs, 1, timed(record, 6));
        }
        assert_eq!(pick("a", &logs, &inputs).as_deref(), Some("s1/a"));
    }

    #[test]
    fn results_land_on_the_runners_clock() {
        let i = input("s1", "b", 1_000_000, true);
        let at = Found { kind: "match_end".into(), at_s: Some(2.5), ..Default::default() };
        assert_eq!(target(&at, &i, i.start_us), json!({"t_us": 3_500_000, "set": "s1", "file": "b"}));
        assert_eq!(target(&at, &i, 9_000_000), json!({"t_us": 11_500_000, "set": "s1", "file": "b"}));
        assert_eq!(target(&Found::default(), &i, i.start_us), json!({"set": "s1", "file": "b"}));
        let r = event_record(plugins::find(APEX).unwrap(), "k", 7, &i, i.start_us, &at, 42);
        assert_eq!((r["k"].clone(), r["run"].clone(), r["plugin"]["name"].clone()), (json!("event"), json!(7), json!("Apex match detection")));
    }

    #[test]
    fn a_capture_spans_its_segments_on_the_local_clock() {
        let seg = |n, t_us| Seg { capture: "1".into(), series: "v", n, t_us, d_us: 4, size: 1, sha256: String::new() };
        let (a, b) = (seg(1, 100), seg(2, 104));
        assert_eq!(span(&[&a, &b], 10), Some((90, 98)));
        assert_eq!(span(&[], 0), None);
    }

    #[test]
    fn room_events_hide_unfinished_plugin_runs() {
        let mut e = plugin_event(1, 10);
        e["target"] = json!({"t_us": 50});
        let logs = base_logs(vec![seg("s1"), e.clone()], vec![]);
        assert!(room::events(&logs, None).is_empty());
        let logs = base_logs(vec![seg("s1"), e, job_record("done", &key("s1/a"), 1, Value::Null)], vec![]);
        assert_eq!(room::events(&logs, None).len(), 1);
    }
}

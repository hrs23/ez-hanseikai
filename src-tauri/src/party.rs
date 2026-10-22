use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::{net, room, Core};

pub const REPEAT: Duration = Duration::from_secs(4);
const SEEN: Duration = Duration::from_secs(10);
const CHECK: Duration = Duration::from_secs(1);
const SHARED: [&str; 10] = ["set", "playing", "live", "wall_us", "at_us", "rate", "focus", "mode", "slots", "order"];

pub type Heads = HashMap<String, u64>;

pub struct Viewer {
    pub set: Value,
    pub heads: Heads,
    pub typing: bool,
    pub seen: Instant,
}

#[derive(Default)]
pub struct Party {
    pub on: bool,
    pub room: String,
    pub lamport: u64,
    pub shared: Value,
    pub heads: Heads,
    pub viewers: HashMap<String, Viewer>,
    pub typing: bool,
    grown: HashMap<String, Instant>,
    sent: Option<(Instant, Heads)>,
}

pub fn key(v: &Value) -> (u64, i64, String) {
    (v["seq"].as_u64().unwrap_or(0), v["stamp"].as_i64().unwrap_or(0), v["origin"].as_str().unwrap_or_default().to_string())
}

fn heads_of(value: &Value) -> Heads {
    value.as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_u64()?))).collect()
}

fn to_local(shared: &mut Value, offset_us: i64) {
    for k in ["wall_us", "at_us"] {
        if let Some(t) = shared[k].as_i64() {
            shared[k] = json!(t - offset_us);
        }
    }
}

fn shared_from(view: &Value, seq: u64, stamp: i64, origin: &str) -> Value {
    let mut shared = json!({"seq": seq, "stamp": stamp, "origin": origin});
    for k in SHARED {
        shared[k] = view[k].clone();
    }
    shared
}

impl Party {
    fn enter(&mut self, room: &str) {
        if self.room != room {
            self.room = room.to_string();
            self.heads.clear();
            self.viewers.clear();
            self.grown.clear();
        }
    }

    fn absorb(&mut self, heads: &Heads, now: Instant) {
        for (owner, n) in heads {
            if self.heads.get(owner).is_none_or(|old| n > old) {
                self.grown.insert(owner.clone(), now);
                self.heads.insert(owner.clone(), *n);
            }
        }
    }

    pub fn turn(&mut self, on: bool, me: &str, view: &Value, room: &str, now_us: i64) {
        self.on = on;
        self.sent = None;
        if on {
            self.enter(room);
            self.shared = shared_from(view, 0, -now_us, me);
        }
    }

    pub fn act(&mut self, me: &str, view: &Value, now_us: i64) {
        if self.on {
            self.lamport += 1;
            self.shared = shared_from(view, self.lamport, now_us, me);
        }
    }

    pub fn receive(&mut self, from: &str, msg: &Value, offset_us: i64, room: &str, now: Instant) -> bool {
        if msg["room"] != room {
            return false;
        }
        self.enter(room);
        let mut shared = msg["shared"].clone();
        let heads = heads_of(&msg["heads"]);
        self.viewers.insert(from.to_string(), Viewer { set: shared["set"].clone(), heads: heads.clone(), typing: msg["typing"] == true, seen: now });
        if !self.on {
            return false;
        }
        self.lamport = self.lamport.max(key(&shared).0);
        self.absorb(&heads, now);
        if !self.shared["set"].is_string() || shared["set"] != self.shared["set"] || key(&shared) <= key(&self.shared) {
            return false;
        }
        to_local(&mut shared, offset_us);
        self.shared = shared;
        true
    }

    pub fn caps(&self) -> Option<Heads> {
        self.on.then(|| self.heads.clone())
    }

    pub fn lacks(&self, own: &Heads) -> bool {
        self.heads.iter().any(|(owner, n)| own.get(owner).copied().unwrap_or(0) < *n)
    }

    pub fn due(&self, own: &Heads, now: Instant) -> bool {
        self.on && self.sent.as_ref().is_none_or(|(at, heads)| now.duration_since(*at) >= REPEAT || heads != own)
    }

    pub fn message(&mut self, room: &str, own: &Heads, now: Instant) -> Value {
        self.enter(room);
        self.absorb(own, now);
        self.sent = Some((now, own.clone()));
        json!({"t": "party", "room": self.room, "shared": self.shared, "heads": own, "typing": self.typing})
    }

    pub fn status(&self, own: &Heads, now: Instant) -> Vec<Value> {
        let mut viewers: Vec<&Heads> = vec![own];
        viewers.extend(self.seen(now).into_iter().map(|(_, v)| &v.heads));
        let mut owners: Vec<&String> = self.heads.keys().collect();
        owners.sort();
        let mut out = Vec::new();
        for owner in owners {
            let settled = self.grown.get(owner).is_none_or(|at| now.duration_since(*at) >= REPEAT);
            if settled && viewers.iter().any(|heads| heads.get(owner).copied().unwrap_or(0) < self.heads[owner]) {
                out.push(json!({"kind": "catching", "who": owner}));
            }
        }
        out
    }

    fn seen(&self, now: Instant) -> Vec<(&String, &Viewer)> {
        let mut out: Vec<(&String, &Viewer)> = self.viewers.iter().filter(|(_, v)| now.duration_since(v.seen) < SEEN).collect();
        out.sort_by_key(|(id, _)| id.as_str());
        out
    }

    pub fn typists(&self, now: Instant) -> Vec<String> {
        self.seen(now).into_iter().filter(|(_, v)| v.typing).map(|(id, _)| id.clone()).collect()
    }

    pub fn watching(&self, own: &str, now: Instant) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let me = self.on.then_some((own, &self.shared["set"]));
        let seen = self.seen(now).into_iter().map(|(id, v)| (id.as_str(), &v.set));
        for (id, set) in seen.chain(me).filter_map(|(id, set)| Some((id, set.as_str()?))) {
            out.entry(set.to_string()).or_default().push(id.to_string());
        }
        out.values_mut().for_each(|ids| ids.sort());
        out
    }

    pub fn open_set(&self) -> Option<String> {
        self.shared["set"].as_str().filter(|_| self.on).map(String::from)
    }

    pub fn reviewing(&self) -> bool {
        self.open_set().is_some()
    }
}

pub fn record(set: &str, shared: &Value, t_us: i64) -> Value {
    json!({"k": "party", "set": set, "t_us": t_us, "shared": shared})
}

pub fn log(core: &Core) {
    let Some(active) = core.active_set.lock().unwrap().clone() else { return };
    let shared = {
        let party = core.party.lock().unwrap();
        if !party.on {
            return;
        }
        party.shared.clone()
    };
    core.append_log(&active.room, &record(&active.id, &shared, crate::now_us()));
}

pub fn records(logs: &[room::Log]) -> HashMap<String, Vec<Value>> {
    let mut out: HashMap<String, Vec<Value>> = HashMap::new();
    for log in logs {
        for r in log.records.iter().filter(|r| r["k"] == "party") {
            let (Some(set), Some(t), Some(shared)) = (r["set"].as_str(), r["t_us"].as_i64(), r["shared"].as_object()) else { continue };
            let mut shared = Value::Object(shared.clone());
            to_local(&mut shared, log.offset_us);
            out.entry(set.to_string()).or_default().push(json!({"t_us": t - log.offset_us, "by": log.owner, "shared": shared}));
        }
    }
    for list in out.values_mut() {
        list.sort_by_key(|r| r["t_us"].as_i64());
    }
    out
}

pub fn watch_json(records: &[Value]) -> Value {
    records.iter().map(|r| json!({"t_us": r["t_us"], "by": r["by"], "set": r["shared"]["set"], "at_us": r["shared"]["at_us"], "playing": r["shared"]["playing"], "rate": r["shared"]["rate"]})).collect()
}

pub fn send(core: &Arc<Core>) {
    if !core.party.lock().unwrap().on {
        return;
    }
    let Some((room, own)) = own_heads(core) else { return };
    let message = core.party.lock().unwrap().message(&room, &own, Instant::now());
    net::broadcast(core, message);
}

fn own_heads(core: &Core) -> Option<(String, Heads)> {
    let room = core.room_id()?;
    let own = room::holdings(&room::logs(core, &room));
    Some((room, own))
}

pub fn receive(core: &Arc<Core>, from: &str, msg: &Value, offset_us: i64) -> bool {
    let Some((room, own)) = own_heads(core) else { return false };
    let (changed, lacks) = {
        let mut party = core.party.lock().unwrap();
        let changed = party.receive(from, msg, offset_us, &room, Instant::now());
        (changed, party.lacks(&own))
    };
    if lacks {
        core.wake.notify_waiters();
    }
    changed
}

pub fn state_json(core: &Core, own: &Heads) -> Value {
    let party = core.party.lock().unwrap();
    let now = Instant::now();
    let watching = party.watching(&core.me(), now);
    if !party.on {
        return json!({"on": false, "watching": watching});
    }
    json!({"on": true, "shared": party.shared, "status": party.status(own, now), "watching": watching, "typing": party.typists(now)})
}

pub async fn repeat(core: Arc<Core>) {
    loop {
        tokio::time::sleep(CHECK).await;
        let Some((_, own)) = own_heads(&core) else { continue };
        if core.party.lock().unwrap().due(&own, Instant::now()) {
            send(&core);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(set: &str, wall: i64) -> Value {
        json!({"set": set, "playing": true, "wall_us": wall, "at_us": wall, "rate": 1.0, "focus": null, "mode": "grid", "order": []})
    }

    fn heads(pairs: &[(&str, u64)]) -> Heads {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    fn msg(p: &mut Party) -> Value {
        p.message("r", &Heads::new(), Instant::now())
    }

    fn party(me: &str) -> Party {
        let mut p = Party::default();
        p.turn(true, me, &view("s1", 0), "r", 1_000);
        p
    }

    #[test]
    fn newest_wins_by_seq_then_time_then_origin() {
        let a = json!({"seq": 2, "stamp": 5, "origin": "a"});
        assert!(key(&json!({"seq": 3, "stamp": 1, "origin": "a"})) > key(&a));
        assert!(key(&json!({"seq": 2, "stamp": 6, "origin": "a"})) > key(&a));
        assert!(key(&json!({"seq": 2, "stamp": 5, "origin": "b"})) > key(&a));
        assert!(key(&json!({"seq": 1, "stamp": 9, "origin": "z"})) < key(&a));
    }

    #[test]
    fn simultaneous_actions_converge() {
        let (mut a, mut b) = (party("a"), party("b"));
        a.act("a", &view("s1", 100), 1_000);
        b.act("b", &view("s1", 200), 1_000);
        let (ma, mb) = (msg(&mut a), msg(&mut b));
        assert!(a.receive("b", &mb, 0, "r", Instant::now()));
        assert!(!b.receive("a", &ma, 0, "r", Instant::now()));
        assert_eq!(key(&a.shared), key(&b.shared));
        assert_eq!(a.shared["wall_us"], b.shared["wall_us"]);
        a.act("a", &view("s2", 0), 900);
        assert!(key(&a.shared).0 > key(&b.shared).0);
    }

    #[test]
    fn a_joiner_adopts_the_running_state() {
        let mut old = party("a");
        old.act("a", &view("s1", 50), 10);
        let mut new = party("z");
        assert!(new.receive("a", &msg(&mut old), 0, "r", Instant::now()));
        assert_eq!(new.shared["wall_us"], 50);
        assert!(!old.receive("z", &msg(&mut new), 0, "r", Instant::now()));
    }

    #[test]
    fn turning_on_joins_the_party_without_moving_it() {
        let mut first = Party::default();
        first.turn(true, "z", &view("s1", 0), "r", 1_000);
        first.act("z", &view("s1", 700), 2_000);
        let mut joiner = Party::default();
        joiner.turn(true, "a", &view("s1", 0), "r", 5_000);
        assert!(joiner.receive("z", &msg(&mut first), 0, "r", Instant::now()));
        assert_eq!(joiner.shared["wall_us"], 700);
        assert!(!first.receive("a", &msg(&mut joiner), 0, "r", Instant::now()));
        assert_eq!(first.shared["wall_us"], 700);
        joiner.act("a", &view("s1", 0), 6_000);
        assert!(first.receive("a", &msg(&mut joiner), 0, "r", Instant::now()));
        assert_eq!(first.shared["wall_us"], 0);
    }

    #[test]
    fn rejoining_takes_the_running_state_again() {
        let (mut a, mut b) = (party("a"), party("b"));
        a.act("a", &view("s1", 10), 10);
        b.receive("a", &msg(&mut a), 0, "r", Instant::now());
        a.turn(false, "a", &Value::Null, "r", 20);
        b.act("b", &view("s1", 30), 30);
        a.turn(true, "a", &view("s1", 0), "r", 40);
        assert!(a.receive("b", &msg(&mut b), 0, "r", Instant::now()));
        assert_eq!(a.shared["wall_us"], 30);
    }

    #[test]
    fn party_messages_for_another_recording_are_not_followed() {
        let (mut a, mut b) = (party("a"), party("b"));
        b.act("b", &view("s2", 900), 5_000);
        assert!(!a.receive("b", &msg(&mut b), 0, "r", Instant::now()));
        assert_eq!((a.shared["set"].clone(), a.shared["wall_us"].clone()), (json!("s1"), json!(0)));
        a.turn(true, "a", &view("s2", 0), "r", 6_000);
        b.act("b", &view("s2", 950), 7_000);
        assert!(a.receive("b", &msg(&mut b), 0, "r", Instant::now()));
        assert_eq!(a.shared["wall_us"], 950);
        let mut idle = Party::default();
        idle.turn(true, "c", &json!({"set": null}), "r", 8_000);
        b.act("b", &view("s2", 960), 9_000);
        assert!(!idle.receive("b", &msg(&mut b), 0, "r", Instant::now()));
    }

    #[test]
    fn watching_lists_viewers_per_recording_and_includes_me_while_in_the_party() {
        let mut a = party("a");
        a.receive("c", &json!({"room": "r", "shared": {"seq": 0, "set": "s1"}}), 0, "r", Instant::now());
        a.receive("b", &json!({"room": "r", "shared": {"seq": 0, "set": "s1"}}), 0, "r", Instant::now());
        a.receive("d", &json!({"room": "r", "shared": {"seq": 0, "set": "s9"}}), 0, "r", Instant::now());
        assert_eq!(a.watching("a", Instant::now()), BTreeMap::from([("s1".to_string(), ids(&["a", "b", "c"])), ("s9".to_string(), ids(&["d"]))]));
        assert!(a.watching("a", Instant::now() + SEEN).get("s9").is_none());
        a.turn(false, "a", &Value::Null, "r", 2_000);
        assert_eq!(a.watching("a", Instant::now()).get("s1"), Some(&ids(&["b", "c"])));
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn positions_move_to_the_receivers_clock() {
        let mut a = party("a");
        let mut b = party("b");
        a.act("a", &view("s1", 1_000), 5);
        b.receive("a", &msg(&mut a), 300, "r", Instant::now());
        assert_eq!(b.shared["wall_us"], 700);
        assert_eq!(b.shared["at_us"], 700);
    }

    #[test]
    fn heads_only_grow_and_show_who_is_behind() {
        let mut a = party("a");
        let msg = json!({"room": "r", "shared": {"seq": 0}, "heads": {"x": 5, "y": 2}});
        a.receive("b", &msg, 0, "r", Instant::now());
        a.receive("c", &json!({"room": "r", "shared": {"seq": 0}, "heads": {"x": 3, "y": 2}}), 0, "r", Instant::now());
        assert_eq!(a.heads, heads(&[("x", 5), ("y", 2)]));
        let own = heads(&[("x", 5), ("y", 2)]);
        assert!(!a.lacks(&own));
        assert!(a.lacks(&heads(&[("x", 4), ("y", 2)])));
        assert!(a.status(&own, Instant::now()).is_empty());
        let status = a.status(&own, Instant::now() + REPEAT);
        assert_eq!(status, vec![json!({"kind": "catching", "who": "x"})]);
        a.message("r", &heads(&[("x", 1), ("y", 9)]), Instant::now());
        assert_eq!(a.heads, heads(&[("x", 5), ("y", 9)]));
    }

    #[test]
    fn typing_shows_while_the_viewer_is_seen() {
        let mut a = party("a");
        a.receive("b", &json!({"room": "r", "shared": {"seq": 0}, "typing": true}), 0, "r", Instant::now());
        a.receive("c", &json!({"room": "r", "shared": {"seq": 0}, "typing": false}), 0, "r", Instant::now());
        assert_eq!(a.typists(Instant::now()), vec!["b".to_string()]);
        assert!(a.typists(Instant::now() + SEEN).is_empty());
        a.typing = true;
        assert_eq!(msg(&mut a)["typing"], true);
    }

    #[test]
    fn other_rooms_and_off_are_ignored_but_viewers_are_counted() {
        let mut a = party("a");
        assert!(!a.receive("b", &json!({"room": "other", "shared": {"seq": 9}}), 0, "r", Instant::now()));
        a.turn(false, "a", &Value::Null, "r", 2_000);
        assert!(!a.receive("b", &json!({"room": "r", "shared": {"seq": 9, "set": "s4"}}), 0, "r", Instant::now()));
        assert!(a.caps().is_none());
        assert_eq!(a.watching("a", Instant::now()), BTreeMap::from([("s4".to_string(), ids(&["b"]))]));
        assert_ne!(a.shared["set"], "s4");
    }

    #[test]
    fn a_review_starts_only_while_watching_a_session_together() {
        let mut a = Party::default();
        assert!(!a.reviewing());
        a.turn(true, "a", &json!({"set": null}), "r", 1);
        assert!(!a.reviewing());
        a.act("a", &view("s1", 5), 2);
        assert!(a.reviewing());
        a.turn(false, "a", &Value::Null, "r", 3);
        assert!(!a.reviewing());
    }

    #[test]
    fn party_records_come_back_on_the_local_clock_in_order() {
        let shared = |wall: i64| json!({"set": "s1", "playing": true, "wall_us": wall, "at_us": wall});
        let a = room::Log { owner: "a".into(), offset_us: 0, records: std::sync::Arc::new(vec![record("rv", &shared(500), 300), record("other", &shared(1), 1), json!({"k": "party", "set": "rv"})]) };
        let b = room::Log { owner: "b".into(), offset_us: 100, records: std::sync::Arc::new(vec![record("rv", &shared(900), 350)]) };
        let all = records(&[a, b]);
        let list = &all["rv"];
        assert_eq!(list.iter().map(|r| (r["t_us"].as_i64().unwrap(), r["by"].as_str().unwrap())).collect::<Vec<_>>(), [(250, "b"), (300, "a")]);
        assert_eq!(list[0]["shared"]["wall_us"], 800);
        assert_eq!(all["other"].len(), 1);
        let watch = watch_json(list);
        assert_eq!(watch[0], json!({"t_us": 250, "by": "b", "set": "s1", "at_us": 800, "playing": true, "rate": null}));
        assert_eq!(watch.as_array().unwrap().len(), 2);
    }
}

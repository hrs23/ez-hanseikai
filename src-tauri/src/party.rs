use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::{file_len, net, room, Core};

pub const REPEAT: Duration = Duration::from_secs(4);
const SEEN: Duration = Duration::from_secs(10);
const CHECK: Duration = Duration::from_secs(1);
const SHARED: [&str; 8] = ["set", "playing", "wall_us", "at_us", "rate", "focus", "mode", "order"];

pub type Heads = HashMap<String, u64>;

pub struct Viewer {
    pub set: Value,
    pub heads: Heads,
    pub progress: Option<f64>,
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
    grown: HashMap<String, Instant>,
    sent: Option<(Instant, Heads)>,
}

pub fn key(v: &Value) -> (u64, i64, String) {
    (v["seq"].as_u64().unwrap_or(0), v["stamp"].as_i64().unwrap_or(0), v["origin"].as_str().unwrap_or_default().to_string())
}

pub fn newer(a: &Value, b: &Value) -> bool {
    key(a) > key(b)
}

pub fn merge_heads(into: &mut Heads, from: &Heads) {
    for (owner, n) in from {
        let at = into.entry(owner.clone()).or_insert(0);
        *at = (*at).max(*n);
    }
}

fn heads_of(value: &Value) -> Heads {
    value.as_object().into_iter().flatten().filter_map(|(k, v)| Some((k.clone(), v.as_u64()?))).collect()
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
            }
        }
        merge_heads(&mut self.heads, heads);
    }

    pub fn turn(&mut self, on: bool, me: &str, view: &Value, room: &str) {
        self.on = on;
        self.sent = None;
        if on {
            self.enter(room);
            self.shared = shared_from(view, 0, 0, me);
        }
    }

    pub fn act(&mut self, me: &str, view: &Value, now_us: i64) {
        if self.on {
            self.lamport += 1;
            self.shared = shared_from(view, self.lamport, now_us, me);
        }
    }

    pub fn receive(&mut self, from: &str, msg: &Value, offset_us: i64, room: &str, now: Instant) -> bool {
        if !self.on || msg["room"] != room {
            return false;
        }
        self.enter(room);
        let mut shared = msg["shared"].clone();
        self.lamport = self.lamport.max(key(&shared).0);
        let heads = heads_of(&msg["heads"]);
        self.absorb(&heads, now);
        self.viewers.insert(from.to_string(), Viewer { set: shared["set"].clone(), heads, progress: msg["progress"].as_f64(), seen: now });
        if !newer(&shared, &self.shared) {
            return false;
        }
        for k in ["wall_us", "at_us"] {
            if let Some(t) = shared[k].as_i64() {
                shared[k] = json!(t - offset_us);
            }
        }
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

    pub fn message(&mut self, room: &str, own: &Heads, progress: Option<f64>, now: Instant) -> Value {
        self.enter(room);
        self.absorb(own, now);
        self.sent = Some((now, own.clone()));
        json!({"t": "party", "room": self.room, "shared": self.shared, "heads": own, "progress": progress})
    }

    pub fn status(&self, me: &str, own: &Heads, progress: Option<f64>, now: Instant, name: impl Fn(&str) -> String) -> Vec<String> {
        let set = &self.shared["set"];
        let mut viewers: Vec<(&str, &Heads, Option<f64>, &Value)> = vec![(me, own, progress, set)];
        let mut others: Vec<(&String, &Viewer)> = self.viewers.iter().filter(|(_, v)| now.duration_since(v.seen) < SEEN).collect();
        others.sort_by_key(|(id, _)| id.as_str());
        viewers.extend(others.into_iter().map(|(id, v)| (id.as_str(), &v.heads, v.progress, &v.set)));
        let mut owners: Vec<&String> = self.heads.keys().collect();
        owners.sort();
        let mut out = Vec::new();
        for owner in owners {
            let settled = self.grown.get(owner).is_none_or(|at| now.duration_since(*at) >= REPEAT);
            if settled && viewers.iter().any(|(_, heads, _, _)| heads.get(owner).copied().unwrap_or(0) < self.heads[owner]) {
                out.push(format!("Catching up with {}", if owner == me { "you".to_string() } else { name(owner) }));
            }
        }
        for (id, _, progress, at) in viewers {
            if let Some(p) = progress.filter(|p| *p < 1.0 && *at == *set) {
                out.push(format!("{} receiving {}%", name(id), (p * 100.0).floor()));
            }
        }
        out
    }

    pub fn viewers(&self, now: Instant) -> usize {
        1 + self.viewers.values().filter(|v| now.duration_since(v.seen) < SEEN && v.set == self.shared["set"]).count()
    }
}

fn progress(core: &Core, room: &str, logs: &[room::Log], set: &Value) -> Option<f64> {
    let set = room::sets(core, room, logs).into_iter().find(|s| set.as_str() == Some(s.id.as_str()))?;
    let (mut have, mut total) = (0u64, 0u64);
    for f in &set.files {
        if let Some(t) = f.total {
            have += file_len(&core.set_file(room, &set.id, &f.owner)).min(t);
            total += t;
        }
    }
    (total > 0).then(|| have as f64 / total as f64)
}

pub fn send(core: &Arc<Core>) {
    let room = core.active_room_id();
    let logs = room::logs(core, &room);
    let own = room::holdings(&logs);
    let set = core.party.lock().unwrap().shared["set"].clone();
    let progress = progress(core, &room, &logs, &set);
    let message = {
        let mut party = core.party.lock().unwrap();
        if !party.on {
            return;
        }
        party.message(&room, &own, progress, Instant::now())
    };
    net::broadcast(core, message);
}

pub fn receive(core: &Arc<Core>, from: &str, msg: &Value, offset_us: i64) -> bool {
    let room = core.active_room_id();
    let own = room::holdings(&room::logs(core, &room));
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
    let room = core.active_room_id();
    let party = core.party.lock().unwrap();
    if !party.on {
        return json!({"on": false});
    }
    let set = party.shared["set"].clone();
    drop(party);
    let progress = progress(core, &room, &room::logs(core, &room), &set);
    let party = core.party.lock().unwrap();
    let now = Instant::now();
    let me = core.me();
    let status = party.status(&me, own, progress, now, |id| if id == me { "You".to_string() } else { room::display_name(core, id) });
    json!({"on": true, "shared": party.shared, "status": status, "viewers": party.viewers(now)})
}

pub async fn repeat(core: Arc<Core>) {
    loop {
        tokio::time::sleep(CHECK).await;
        let room = core.active_room_id();
        let own = room::holdings(&room::logs(&core, &room));
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

    fn party(me: &str) -> Party {
        let mut p = Party::default();
        p.turn(true, me, &view("s1", 0), "r");
        p
    }

    #[test]
    fn newest_wins_by_seq_then_time_then_origin() {
        let a = json!({"seq": 2, "stamp": 5, "origin": "a"});
        assert!(newer(&json!({"seq": 3, "stamp": 1, "origin": "a"}), &a));
        assert!(newer(&json!({"seq": 2, "stamp": 6, "origin": "a"}), &a));
        assert!(newer(&json!({"seq": 2, "stamp": 5, "origin": "b"}), &a));
        assert!(!newer(&a, &a));
        assert!(!newer(&json!({"seq": 1, "stamp": 9, "origin": "z"}), &a));
    }

    #[test]
    fn simultaneous_actions_converge() {
        let (mut a, mut b) = (party("a"), party("b"));
        a.act("a", &view("s1", 100), 1_000);
        b.act("b", &view("s1", 200), 1_000);
        let (ma, mb) = (a.message("r", &Heads::new(), None, Instant::now()), b.message("r", &Heads::new(), None, Instant::now()));
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
        old.act("a", &view("s2", 50), 10);
        let mut new = party("z");
        assert!(new.receive("a", &old.message("r", &Heads::new(), None, Instant::now()), 0, "r", Instant::now()));
        assert_eq!(new.shared["set"], "s2");
        assert!(!old.receive("z", &new.message("r", &Heads::new(), None, Instant::now()), 0, "r", Instant::now()));
    }

    #[test]
    fn positions_move_to_the_receivers_clock() {
        let mut a = party("a");
        let mut b = party("b");
        a.act("a", &view("s1", 1_000), 5);
        b.receive("a", &a.message("r", &Heads::new(), None, Instant::now()), 300, "r", Instant::now());
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
        assert!(a.status("a", &own, None, Instant::now(), |id| id.to_uppercase()).is_empty());
        let status = a.status("a", &own, None, Instant::now() + REPEAT, |id| id.to_uppercase());
        assert_eq!(status, vec!["Catching up with X".to_string()]);
        a.message("r", &heads(&[("x", 1), ("y", 9)]), None, Instant::now());
        assert_eq!(a.heads, heads(&[("x", 5), ("y", 9)]));
    }

    #[test]
    fn progress_is_shown_for_the_same_set() {
        let mut a = party("a");
        a.receive("b", &json!({"room": "r", "shared": {"seq": 0, "set": "s1"}, "progress": 0.725}), 0, "r", Instant::now());
        a.receive("c", &json!({"room": "r", "shared": {"seq": 0, "set": "s9"}, "progress": 0.1}), 0, "r", Instant::now());
        assert_eq!(a.status("a", &Heads::new(), Some(1.0), Instant::now(), |id| id.to_string()), vec!["b receiving 72%".to_string()]);
    }

    #[test]
    fn other_rooms_and_off_are_ignored() {
        let mut a = party("a");
        assert!(!a.receive("b", &json!({"room": "other", "shared": {"seq": 9}}), 0, "r", Instant::now()));
        a.turn(false, "a", &Value::Null, "r");
        assert!(!a.receive("b", &json!({"room": "r", "shared": {"seq": 9}}), 0, "r", Instant::now()));
        assert!(a.caps().is_none());
    }
}

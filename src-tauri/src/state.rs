// The app state the window shows: one snapshot of the room, its recordings, the people and the app, built from the other modules.
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;

use serde_json::{json, Value};

use crate::room::{self, board_cards, colors, covers, debrief_of, display_name, events, full, holdings, last_end, my_profile, named_debriefs, names, others_have, others_hold, parts_json, profiles, progress, set_layout, sets, stuck, wants, Log, Room, SERIES};
use crate::{data, ffmpeg, now_us, party, record, stream, Core, Peer, PROTOCOL, VERSION};

struct Holder<'a> {
    id: &'a str,
    have: &'a HashMap<String, u64>,
}

pub fn sets_json(core: &Core, room: &str, logs: &[Log], events: &[Value]) -> Vec<Value> {
    let me = core.me();
    let peers = others_have(core);
    let online: Vec<HashMap<String, u64>> = core.peers.lock().unwrap().iter().filter(|p| p.conn.is_some()).map(crate::Peer::sendable).collect();
    let online: Vec<&HashMap<String, u64>> = online.iter().collect();
    let reports: Vec<&HashMap<String, u64>> = peers.iter().map(|p| &p.1).collect();
    let all = sets(core, room, logs);
    let mine = data::have_counts(core, room, &all);
    let mut analysis = crate::jobs::analysis_by_set(core, room, logs, &all, &mine);
    let mut out = Vec::new();
    for set in all {
        let mut holders = vec![Holder { id: &me, have: &mine }];
        holders.extend(peers.iter().map(|(id, have, _)| Holder { id, have }));
        let mut players = Vec::new();
        for f in &set.files {
            let (have, total) = data::held(core, room, f);
            let by: Vec<&str> = holders.iter().filter(|h| full(h.have, &set, f)).map(|h| h.id).collect();
            players.push(json!({
                "owner": f.owner, "imported": f.view.is_some(), "start_us": f.start_us, "end_us": f.end_us, "offset_us": f.offset_us, "have": have, "total": total,
                "recording": f.recording(), "verified_by": by, "stuck": stuck(&mine, &online, &set, f), "parts": parts_json(room, &set, f, &|capture, series| stream::folder_size(&core.files, &data::seg_dir(&core.room_dir(room), f, capture, series))),
                "last_us": {"v": last_end(f, SERIES[0]), "m": last_end(f, SERIES[1])},
            }));
        }
        let (have, total, stuck_all) = progress(core, &set, &mine, &online);
        let covered: Vec<&Value> = events.iter().filter(|e| covers(&set, e)).map(|e| &e["key"]).collect();
        let imported = set.files.iter().find_map(|f| f.view.as_ref());
        out.push(json!({
            "id": set.id, "room": room, "start_us": set.start(), "end_us": set.end_shown(), "live": set.recording(),
            "thumb": crate::thumbs::path(&core.room_dir(room), &set.id).exists(), "title": imported.and_then(|v| v.title.clone()), "imported_by": imported.map(|v| display_name(core, &v.by)),
            "players": players, "events": covered, "progress": {"have": have, "total": total, "stuck": stuck_all}, "wanted": wants(core, room, &set.id) == Some(true), "shared": others_hold(&set.id, &reports),
            "layout": set_layout(logs, &set.id), "of": debrief_of(&set.id, logs), "debriefs": named_debriefs(logs, &set.id), "analysis": analysis.remove(&set.id).unwrap_or_default(),
        }));
    }
    out
}

fn peer_json(p: &Peer) -> Value {
    let alive = p.alive();
    let kind = alive.map(crate::relay::path_kind);
    let (path, rtt, relay) = (kind.as_ref().map(|k| k.0), kind.as_ref().map(|k| k.1), kind.and_then(|k| k.2));
    let stats = alive.map(|c| c.stats());
    let other_version = p.info.as_ref().is_some_and(|(_, proto)| *proto != PROTOCOL);
    let sharing = p.live.is_some() && alive.is_some();
    json!({
        "id": p.id.to_string(),
        "name": p.name,
        "connected": alive.is_some(),
        "path": path,
        "rtt_ms": rtt,
        "relay": relay,
        "sent": stats.as_ref().map(|s| s.udp_tx.bytes),
        "received": stats.as_ref().map(|s| s.udp_rx.bytes),
        "failures": p.failures,
        "seen_us": p.seen_us,
        "offset_us": p.offset_us,
        "version": p.info.as_ref().map(|(version, _)| version),
        "other_version": other_version,
        "sharing": sharing,
        "speaking": p.speaking && alive.is_some(),
    })
}

fn last_used(core: &Core, room: &str) -> Option<i64> {
    std::fs::metadata(core.log_path(room, &core.me())).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_micros() as i64)
}

pub fn state_json(core: &Core) -> Value {
    let room_id = core.room_id();
    let live = |owner: &str| room_id.as_deref().and_then(|room| stream::live_json(core, room, owner));
    let my_id = core.ep.id().to_string();
    let active = core.active_room();
    let mut peers: Vec<Value> = core.peers.lock().unwrap().iter().map(peer_json).collect();
    for p in peers.iter_mut().filter(|p| p["sharing"] == true) {
        p["live"] = json!(live(p["id"].as_str().unwrap_or_default()));
    }
    let mut names = names(core, active.as_ref(), false);
    names.insert(my_id.clone(), "You".to_string());
    let logs = room_id.as_deref().map(|room| room::logs(core, room)).unwrap_or_default();
    if let Some(room) = room_id.as_deref() {
        room::keep_label(core, room, &logs);
    }
    let active = core.active_room();
    let caps = core.party.lock().unwrap().caps();
    let mut events = events(&logs, caps.as_ref());
    let sets = room_id.as_deref().map(|room| sets_json(core, room, &logs, &events)).unwrap_or_default();
    let recorded: Vec<(u64, i64)> = sets.iter().filter_map(|s| Some((s["progress"]["total"].as_u64()?, s["end_us"].as_i64()? - s["start_us"].as_i64()?))).collect();
    let disk = data::disk_json(core.free_bytes.load(Ordering::Relaxed), core.total_bytes.load(Ordering::Relaxed), data::bytes_per_hour(&recorded), core.disk_full());
    let members: Vec<String> = active.as_ref().map(|r| r.members.clone()).unwrap_or_default();
    let board: Vec<Value> = board_cards(&events, &members).into_iter().filter(|c| c["gone"] != true).collect();
    let seekable: HashSet<&str> = sets.iter().filter(|s| s["progress"]["have"].as_u64() > Some(0)).flat_map(|s| s["events"].as_array().into_iter().flatten().filter_map(|k| k.as_str())).collect();
    for e in events.iter_mut() {
        let (can_seek, mine) = (seekable.contains(e["key"].as_str().unwrap_or_default()) && e["local_us"].is_i64(), e["author"] == my_id.as_str() && e["plugin"].is_null());
        e["seekable"] = json!(can_seek);
        e["mine"] = json!(mine);
    }
    let mut asks = crate::net::pending_asks(core);
    for p in core.peers.lock().unwrap().iter().filter(|p| p.alive().is_some()) {
        for a in &p.asks {
            if !asks.iter().any(|b| b["id"] == a["id"]) {
                asks.push(a.clone());
            }
        }
    }
    let mut looks = profiles(&logs);
    looks.insert(my_id.clone(), my_profile(core));
    let chosen: HashMap<String, usize> = looks.iter().filter_map(|(id, p)| Some((id.clone(), p.color?))).collect();
    let avatars: HashMap<&String, &String> = looks.iter().filter(|(_, p)| !p.avatar.is_empty()).map(|(id, p)| (id, &p.avatar)).collect();
    let mut people = active.as_ref().map(Room::people).unwrap_or_default();
    if !people.contains(&my_id) {
        people.push(my_id.clone());
    }
    let rooms = core.rooms.lock().unwrap();
    let room_list: Vec<Value> = rooms.list.iter().map(|r| json!({"id": r.id, "name": r.name, "icon": r.icon, "active": rooms.active.as_deref() == Some(&r.id), "used_us": last_used(core, &r.id)})).collect();
    drop(rooms);
    json!({
        "app": crate::shell::app_name(&core.app, &crate::language(core)),
        "version": VERSION,
        "now_us": now_us(),
        "clock": crate::clock::status(),
        "ffmpeg": core.ffmpeg_status.lock().unwrap().clone(),
        "ffmpeg_progress": ffmpeg::progress(),
        "id": my_id,
        "name": core.name.lock().unwrap().clone(),
        "lang": crate::language(core),
        "language": *core.language.lock().unwrap(),
        "colors": colors(&people, &chosen),
        "avatars": avatars,
        "invites": core.invites.lock().unwrap().len(),
        "limit_mbps": *core.limit_mbps.lock().unwrap(),
        "relays": core.relays.lock().unwrap().iter().map(|u| u.to_string()).collect::<Vec<_>>(),
        "recovery": core.recovery.lock().unwrap().clone(),
        "notice": core.notice.lock().unwrap().clone(),
        "disk": disk,
        "previous_version": core.previous_version,
        "crashed": core.crashed.load(Ordering::Relaxed),
        "recording": core.any_recording(),
        "capture": record::capture_json(core, live(&my_id)),
        "active_set": core.active_set_json(),
        "room": active.map(|r| json!({"id": r.id, "name": r.name, "icon": r.icon, "pending": r.invite.is_some(), "waiting": r.waiting})),
        "rooms": room_list,
        "layout": core.layout.lock().unwrap().clone(),
        "hotkeys": crate::shell::hotkeys_json(&core.hotkeys.lock().unwrap(), &core.hotkeys_taken.lock().unwrap()),
        "notify_joins": *core.notify_joins.lock().unwrap(),
        "api": {"on": *core.api_on.lock().unwrap(), "address": crate::api::address(), "error": core.api_error.lock().unwrap().clone()},
        "sets": sets,
        "peers": peers,
        "events": events,
        "tag_names": crate::jobs::tag_names(&logs, caps.as_ref()),
        "board": board,
        "names": names,
        "asks": asks,
        "party": party::state_json(core, &holdings(&logs)),
    })
}

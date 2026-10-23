use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use iroh::Watcher;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::{api, audio, clean_name, cleanup, clean_note, clean_text, data, debug, export, ffmpeg, jobs, merge, net, now_us, party, record, relay, room, shell, sources, Core, BOARD_MAX, EVENT_KINDS, NAME_MAX, NOTE_MAX, REPO, VALUES_MAX};

pub type Op = fn(&Arc<Core>, Value) -> Result<Value, String>;

pub const OPS: &[(&str, Op)] = &[
    ("create_room", |c, a| room::create_room(c, &arg::<String>(&a, "name")?).map(|id| json!(id))),
    ("join_room", |c, a| null(room::join_room(c, &arg::<String>(&a, "code")?))),
    ("enter_room", |c, a| null(room::enter_room(c, &arg::<String>(&a, "id")?))),
    ("close_room", close_room),
    ("remove_room", remove_room),
    ("rename_room", |c, a| null(room::rename_room(c, &arg::<String>(&a, "name")?))),
    ("set_room_icon", |c, a| null(room::set_room_icon(c, &arg::<String>(&a, "icon")?))),
    ("create_invite", create_invite),
    ("cancel_invites", cancel_invites),
    ("answer_ask", answer_ask),
    ("remove_peer", remove_peer),
    ("measure_speed", |c, a| tauri::async_runtime::block_on(net::measure_speed(c, &arg::<String>(&a, "id")?)).ok_or_else(|| "No answer".into())),
    ("check", check),
    ("start_recording", |c, _| null(room::start_set(c))),
    ("stop_recording", stop_recording),
    ("mark", mark),
    ("list_sources", |_, _| Ok(json!(with_sizes(sources::list(), &display_sizes())))),
    ("set_source", set_source),
    ("list_mics", |_, _| Ok(json!(audio::list_mics()))),
    ("set_mic", set_mic),
    ("set_mix", set_mix),
    ("set_quality", set_quality),
    ("set_limit", set_limit),
    ("set_relays", set_relays),
    ("set_codecs", set_codecs),
    ("set_name", set_name),
    ("set_profile", set_profile),
    ("set_language", set_language),
    ("set_hotkey", |c, a| null(shell::set_hotkey(c, &arg::<String>(&a, "action")?, &arg::<String>(&a, "keys")?))),
    ("set_layout", set_layout),
    ("check_update", |c, _| Ok(json!(find_update(c)?.map(|u| u.version)))),
    ("install_update", install_update),
    ("data_info", |c, _| Ok(json!({"dir": c.dir, "used_bytes": data::used_bytes(&c.dir), "free_bytes": c.free_bytes.load(Ordering::Relaxed)}))),
    ("reset", reset),
    ("dismiss_recovery", dismiss_recovery),
    ("retry_ffmpeg", retry_ffmpeg),
    ("add_event", add_event),
    ("event_action", event_action),
    ("delete_ai_notes", delete_ai_notes),
    ("put_board_card", put_board_card),
    ("open_board_window", open_board_window),
    ("close_board_window", |c, _| { shell::close_board(&c.app); Ok(Value::Null) }),
    ("set_fullscreen", |c, a| null(shell::set_fullscreen(&c.app, &arg::<String>(&a, "window")?, arg(&a, "on")?))),
    ("party_on", party_on),
    ("party_act", party_act),
    ("party_typing", party_typing),
    ("draw", draw),
    ("room_plugins", jobs::room_plugins),
    ("set_room_plugin", jobs::set_room_plugin),
    ("set_plugin_runner", jobs::set_plugin_runner),
    ("set_plugin_setting", jobs::set_plugin_setting),
    ("retry_job", jobs::retry_job),
    ("delete_set", |c, a| {
        let (room, set) = recording_ids(c, &a)?;
        room::delete_set(c, &room, &set).map(|deleted| json!(deleted))
    }),
    ("download_set", |c, a| {
        let (room, set) = recording_ids(c, &a)?;
        null(room::download_set(c, &room, &set))
    }),
    ("focus_set", focus_set),
    ("export_check", |c, _| null(export::check(c))),
    ("export_folder", export_folder),
    ("export_target", export_target),
    ("set_api", set_api),
    ("set_notify_joins", set_notify_joins),
    ("new_api_token", new_api_token),
    ("api_info", api_info),
    ("api_reference", |_, _| Ok(json!(api::reference()))),
    ("state", |c, _| Ok(room::state_json(c))),
    ("debug_flow", debug::debug_flow),
    ("debug_report", debug::debug_report),
    ("ui_commands", |c, _| Ok(c.ui.lock().unwrap()["commands"].clone())),
    ("ui_state", |c, _| Ok(c.ui.lock().unwrap()["state"].clone())),
    ("ui_do", ui_do),
    ("player_log", |_, a| {
        let text: String = arg(&a, "text")?;
        tracing::info!("player: {}", text.chars().filter(|c| !c.is_control()).take(300).collect::<String>());
        Ok(Value::Null)
    }),
];

pub fn find(name: &str) -> Option<Op> {
    OPS.iter().find(|(known, _)| *known == name).map(|(_, op)| *op)
}

fn null<T>(result: Result<T, String>) -> Result<Value, String> {
    result.map(|_| Value::Null)
}

pub fn arg<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T, String> {
    serde_json::from_value(args[name].clone()).map_err(|_| format!("{name} is missing or not valid."))
}

pub fn recording_ids(core: &Core, args: &Value) -> Result<(String, String), String> {
    let (room, set): (String, String) = (arg(args, "room")?, arg(args, "set")?);
    if room::valid_id(&room) && room::valid_id(&set) && core.room(&room).is_some() { Ok((room, set)) } else { Err("That recording is not on this PC.".into()) }
}

pub fn warm(core: &Core) {
    let wait = Duration::from_secs(6);
    tauri::async_runtime::block_on(async {
        let _ = tokio::time::timeout(wait, core.ep.online()).await;
        let _ = tokio::time::timeout(wait, core.ep.net_report().initialized()).await;
    });
}

fn check(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    match args["id"].as_str().filter(|id| *id != core.me()) {
        Some(id) => Ok(tauri::async_runtime::block_on(net::check_peer(core, id))),
        None => {
            warm(core);
            Ok(crate::check::json(core))
        }
    }
}

fn close_room(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    room::leave_room(core, &arg::<String>(&args, "id")?);
    Ok(Value::Null)
}

fn remove_room(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let id: String = arg(&args, "id")?;
    room::listed(core, &id)?;
    if core.export.lock().unwrap().as_ref().is_some_and(|e| e.set.0 == id) {
        return Err("This recording is being exported. Try again when it finishes.".into());
    }
    room::forget_room(core, &id);
    jobs::release_room(&id);
    core.removed_rooms.lock().unwrap().insert(id);
    room::sweep_removed(core);
    Ok(Value::Null)
}

fn create_invite(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let room = core.joined_room()?;
    let reuse: bool = arg(&args, "reuse")?;
    let mut invites = core.invites.lock().unwrap();
    let token = match invites.iter().rev().find(|(_, r)| *r == room).filter(|_| reuse) {
        Some((token, _)) => token.clone(),
        None => {
            let token = room::random_hex(room::TOKEN_BYTES);
            invites.push((token.clone(), room));
            token
        }
    };
    Ok(json!(format!("ezh-{}-{token}", core.ep.id())))
}

fn cancel_invites(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    core.invites.lock().unwrap().clear();
    core.asks.lock().unwrap().clear();
    net::send_asks(core);
    Ok(Value::Null)
}

fn answer_ask(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    net::answer(core, &arg::<String>(&args, "id")?, arg(&args, "approve")?);
    Ok(Value::Null)
}

fn remove_peer(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    null(room::remove_member(core, &arg::<String>(&args, "id")?))
}

fn stop_recording(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    room::stop_set(core);
    Ok(Value::Null)
}

pub fn log_and_share(core: &Arc<Core>, room: &str, record: &Value) {
    core.append_log(room, record);
    party::send(core);
}

fn log_event(core: &Arc<Core>, room: &str, kind: &str, text: &str, target: Value, values: Value) -> Value {
    let id = now_us();
    log_and_share(core, room, &room::user_event(id, kind, text, target, values));
    json!(id)
}

pub fn mark(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    let room = core.active_set.lock().unwrap().as_ref().map(|s| s.room.clone()).ok_or("Start recording first.")?;
    Ok(log_event(core, &room, "mark", "", json!({"t_us": now_us()}), Value::Null))
}

fn display_sizes() -> Vec<(u32, u32)> {
    windows_capture::monitor::Monitor::enumerate().unwrap_or_default().iter().map(|m| (m.width().unwrap_or(0), m.height().unwrap_or(0))).collect()
}

fn with_sizes(mut list: Vec<Value>, sizes: &[(u32, u32)]) -> Vec<Value> {
    for s in list.iter_mut() {
        let index = s["id"]["index"].as_u64().unwrap_or(0) as usize;
        if let Some((w, h)) = index.checked_sub(1).and_then(|i| sizes.get(i)).filter(|(w, h)| *w > 0 && *h > 0) {
            s["width"] = json!(w);
            s["height"] = json!(h);
        }
    }
    list
}

fn set_source(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let source = args["source"].clone();
    if !sources::valid(&source) {
        return Err("That capture source is not available.".into());
    }
    tracing::info!("capture source chosen: {}", if source.is_null() { "none" } else { source["kind"].as_str().unwrap_or_default() });
    *core.engine.source.lock().unwrap() = source;
    Ok(Value::Null)
}

fn set_mic(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    *core.engine.mic.lock().unwrap() = arg(&args, "id")?;
    core.save_settings();
    Ok(Value::Null)
}

fn set_mix(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let kind: String = arg(&args, "kind")?;
    let track = match kind.as_str() {
        "app" => record::APP,
        "mic" => record::MIC,
        _ => return Err(format!("unknown kind {kind}")),
    };
    let (volume, muted): (f64, bool) = (arg(&args, "volume")?, arg(&args, "muted")?);
    core.engine.mix.lock().unwrap()[record::MIX_KEYS[track]] = json!({"volume": volume.clamp(0.0, 1.0), "muted": muted});
    core.save_settings();
    Ok(Value::Null)
}

fn set_quality(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let id: String = arg(&args, "id")?;
    if !record::QUALITIES.iter().any(|q| q.0 == id) {
        return Err(format!("unknown quality {id}"));
    }
    *core.engine.quality.lock().unwrap() = id;
    *core.engine.cap.lock().unwrap() = None;
    core.save_settings();
    Ok(Value::Null)
}

fn set_limit(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    *core.limit_mbps.lock().unwrap() = arg::<Option<f64>>(&args, "mbps")?.filter(|v| v.is_finite() && *v > 0.0).map(|v| v.min(100_000.0));
    *core.pacer.lock().unwrap() = net::Pacer::new();
    core.save_settings();
    Ok(Value::Null)
}

fn set_relays(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    *core.relays.lock().unwrap() = relay::parse(&arg::<Vec<String>>(&args, "urls")?)?;
    core.relay_wake.notify_one();
    core.save_settings();
    Ok(Value::Null)
}

fn set_codecs(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    *core.codecs.lock().unwrap() = arg(&args, "list")?;
    Ok(Value::Null)
}

fn set_name(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    *core.name.lock().unwrap() = clean_name(&arg::<String>(&args, "name")?);
    core.save_settings();
    Ok(Value::Null)
}

fn set_profile(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    struct Look {
        color: Option<u64>,
        avatar: Option<String>,
    }
    let look: Look = serde_json::from_value(args).map_err(|_| "color or avatar is not valid.".to_string())?;
    if look.color.is_some_and(|c| room::check_color(c).is_none()) {
        return Err("color is not valid.".into());
    }
    if let Some(avatar) = &look.avatar {
        room::check_avatar(avatar)?;
    }
    if let Some(color) = look.color {
        *core.color.lock().unwrap() = Some(color as usize);
    }
    if let Some(avatar) = look.avatar {
        *core.avatar.lock().unwrap() = avatar;
    }
    core.save_settings();
    room::share_profile(core);
    Ok(Value::Null)
}

fn set_language(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let language: String = arg(&args, "language")?;
    if !["", "en", "ja"].contains(&language.as_str()) {
        return Err(format!("unknown language {language}"));
    }
    *core.language.lock().unwrap() = language;
    core.save_settings();
    let _ = shell::install_menus(&core.app, core);
    Ok(Value::Null)
}

fn set_layout(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    *core.layout.lock().unwrap() = args["layout"].clone();
    core.save_settings();
    room::log_set_layout(core);
    Ok(Value::Null)
}

fn find_update(core: &Core) -> Result<Option<tauri_plugin_updater::Update>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let url = tauri::Url::parse(&format!("{REPO}/releases/download/updater/latest.json")).map_err(|e| e.to_string())?;
    let updater = core.app.updater_builder().endpoints(vec![url]).and_then(|b| b.build()).map_err(|e| e.to_string())?;
    tauri::async_runtime::block_on(updater.check()).map_err(|e| e.to_string())
}

fn install_update(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    let update = find_update(core)?.ok_or("No update is available.")?;
    let bytes = tauri::async_runtime::block_on(update.download(|_, _| {}, || {})).map_err(|e| e.to_string())?;
    tracing::info!("installing version {}", update.version);
    data::resume_later(core);
    data::release(core);
    let installed = update.install(bytes).map_err(|e| e.to_string());
    if installed.is_err() {
        let _ = data::lock(&core.dir);
        data::forget_resume(&core.dir);
    }
    installed.map(|_| Value::Null)
}

fn reset(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let keep_rooms: bool = arg(&args, "keep_rooms")?;
    tracing::info!("resetting the app{}", if keep_rooms { ", keeping the rooms" } else { "" });
    data::release(core);
    cleanup::before_reset(core, keep_rooms);
    data::reset(&core.dir, keep_rooms);
    data::forget(&core.files, &core.dir);
    core.app.restart()
}

fn dismiss_recovery(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    *core.recovery.lock().unwrap() = None;
    Ok(Value::Null)
}

fn retry_ffmpeg(_: &Arc<Core>, _: Value) -> Result<Value, String> {
    ffmpeg::retry();
    Ok(Value::Null)
}

fn add_event(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let room = core.joined_room()?;
    let (kind, text): (String, String) = (arg(&args, "kind")?, arg(&args, "text")?);
    let (target, values) = (args["target"].clone(), args["values"].clone());
    if !EVENT_KINDS.contains(&kind.as_str()) || !target.is_object() || values.to_string().len() > VALUES_MAX {
        return Err(format!("not a valid {kind} event"));
    }
    let text = clean_note(&text, if kind == "board" { BOARD_MAX } else { NOTE_MAX });
    let text = if kind == "board" { text } else { text.replace(['\n', '\t'], " ").trim().to_string() };
    if kind == "note" && text.is_empty() {
        return Err("Write something first.".into());
    }
    let id = now_us();
    let mut record = room::user_event(id, &kind, &text, target, values);
    if kind == "note" {
        record = room::with_mentions(record, &room::names(core, core.room(&room).as_ref(), false));
    }
    log_and_share(core, &room, &record);
    Ok(json!(id))
}

fn event_action(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let room = core.joined_room()?;
    let mut record = match arg::<String>(&args, "kind")?.as_str() {
        "edit" => json!({"k": "event_edit"}),
        "delete" => json!({"k": "event_del"}),
        "restore" => json!({"k": "event_restore"}),
        other => return Err(format!("unknown kind {other}")),
    };
    let text = clean_note(&arg::<String>(&args, "text")?, NOTE_MAX).replace(['\n', '\t'], " ").trim().to_string();
    if text.is_empty() && record["k"] == "event_edit" {
        return Err("Write something first.".into());
    }
    merge(&mut record, &json!({"id": arg::<i64>(&args, "id")?, "t_us": now_us(), "text": text, "tags": room::tags(&text)}));
    record = room::with_mentions(record, &room::names(core, core.room(&room).as_ref(), false));
    log_and_share(core, &room, &record);
    Ok(Value::Null)
}

fn delete_ai_notes(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let (room, set) = recording_ids(core, &args)?;
    core.check_joined(&room)?;
    log_and_share(core, &room, &json!({"k": "ai_del", "set": set, "t_us": now_us()}));
    Ok(Value::Null)
}

fn put_board_card(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let room: String = arg(&args, "room")?;
    core.check_joined(&room)?;
    let wanted: String = arg(&args, "card")?;
    let card = if wanted.is_empty() { format!("c:{}", room::random_hex(4)) } else { wanted };
    if !room::board_card_valid(&card) {
        return Err("That is not a card.".into());
    }
    let members = core.active_room().map(|r| r.members).unwrap_or_default();
    let cards = room::board_cards(&room::events(&room::logs(core, &room), None), &members);
    let current = cards.iter().find(|c| c["card"] == card.as_str());
    let text = match arg::<Option<String>>(&args, "text")? {
        Some(text) => clean_note(&text, BOARD_MAX),
        None => current.and_then(|c| c["text"].as_str()).unwrap_or_default().to_string(),
    };
    let title = match arg::<Option<String>>(&args, "title")? {
        Some(title) => clean_text(&title, NAME_MAX),
        None => current.and_then(|c| c["title"].as_str()).unwrap_or_default().to_string(),
    };
    let last = cards.iter().filter_map(|c| c["pos"].as_f64()).fold(0.0, f64::max);
    let pos = arg::<Option<f64>>(&args, "pos")?.filter(|p| p.is_finite()).or_else(|| current.and_then(|c| c["pos"].as_f64())).unwrap_or(last + 1.0);
    let gone = arg::<Option<bool>>(&args, "gone")?.unwrap_or_else(|| current.is_some_and(|c| c["gone"] == true));
    let size = |k: &str| -> Result<Value, String> { Ok(arg::<Option<f64>>(&args, k)?.map_or_else(|| current.map_or(Value::Null, |c| c[k].clone()), |v| json!(v))) };
    let (w, h) = room::board_span(&card, &size("w")?, &size("h")?);
    let mut values = json!({"card": card, "pos": pos, "w": w, "h": h, "gone": gone});
    if !title.is_empty() {
        values["title"] = json!(title);
    }
    log_event(core, &room, "board", &text, json!({}), values);
    Ok(json!(card))
}

fn open_board_window(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    shell::open_board(&core.app, &core.dir);
    Ok(Value::Null)
}

fn party_on(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let me = core.me();
    let room = core.joined_room()?;
    core.party.lock().unwrap().turn(arg(&args, "on")?, &me, &args["view"], &room, now_us());
    party::send(core);
    Ok(Value::Null)
}

fn party_act(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let me = core.me();
    core.party.lock().unwrap().act(&me, &args["view"], now_us());
    party::send(core);
    party::log(core);
    Ok(Value::Null)
}

fn party_typing(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    core.party.lock().unwrap().typing = arg(&args, "on")?;
    party::send(core);
    Ok(Value::Null)
}

fn draw(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let Some(room) = core.room_id() else { return Ok(Value::Null) };
    if core.party.lock().unwrap().on {
        net::broadcast(core, json!({"t": "draw", "room": room, "stroke": args["stroke"]}));
    }
    Ok(Value::Null)
}

fn focus_set(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    *core.focus.lock().unwrap() = arg(&args, "set")?;
    Ok(Value::Null)
}

fn export_folder(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let (room, set) = recording_ids(core, &args)?;
    let on = |key: &str| arg::<Option<bool>>(&args, key).map(|v| v.unwrap_or(true));
    let burn = crate::debrief::Burn { notes: on("notes")?, marks: on("marks")?, matches: on("matches")? };
    let opts = export::Options { range: arg::<Option<_>>(&args, "range")?.unwrap_or_default(), quality: arg::<Option<_>>(&args, "quality")?.unwrap_or_default(), member: arg(&args, "member")?, kind: arg::<Option<_>>(&args, "kind")?.unwrap_or_default(), burn };
    let dir = match arg::<Option<String>>(&args, "folder")? {
        Some(folder) => PathBuf::from(folder),
        None => target(core, &args, &room, &set)?,
    };
    export::folder(core, &room, &set, &dir, &opts)
}

fn export_target(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let (room, set) = recording_ids(core, &args)?;
    Ok(json!(target(core, &args, &room, &set)?.to_string_lossy()))
}

fn target(core: &Core, args: &Value, room: &str, set: &str) -> Result<PathBuf, String> {
    export::target(core, room, set, arg::<Option<String>>(args, "base")?.map(PathBuf::from), arg::<Option<String>>(args, "name")?)
}

fn ui_do(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    use tauri::{Emitter, Manager};
    if core.app.get_webview_window("main").is_none() {
        return Ok(Value::Null);
    }
    let cmd: String = arg(&args, "cmd")?;
    let given = if args["args"].is_null() { json!({}) } else { args["args"].clone() };
    check_ui_args(&core.ui.lock().unwrap()["commands"], &cmd, &given)?;
    core.app.emit_to("main", "ui_command", json!({"cmd": cmd, "args": given})).map_err(|e| e.to_string())?;
    Ok(Value::Null)
}

fn check_ui_args(commands: &Value, cmd: &str, args: &Value) -> Result<(), String> {
    if commands.is_null() {
        return Err("the window has not reported its commands yet".into());
    }
    let spec = commands.get(cmd).and_then(|c| c["args"].as_object()).ok_or_else(|| format!("unknown cmd {cmd}; ui_commands lists them"))?;
    let given = args.as_object().ok_or("args is not an object.")?;
    if let Some(extra) = given.keys().find(|k| !spec.contains_key(*k)) {
        return Err(format!("{cmd} takes no argument {extra}."));
    }
    for (name, kind) in spec {
        let v = given.get(name).unwrap_or(&Value::Null);
        let ok = match kind {
            Value::Array(allowed) => allowed.iter().any(|a| a == v || (a.is_number() && a.as_f64() == v.as_f64())),
            Value::String(t) => match t.as_str() {
                "number" => v.is_number(),
                "integer" => v.is_i64() || v.is_u64(),
                "boolean" => v.is_boolean(),
                "string" => v.is_string(),
                _ => false,
            },
            _ => false,
        };
        if !ok {
            return Err(format!("{name} is missing or not valid."));
        }
    }
    Ok(())
}

pub fn enable_api(core: &Arc<Core>, on: bool) {
    *core.api_on.lock().unwrap() = on;
    if on && core.api_token.lock().unwrap().is_empty() {
        *core.api_token.lock().unwrap() = room::random_hex(api::TOKEN_BYTES);
    }
    core.save_settings();
    api::apply(core);
}

fn set_api(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    enable_api(core, arg(&args, "on")?);
    Ok(Value::Null)
}

fn set_notify_joins(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    *core.notify_joins.lock().unwrap() = arg(&args, "on")?;
    core.save_settings();
    Ok(Value::Null)
}

fn new_api_token(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    *core.api_token.lock().unwrap() = room::random_hex(api::TOKEN_BYTES);
    core.save_settings();
    api::refresh(core);
    Ok(Value::Null)
}

fn api_info(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    let on = *core.api_on.lock().unwrap();
    Ok(json!({"on": on, "address": api::address(), "token": if on { core.api_token.lock().unwrap().clone() } else { String::new() }, "error": core.api_error.lock().unwrap().clone()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names;
    use std::path::Path;

    const WINDOW_COMMANDS: [&str; 11] = ["feedback_snap", "feedback_preview", "send_feedback", "ui_report", "open_set_folder", "open_data_folder", "open_link", "open_folder", "pick_export_base", "save_text", "show_main"];

    fn files(dir: &str, ext: &str) -> Vec<(String, String)> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
        let mut out: Vec<(String, String)> = std::fs::read_dir(dir).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == ext)).map(|e| (e.file_name().to_string_lossy().to_string(), std::fs::read_to_string(e.path()).unwrap())).collect();
        out.sort();
        out
    }

    #[test]
    fn the_page_calls_only_listed_operations_and_every_operation_is_documented() {
        let page = files("../ui", "js");
        assert!(page.len() > 10);
        for (file, text) in &page {
            for name in names(r#"\b(?:invoke|run|quiet)\("([a-z_]+)""#, text) {
                assert!(find(&name).is_some(), "{file} calls {name}, which is not an operation");
            }
            for name in names(r#"\bdialog\("([a-z_]+)""#, text) {
                assert!(WINDOW_COMMANDS.contains(&name.as_str()) && find(&name).is_none(), "{file} opens {name}, which is not a window command");
            }
            for name in names(r#"\bnew Set\(\[([^\]]*)\]"#, text).iter().flat_map(|list| names(r#""([a-z_]+)""#, list)) {
                assert!(find(&name).is_some(), "{file} lists {name}, which is not an operation");
            }
            assert!(file == "util.js" || !text.contains("__TAURI__.core"), "{file} reaches the backend around invoke");
        }
        let e2e = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/e2e.mjs")).unwrap();
        for name in names(r#"\binvoke\(\w+, "([a-z_]+)""#, &e2e) {
            assert!(find(&name).is_some(), "the end-to-end test calls {name}, which is not an operation");
        }
        let listed = include_str!("../../docs/llms.txt").lines().skip_while(|l| !l.starts_with("POST /v1/ops/{name}")).skip(2).take_while(|l| l.starts_with("  ")).collect::<Vec<_>>().join("\n");
        let listed = regex::Regex::new(r"\{[^}]*\}|\([^)]*\)").unwrap().replace_all(&listed, "").to_string();
        let listed: Vec<&str> = listed.split(|c| c == ',' || c == ':' || c == '\n').filter_map(|item| item.split_whitespace().next()).collect();
        for (i, (name, _)) in OPS.iter().enumerate() {
            assert!(OPS[..i].iter().all(|(other, _)| other != name), "{name} is listed twice");
            assert!(listed.contains(name), "{name} is not in the operations of docs/llms.txt");
        }
    }

    #[test]
    fn arguments_are_read_by_name() {
        let args = json!({"id": "a", "n": 3, "gone": null});
        assert_eq!(arg::<String>(&args, "id"), Ok("a".to_string()));
        assert_eq!((arg::<Option<String>>(&args, "gone"), arg::<Option<u32>>(&args, "absent")), (Ok(None), Ok(None)));
        assert_eq!(arg::<String>(&args, "n"), Err("n is missing or not valid.".into()));
        assert_eq!(arg::<bool>(&args, "absent"), Err("absent is missing or not valid.".into()));
        assert_eq!(arg::<String>(&Value::Null, "id"), Err("id is missing or not valid.".into()));
    }

    #[test]
    fn ui_commands_take_only_the_arguments_the_window_reported() {
        let commands = json!({"play": {"about": "Play", "args": {}}, "seek": {"args": {"seconds": "number"}}, "set_rate": {"args": {"rate": [0.5, 1, 2]}}, "set_view": {"args": {"index": "integer"}}, "full_screen": {"args": {"on": "boolean"}}, "open_recording": {"args": {"set": ["a", "b"]}}, "odd": {"args": {"x": {}}}});
        let check = |cmd: &str, args: Value| check_ui_args(&commands, cmd, &args);
        assert_eq!(check("play", json!({})), Ok(()));
        assert_eq!(check("seek", json!({"seconds": 12.5})), Ok(()));
        assert_eq!(check("set_rate", json!({"rate": 1.0})), Ok(()));
        assert_eq!(check("set_view", json!({"index": 2})), Ok(()));
        assert_eq!(check("full_screen", json!({"on": false})), Ok(()));
        assert_eq!(check("open_recording", json!({"set": "b"})), Ok(()));
        assert_eq!(check("jump", json!({})), Err("unknown cmd jump; ui_commands lists them".into()));
        assert_eq!(check("toString", json!({})), Err("unknown cmd toString; ui_commands lists them".into()));
        assert_eq!(check("play", json!({"now": true})), Err("play takes no argument now.".into()));
        assert_eq!(check("play", json!([])), Err("args is not an object.".into()));
        assert_eq!(check("seek", json!({})), Err("seconds is missing or not valid.".into()));
        assert_eq!(check("seek", json!({"seconds": "5"})), Err("seconds is missing or not valid.".into()));
        assert_eq!(check("set_rate", json!({"rate": 1.1})), Err("rate is missing or not valid.".into()));
        assert_eq!(check("set_view", json!({"index": 1.5})), Err("index is missing or not valid.".into()));
        assert_eq!(check("full_screen", json!({"on": 1})), Err("on is missing or not valid.".into()));
        assert_eq!(check("open_recording", json!({"set": "c"})), Err("set is missing or not valid.".into()));
        assert_eq!(check("odd", json!({"x": {}})), Err("x is missing or not valid.".into()));
        assert_eq!(check_ui_args(&Value::Null, "play", &json!({})), Err("the window has not reported its commands yet".into()));
    }

    #[test]
    fn displays_get_their_size_and_windows_do_not() {
        let list = vec![json!({"id": {"kind": "window", "exe": "a.exe", "title": "A"}}), json!({"id": {"kind": "display", "index": 2}}), json!({"id": {"kind": "display", "index": 3}})];
        let out = with_sizes(list, &[(1920, 1080), (2560, 1440)]);
        assert!(out[0]["width"].is_null());
        assert_eq!((out[1]["width"].as_u64(), out[1]["height"].as_u64()), (Some(2560), Some(1440)));
        assert!(out[2]["width"].is_null());
    }
}

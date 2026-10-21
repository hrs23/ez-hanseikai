use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use iroh::Watcher;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::{api, audio, check, clean_name, clean_note, clean_text, data, debug, export, ffmpeg, jobs, merge, net, now_us, party, record, relay, room, shell, sources, Core, BOARD_MAX, CHANNELS, EVENT_KINDS, NAME_MAX, NOTE_MAX, REPO, VALUES_MAX};

pub type Op = fn(&Arc<Core>, Value) -> Result<Value, String>;

pub const OPS: &[(&str, Op)] = &[
    ("create_room", create_room),
    ("join_room", join_room),
    ("enter_room", enter_room),
    ("close_room", close_room),
    ("remove_room", remove_room),
    ("rename_room", rename_room),
    ("set_room_icon", set_room_icon),
    ("create_invite", create_invite),
    ("cancel_invites", cancel_invites),
    ("answer_ask", answer_ask),
    ("remove_peer", remove_peer),
    ("check_peer", check_peer),
    ("measure_speed", measure_speed),
    ("start_recording", start_recording),
    ("stop_recording", stop_recording),
    ("mark", mark),
    ("list_sources", list_sources),
    ("set_source", set_source),
    ("list_mics", list_mics),
    ("set_mic", set_mic),
    ("set_mix", set_mix),
    ("set_quality", set_quality),
    ("set_limit", set_limit),
    ("set_relays", set_relays),
    ("set_codecs", set_codecs),
    ("set_name", set_name),
    ("set_profile", set_profile),
    ("set_language", set_language),
    ("set_hotkey", set_hotkey),
    ("set_channel", set_channel),
    ("set_layout", set_layout),
    ("check_update", check_update),
    ("install_update", install_update),
    ("data_info", data_info),
    ("delete_old_files", delete_old_files),
    ("reset", reset),
    ("dismiss_recovery", dismiss_recovery),
    ("retry_ffmpeg", retry_ffmpeg),
    ("add_event", add_event),
    ("event_action", event_action),
    ("delete_ai_notes", delete_ai_notes),
    ("put_board_card", put_board_card),
    ("open_board_window", open_board_window),
    ("set_fullscreen", set_fullscreen),
    ("log_layout", log_layout),
    ("party_on", party_on),
    ("party_act", party_act),
    ("party_typing", party_typing),
    ("draw", draw),
    ("room_plugins", jobs::room_plugins),
    ("set_room_plugin", jobs::set_room_plugin),
    ("set_plugin_runner", jobs::set_plugin_runner),
    ("prepare_plugin", jobs::prepare_plugin),
    ("retry_job", jobs::retry_job),
    ("delete_set", delete_set),
    ("download_set", download_set),
    ("focus_set", focus_set),
    ("export_check", export_check),
    ("export_folder", export_folder),
    ("set_api", set_api),
    ("set_api_lan", set_api_lan),
    ("new_api_token", new_api_token),
    ("api_info", api_info),
    ("api_reference", api_reference),
    ("state", state),
    ("debug_view", debug::debug_view),
    ("debug_report", debug::debug_report),
];

pub fn find(name: &str) -> Option<Op> {
    OPS.iter().find(|(known, _)| *known == name).map(|(_, op)| *op)
}

pub fn arg<T: DeserializeOwned>(args: &Value, name: &str) -> Result<T, String> {
    serde_json::from_value(args[name].clone()).map_err(|_| format!("{name} is missing or not valid."))
}

fn recording(args: &Value) -> Result<(String, String), String> {
    let (room, set): (String, String) = (arg(args, "room")?, arg(args, "set")?);
    if room::valid_id(&room) && room::valid_id(&set) { Ok((room, set)) } else { Err("That recording is not on this PC.".into()) }
}

pub fn warm(core: &Core) {
    let wait = Duration::from_secs(6);
    tauri::async_runtime::block_on(async {
        let _ = tokio::time::timeout(wait, core.ep.online()).await;
        let _ = tokio::time::timeout(wait, core.ep.net_report().initialized()).await;
    });
}

fn create_room(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    room::create_room(core, &arg::<String>(&args, "name")?).map(|id| json!(id))
}

fn join_room(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    room::join_room(core, &arg::<String>(&args, "code")?).map(|_| Value::Null)
}

fn enter_room(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    room::enter_room(core, &arg::<String>(&args, "id")?).map(|_| Value::Null)
}

fn close_room(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    room::leave_room(core, &arg::<String>(&args, "id")?);
    Ok(Value::Null)
}

fn remove_room(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let id: String = arg(&args, "id")?;
    if !core.rooms.lock().unwrap().list.iter().any(|r| r.id == id) {
        return Err("That room is not in your list.".into());
    }
    if core.export.lock().unwrap().as_ref().is_some_and(|e| e.set.0 == id) {
        return Err("This recording is being exported. Cancel the export first.".into());
    }
    room::forget_room(core, &id);
    jobs::release_room(&id);
    core.removed_rooms.lock().unwrap().insert(id);
    room::sweep_removed(core);
    Ok(Value::Null)
}

fn rename_room(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    room::rename_room(core, &arg::<String>(&args, "name")?).map(|_| Value::Null)
}

fn set_room_icon(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    room::set_room_icon(core, &arg::<String>(&args, "icon")?).map(|_| Value::Null)
}

fn create_invite(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let reuse: bool = arg(&args, "reuse")?;
    let room = core.active_room_id();
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
    net::broadcast(core, net::asks_message(core));
    Ok(Value::Null)
}

fn answer_ask(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    net::answer(core, &arg::<String>(&args, "id")?, arg(&args, "approve")?);
    Ok(Value::Null)
}

fn remove_peer(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    room::remove_member(core, &arg::<String>(&args, "id")?);
    Ok(Value::Null)
}

fn measure_speed(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let id: String = arg(&args, "id")?;
    tauri::async_runtime::block_on(net::measure_speed(core, &id)).ok_or_else(|| "No answer".into())
}

fn check_peer(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let id: String = arg(&args, "id")?;
    if id != core.me() {
        return Ok(tauri::async_runtime::block_on(net::check_peer(core, &id)));
    }
    warm(core);
    Ok(json!(check::run(core)))
}

fn start_recording(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    room::start_set(core, arg(&args, "debrief")?).map(|_| Value::Null)
}

fn stop_recording(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    room::stop_set(core);
    Ok(Value::Null)
}

fn log_event(core: &Arc<Core>, room: &str, kind: &str, text: &str, target: Value, values: Value) -> Value {
    let id = now_us();
    core.append_log(room, &room::user_event(id, kind, text, target, values));
    party::send(core);
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

fn list_sources(_: &Arc<Core>, _: Value) -> Result<Value, String> {
    Ok(json!(with_sizes(sources::list(), &display_sizes())))
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

fn list_mics(_: &Arc<Core>, _: Value) -> Result<Value, String> {
    Ok(json!(audio::list_mics()))
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
    *core.limit_mbps.lock().unwrap() = arg::<Option<f64>>(&args, "mbps")?.filter(|v| *v > 0.0);
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
    shell::rebuild_menus(&core.app, core);
    Ok(Value::Null)
}

fn set_hotkey(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    shell::set_hotkey(core, &arg::<String>(&args, "keys")?).map(|_| Value::Null)
}

fn set_channel(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let channel: String = arg(&args, "channel")?;
    if !CHANNELS.contains(&channel.as_str()) {
        return Err(format!("unknown channel {channel}"));
    }
    *core.channel.lock().unwrap() = channel;
    core.save_settings();
    Ok(Value::Null)
}

fn set_layout(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    *core.layout.lock().unwrap() = args["layout"].clone();
    core.save_settings();
    room::log_set_layout(core);
    Ok(Value::Null)
}

fn update_allowed(core: &Core) -> Result<(), String> {
    if core.export.lock().unwrap().is_some() {
        return Err("Wait for the export to finish before updating.".into());
    }
    Ok(())
}

pub fn update_url(channel: &str) -> String {
    let file = if channel == CHANNELS[1] { "latest-beta.json" } else { "latest.json" };
    format!("{REPO}/releases/download/updater/{file}")
}

fn find_update(core: &Core) -> Result<Option<tauri_plugin_updater::Update>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let channel = core.channel.lock().unwrap().clone();
    let url = tauri::Url::parse(&update_url(&channel)).map_err(|e| e.to_string())?;
    let updater = core.app.updater_builder().endpoints(vec![url]).and_then(|b| b.build()).map_err(|e| e.to_string())?;
    tauri::async_runtime::block_on(updater.check()).map_err(|e| e.to_string())
}

fn check_update(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    Ok(json!(find_update(core)?.map(|u| u.version)))
}

fn install_update(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    update_allowed(core)?;
    let update = find_update(core)?.ok_or("No update is available.")?;
    let bytes = tauri::async_runtime::block_on(update.download(|_, _| {}, || {})).map_err(|e| e.to_string())?;
    update_allowed(core)?;
    tracing::info!("installing version {}", update.version);
    data::resume_later(core);
    jobs::stop_and_wait(core);
    data::unlock(&core.dir);
    let installed = update.install(bytes).map_err(|e| e.to_string());
    if installed.is_err() {
        let _ = data::lock(&core.dir);
        data::forget_resume(&core.dir);
    }
    installed.map(|_| Value::Null)
}

fn data_info(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    Ok(json!({"dir": core.dir, "used_bytes": data::used_bytes(&core.dir), "old_bytes": data::old_bytes(&core.dir), "free_bytes": core.free_bytes.load(Ordering::Relaxed)}))
}

fn delete_old_files(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    data::delete_old_files(core).map(|freed| json!({"freed": freed}))
}

fn reset(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let keep_rooms: bool = arg(&args, "keep_rooms")?;
    tracing::info!("resetting the app{}", if keep_rooms { ", keeping the rooms" } else { "" });
    room::end_set(core, false, None);
    export::cancel(core);
    jobs::stop_and_wait(core);
    data::unlock(&core.dir);
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
    let (kind, text): (String, String) = (arg(&args, "kind")?, arg(&args, "text")?);
    let (target, values) = (args["target"].clone(), args["values"].clone());
    if !EVENT_KINDS.contains(&kind.as_str()) || !target.is_object() || values.to_string().len() > VALUES_MAX {
        return Err(format!("not a valid {kind} event"));
    }
    let text = clean_note(&text, if kind == "board" { BOARD_MAX } else { NOTE_MAX });
    if kind == "note" && text.trim().is_empty() {
        return Err("Write something first.".into());
    }
    Ok(log_event(core, &core.active_room_id(), &kind, &text, target, values))
}

fn event_action(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let mut record = match arg::<String>(&args, "kind")?.as_str() {
        "edit" => json!({"k": "event_edit"}),
        "delete" => json!({"k": "event_del"}),
        "restore" => json!({"k": "event_restore"}),
        other => return Err(format!("unknown kind {other}")),
    };
    let text = clean_note(&arg::<String>(&args, "text")?, BOARD_MAX);
    merge(&mut record, &json!({"id": arg::<i64>(&args, "id")?, "t_us": now_us(), "text": text, "tags": room::tags(&text)}));
    core.append_log(&core.active_room_id(), &record);
    party::send(core);
    Ok(Value::Null)
}

fn delete_ai_notes(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let (room, set) = recording(&args)?;
    core.append_log(&room, &json!({"k": "ai_del", "set": set, "t_us": now_us()}));
    party::send(core);
    Ok(Value::Null)
}

fn put_board_card(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let room: String = arg(&args, "room")?;
    if room != core.active_room_id() {
        return Err("Enter the room first.".into());
    }
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
    let mut values = json!({"card": card, "pos": pos, "gone": gone});
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

fn set_fullscreen(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    shell::set_fullscreen(&core.app, &arg::<String>(&args, "window")?, arg(&args, "on")?).map(|_| Value::Null)
}

fn log_layout(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    core.append_log(&core.active_room_id(), &json!({"k": "layout", "t_us": now_us(), "layout": args["layout"]}));
    Ok(Value::Null)
}

fn party_on(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let me = core.me();
    core.party.lock().unwrap().turn(arg(&args, "on")?, &me, &args["view"], &core.active_room_id(), now_us());
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
    if core.party.lock().unwrap().on {
        net::broadcast(core, json!({"t": "draw", "room": core.active_room_id(), "stroke": args["stroke"]}));
    }
    Ok(Value::Null)
}

fn delete_set(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    room::delete_set(core, &arg::<String>(&args, "room")?, &arg::<String>(&args, "set")?).map(|deleted| json!(deleted))
}

fn download_set(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    room::download_set(core, &arg::<String>(&args, "room")?, &arg::<String>(&args, "set")?).map(|_| Value::Null)
}

fn focus_set(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    *core.focus.lock().unwrap() = arg(&args, "set")?;
    Ok(Value::Null)
}

fn export_check(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    export::check(core).map(|_| Value::Null)
}

fn export_folder(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    let (room, set) = recording(&args)?;
    export::folder(core, &room, &set, Path::new(&arg::<String>(&args, "folder")?))
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

fn set_api_lan(core: &Arc<Core>, args: Value) -> Result<Value, String> {
    *core.api_lan.lock().unwrap() = arg(&args, "lan")?;
    core.save_settings();
    api::apply(core);
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
    let lan = *core.api_lan.lock().unwrap();
    Ok(json!({"on": on, "lan": lan, "address": api::address(), "hosts": if on && lan { api::lan_addresses() } else { Vec::new() }, "token": if on { core.api_token.lock().unwrap().clone() } else { String::new() }, "error": core.api_error.lock().unwrap().clone()}))
}

fn api_reference(_: &Arc<Core>, _: Value) -> Result<Value, String> {
    Ok(json!(api::reference()))
}

fn state(core: &Arc<Core>, _: Value) -> Result<Value, String> {
    Ok(room::state_json(core))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names;

    const WINDOW_COMMANDS: [&str; 6] = ["identity", "open_set_folder", "open_data_folder", "open_link", "pick_folder", "save_text"];

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
    fn displays_get_their_size_and_windows_do_not() {
        let list = vec![json!({"id": {"kind": "window", "exe": "a.exe", "title": "A"}}), json!({"id": {"kind": "display", "index": 2}}), json!({"id": {"kind": "display", "index": 3}})];
        let out = with_sizes(list, &[(1920, 1080), (2560, 1440)]);
        assert!(out[0]["width"].is_null());
        assert_eq!((out[1]["width"].as_u64(), out[1]["height"].as_u64()), (Some(2560), Some(1440)));
        assert!(out[2]["width"].is_null());
    }

    #[test]
    fn each_update_channel_reads_its_own_manifest() {
        assert!(update_url("stable").ends_with("/updater/latest.json"));
        assert!(update_url("beta").ends_with("/updater/latest-beta.json"));
        assert_eq!(update_url("nonsense"), update_url("stable"));
    }
}

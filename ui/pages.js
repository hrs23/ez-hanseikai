const SHORTCUTS = [
  ["Live / Game plan / Members / Extensions", ["Ctrl+1", "Ctrl+2", "Ctrl+3", "Ctrl+4"]],
  ["Sidebar", ["Ctrl+B"]],
  ["Settings", ["Ctrl+,"]],
  ["Play / pause", ["Space", "K"]],
  ["Back / forward 5 s (Shift 1 s)", ["←", "→"]],
  ["Back / forward 10 s (Shift 1 min)", ["J", "L"]],
  ["Back / forward 1 min", ["Ctrl+←", "Ctrl+→"]],
  ["Slower / faster", ["<", ">"]],
  ["Mute / volume", ["M", "↑", "↓"]],
  ["Player full screen / leave", ["F", "Esc"]],
  ["Full screen", ["F11"]],
  ["All players / one player", ["0", "1–6", "Tab"]],
  ["Previous / next recording", ["Shift+P", "Shift+N"]],
  ["Add note", ["C"]],
  ["Microphone / screen while recording", ["M", "V"]],
  ["Previous / next note", ["Q", "E"]],
  ["Previous / next match (or event)", ["A", "D"]],
  ["Drawing tool: pen, laser, off / clear drawings", ["P", "X"]],
  ["Undo", ["Ctrl+Z"]],
  ["Shortcuts", ["?"]],
];

let askName = false;
let roomTab = "members";
let drawerFor = null;
let speed = null;
let settingsPane = "general";
let storageInfo = null;
let storageFailed = false;
let picked = new Set();
let flow = null;
let flowAt = 0;

function renderStart() {
  if (inRoom()) return;
  $("brand").textContent = st.app;
  const pendingRoom = st.room && st.room.pending;
  if (!st.name) askName = true;
  $("namefield").hidden = !askName;
  $("nameshow").hidden = askName;
  if (!askName && changed("startname", [st.name, look(st.id)])) $("nameshow").replaceChildren(nameChip(avatar(st.id, "lg"), st.name, $("nameedit")));
  if (document.activeElement !== $("startname")) $("startname").value = st.name;
  const named = !!st.name || !!cleanText($("startname").value, LIMITS.name);
  $("startpending").hidden = !pendingRoom;
  $("startform").hidden = !!pendingRoom;
  $("pendingtext").textContent = pendingRoom ? (st.room.waiting ? t("Waiting for approval") : t("Connecting…")) : "";
  $("newroom").disabled = !named;
  $("join").disabled = !named || !$("joincode").value.trim();
  const others = st.rooms.filter((r) => !r.id.startsWith("pending-")).sort((a, b) => (b.used_us || 0) - (a.used_us || 0));
  $("roomsbox").hidden = !others.length;
  if (!changed("rooms", [others.map((r) => [r.id, r.name, stamp(r.icon), r.used_us]), lang])) return;
  $("roomlist").replaceChildren(...others.map((r, i) => {
    const enter = () => run("enter_room", { id: r.id }, openLive);
    const more = iconButton("more", t("More"), (e) => { e.stopPropagation(); openMenu([{ label: t("Delete room…"), icon: "trash", danger: true, run: () => deleteRoom(r) }], e.currentTarget); }, "icon sm more");
    const row = el("div", { className: "rrow", tabIndex: 0, role: "button" }, nameChip(roomIcon(r, "lg"), r.name), el("span", { className: "sp" }), r.used_us ? el("span", { className: "muted small", textContent: formatWhen(new Date(r.used_us / 1000), new Date(), lang), title: fullWhen(new Date(r.used_us / 1000), lang) }) : null, el("button", { className: "sm" + (i ? "" : " primary"), textContent: t("Enter"), onclick: (e) => { e.stopPropagation(); enter(); } }), more);
    row.onclick = enter;
    row.onkeydown = (e) => { if (e.key === "Enter") enter(); };
    return row;
  }));
}
async function deleteRoom(room) {
  if (await ask(t("Leave {room} and delete its recordings and notes?", { room: room.name }), t("Delete"), true)) run("remove_room", { id: room.id });
}
async function closeRoom() {
  if (!st.room || (st.recording && !(await ask(t("Stop your recording and close the room?"), t("Close room"), true)))) return;
  run("close_room", { id: st.room.id }, openLive);
}
const joinWith = (input) => run("join_room", { code: input.value }, () => { input.value = ""; refresh(); });
const typedName = () => cleanText($("startname").value, LIMITS.name) || st.name;
const copyInvite = () => invoke("create_invite", { reuse: true }).then((code) => navigator.clipboard.writeText(code)).then(() => { flash(t("Invite code copied")); refresh(); }).catch(fail);
const cancelInvite = () => run("cancel_invites", null, () => { flash(t("Invite cancelled")); refresh(); });
const createRoom = () => invoke("create_room", { name: "" }).then(() => { openLive(); return refresh().then(copyInvite); }).catch(fail);
async function removeMember(id) {
  if (await ask(t("Remove {name} from the room?", { name: nameOf(id) }), t("Remove from room"), true)) { drawerFor = null; run("remove_peer", { id }); }
}
let iconDraft = "";
function roomDirty() {
  const name = cleanText($("roomnameinput").value, LIMITS.room);
  $("roomsave").disabled = !name || (name === st.room.name && iconDraft === st.room.icon);
}
const roomPresetBtns = [];
function showIconDraft() {
  $("roomiconshow").replaceChildren(roomIcon({ id: st.room.id, name: $("roomnameinput").value, icon: iconDraft }, "xl"));
  $("roomiconclear").hidden = !iconDraft;
  roomPresetBtns.forEach(([b, name]) => press(b, iconDraft === presetImage(name)));
  roomDirty();
}
function openRoomSettings() {
  if (!st.room) return;
  $("roomnameinput").value = st.room.name;
  iconDraft = st.room.icon;
  showIconDraft();
  $("roomdlg").showModal();
  $("roomnameinput").select();
}
function showRoomTab(tab) {
  if (!inRoom()) return;
  roomTab = tab;
  drawerFor = null;
  delete sigs.roomtab;
  go("room");
  renderRoom();
}

function renderRoom() {
  if (page !== "room" || !inRoom()) return;
  $("boardpane").hidden = roomTab !== "board";
  $("roomtab").parentElement.hidden = roomTab === "board";
  if (roomTab === "board") renderBoard();
  else if (roomTab === "members") renderMembers();
  else renderExtensions();
  renderDrawer();
}
function memberOf(id) {
  const me = id === st.id;
  const p = me ? null : peerOf(id);
  const status = memberStatus({ connected: me || !!(p && p.connected), sharing: me ? !!st.capture.source : !!(p && p.sharing), relayed: !!p && !sendsPicture(p), seen_us: p && p.seen_us }, new Date());
  return { p, status };
}
function renderMembers() {
  const rows = people().map((id) => ({ id, p: id === st.id ? null : peerOf(id) }));
  const sig = ["members", rows.map((r) => [r.id, memberOf(r.id).status, r.p && [r.p.connected, r.p.path, r.p.rtt_ms, r.p.version, r.p.failures]]), st.asks, st.invites, drawerFor, st.names, looks(), lang];
  if (!changed("roomtab", sig)) return;
  const out = [];
  for (const a of st.asks) {
    const answer = (approve) => () => run("answer_ask", { id: a.id, approve });
    out.push(el("div", { className: "askbox" }, picture(a.name, null, "", "lg"), stack(t("{name} wants to join", { name: a.name || longId(a.id) }), el("small", { className: "mono", textContent: longId(a.id) })), el("span", { className: "sp" }), el("button", { className: "primary", textContent: t("Approve"), onclick: answer(true) }), el("button", { textContent: t("Decline"), onclick: answer(false) })));
  }
  if (st.invites) out.push(el("div", { className: "invitebox" }, icon("link", "sm"), el("span", { textContent: t("An invite code is open") }), el("span", { className: "sp" }), el("button", { className: "sm", onclick: copyInvite }, icon("copy", "sm"), t("Copy")), el("button", { className: "sm", onclick: cancelInvite, textContent: t("Cancel invite") })));
  const head = el("tr", {}, ...["Name", "ID", "Status", "Connection", "Round trip", "Version", ""].map((h) => el("th", { textContent: h ? t(h) : "" })));
  const body = rows.map(({ id, p }) => {
    const me = id === st.id;
    const state = memberOf(id).status;
    const conn = me || !p.connected ? el("span", { className: "faint", textContent: "—" }) : el("span", { className: "net " + (p.path === "relay" ? "warn" : "ok"), onclick: (e) => { e.stopPropagation(); openDiagnostics(); } }, icon("relay", "sm"), p.path === "relay" ? t("Relay") : t("Direct"));
    const version = me ? st.version : p && p.version;
    const tr = el("tr", { className: "pick" + (drawerFor === id ? " sel" : "") + (state.kind === "off" ? " dim" : ""), tabIndex: 0 },
      el("td", {}, withClass(nameChip(me ? el("button", { className: "plain", ariaLabel: t("Avatar"), onclick: (e) => { e.stopPropagation(); openSettings("general"); } }, avatar(id, "", "on", false, true)) : avatar(id, "", p && p.connected ? "on" : null, false, true), nameOf(id), me ? el("span", { className: "pill gray", textContent: t("You") }) : null), "who")),
      el("td", {}, idText(id), iconButton("copy", t("Copy ID"), (e) => { e.stopPropagation(); navigator.clipboard.writeText(id).then(() => flash(t("ID copied"))).catch(fail); })),
      el("td", { className: "state " + state.kind, title: state.title || "" }, el("i", { className: "sdot" }), state.text),
      el("td", {}, conn),
      el("td", { textContent: !me && p.connected ? `${p.rtt_ms} ms` : "" }),
      el("td", { className: version && version !== st.version ? "warn" : "", textContent: version || "—" }),
      el("td", { className: "chev" }, icon("chevron-right", "sm")));
    const openIt = () => { drawerFor = drawerFor === id ? null : id; delete sigs.roomtab; renderRoom(); };
    tr.onclick = openIt;
    tr.onkeydown = (e) => { if (e.key === "Enter") openIt(); };
    return tr;
  });
  out.push(el("div", { className: "tablebox" }, el("table", { className: "table" }, el("thead", {}, head), el("tbody", {}, ...body))));
  $("roomtab").replaceChildren(...out);
}
const speedLine = (r) => [t("Up {n} Mbit/s", { n: r.up_mbps.toFixed(1) }), t("Down {n} Mbit/s", { n: r.down_mbps.toFixed(1) }), `${r.rtt_ms} ms`, r.path === "relay" ? t("Relay") : t("Direct")].join(" · ");
function measureSpeed(id) {
  speed = { id, line: null };
  delete sigs.drawer;
  renderDrawer();
  invoke("measure_speed", { id }).then(speedLine, () => t("No answer")).then((line) => { if (speed && speed.id === id) speed.line = line; delete sigs.drawer; renderDrawer(); });
}
function renderDrawer() {
  const box = $("drawer");
  const id = roomTab === "members" ? drawerFor : null;
  box.hidden = !id || !people().includes(id);
  if (box.hidden) { speed = null; return; }
  const me = id === st.id;
  const p = me ? null : peerOf(id);
  const receivingFrom = st.sets.filter((s) => !s.live).flatMap((s) => s.players.filter((x) => x.owner === id && x.total && x.have < x.total).map((x) => ({ s, x })));
  const sig = [id, p && [p.connected, p.path, p.rtt_ms, p.sent, p.received, p.offset_us, p.version], speed, receivingFrom.map((r) => [r.s.id, r.x.have]), st.names, look(id), lang];
  if (!changed("drawer", sig)) return;
  const kv = [];
  const add = (k, v) => kv.push(el("dt", { textContent: k }), el("dd", { textContent: v }));
  let path = null;
  if (p && p.connected) {
    path = p.path === "relay" ? el("div", { className: "pathbox relay" }, el("b", {}, icon("relay", "sm"), t("Via relay server")), el("small", { textContent: [p.relay, t("end-to-end encrypted")].filter(Boolean).join(" · ") })) : el("div", { className: "pathbox direct" }, el("b", {}, icon("relay", "sm"), t("Direct")), el("small", { textContent: t("end-to-end encrypted") }));
    add(t("Round trip"), `${p.rtt_ms} ms`);
    if (p.sent != null) add(t("Sent"), bytes(p.sent));
    if (p.received != null) add(t("Received"), bytes(p.received));
    if (p.offset_us != null) add(t("Clock offset"), `${p.offset_us >= 0 ? "+" : "−"}${(Math.abs(p.offset_us) / 1e6).toFixed(2)} s`);
  } else if (!me) path = el("div", { className: "pathbox off" }, el("b", { textContent: memberOf(id).status.text, title: memberOf(id).status.title || "" }));
  add(t("Version"), (me ? st.version : p && p.version) || "—");
  const measuring = !!speed && speed.id === id && !speed.line;
  if (speed && speed.id === id && speed.line) add(t("Speed"), speed.line);
  box.replaceChildren(
    el("div", { className: "dhead" }, avatar(id, "xl"), stack(nameOf(id), idText(id, "small"), true), iconButton("x", t("Close"), () => { drawerFor = null; delete sigs.roomtab; renderRoom(); })),
    path || "",
    el("dl", { className: "kv" }, ...kv),
    receivingFrom.length ? el("div", { className: "h3", textContent: t("Receiving") }) : "",
    ...receivingFrom.map(({ s, x }) => el("div", { className: "recvrow" }, el("div", { className: "row" }, el("span", { textContent: sessionName(s.id) }), el("span", { className: "sp" }), el("span", { className: "muted", textContent: `${Math.floor((x.have / x.total) * 100)}%` })), el("div", { className: "bar" }, el("i", { style: `width:${(x.have / x.total) * 100}%` })))),
    me ? "" : el("div", { className: "row" }, el("button", { className: "sm", disabled: measuring || !(p && p.connected), onclick: () => measureSpeed(id) }, icon("activity", "sm"), measuring ? t("Measuring…") : t("Measure speed"))),
    el("span", { className: "sp" }),
    me ? "" : el("button", { className: "danger", onclick: () => removeMember(id), textContent: t("Remove from room…") }),
  );
}
function openSettings(pane) {
  if (!st) return;
  if (pane) settingsPane = pane;
  $("name").value = st.name;
  $("language").value = st.language;
  $("sessionsound").checked = sessionSound;
  pressVoice();
  $("relays").value = st.relays.join("\n");
  renderHotkeys();
  $("myid").textContent = longId(st.id);
  $("myid").title = st.id;
  invoke("data_info").then((info) => { $("datadir").textContent = info.dir; }).catch(warn);
  $("version").textContent = st.version;
  if (!$("settings").open) $("settings").showModal();
  showPane();
}
function showPane() {
  for (const b of $("setnav").querySelectorAll("button")) b.classList.toggle("on", b.dataset.pane === settingsPane);
  for (const s of $("settings").querySelectorAll("section[data-pane]")) s.hidden = s.dataset.pane !== settingsPane;
  if (settingsPane === "storage") loadStorage();
  if (settingsPane === "shortcuts") renderKeys();
  renderSettings();
}
function renderKeys() {
  $("keylist").replaceChildren(...SHORTCUTS.flatMap(([label, keys]) => [el("dt", { textContent: t(label) }), el("dd", {}, ...keys.map((k) => el("kbd", { textContent: k })))]));
}
function renderHotkeys() {
  $("hotkeys").replaceChildren(...hotkeyRows(st.hotkeys).map((h) => {
    const keys = el("button", { className: "keys w240", textContent: h.keys });
    const reset = el("button", { className: "icon sm", title: t("Reset to default"), ariaLabel: t("Reset to default"), disabled: !h.changed }, icon("rotate-ccw", "sm"));
    const err = el("span", { className: "err", hidden: !h.warning, textContent: t(h.warning) });
    const save = (value) => invoke("set_hotkey", { action: h.action, keys: value }).then(refresh).then(renderHotkeys).catch((e) => { keys.textContent = h.keys; err.textContent = t(String(e)); err.hidden = false; });
    keys.onclick = () => { listening = true; keys.classList.add("listening"); keys.textContent = t("Press the keys…"); };
    keys.onblur = () => { if (listening) { listening = false; keys.classList.remove("listening"); keys.textContent = h.keys; } };
    keys.onkeydown = (e) => {
      if (!listening) return;
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") { keys.blur(); return; }
      const k = keysOf(e);
      if (!k) return;
      listening = false;
      keys.classList.remove("listening");
      keys.textContent = k;
      save(k);
    };
    reset.onclick = () => save(h.default);
    return el("div", { className: "srow2" + (h.warning ? " warn" : "") }, el("span", { className: "lab", textContent: t(h.label) }), keys, reset, err);
  }));
}
function renderSettings() {
  if (!$("settings").open) return;
  const q = st.capture.quality;
  pick($("quality"), q.id, (id) => run("set_quality", { id }));
  $("qinfo").textContent = [q.width ? `${q.width} × ${q.height}` : "", `${q.fps} fps`, `${(q.kbps / 1000).toFixed(1)} Mbit/s`, String(q.codec).toUpperCase()].filter(Boolean).join(" · ");
  if (document.activeElement !== $("limit")) $("limit").value = st.limit_mbps || "";
  if (document.activeElement !== $("apion")) $("apion").checked = st.api.on;
  if (document.activeElement !== $("notifyjoins")) $("notifyjoins").checked = st.notify_joins;
  $("apirows").hidden = !st.api.on;
  $("apiaddr").textContent = apiUrl(st.api.address);
  if (st.api.on && settingsPane === "api") renderApiReference();
  $("apierrrow").hidden = !st.api.error;
  $("apierr").textContent = st.api.error ? t(st.api.error) : "";
  if (settingsPane === "storage") renderStorage();
  if (changed("profile", look(st.id))) renderProfile();
}
function renderProfile() {
  const mine = colorIndex(st.id);
  const image = st.avatars[st.id] || "";
  $("colorpick").replaceChildren(...MEMBER_COLORS.map((_, i) => {
    const b = el("button", { className: "swatch", ariaLabel: `${t("Color")} ${i + 1}`, onclick: () => run("set_profile", { color: i }) });
    b.style.background = colorOf(i);
    press(b, i === mine);
    return b;
  }));
  const own = IMAGE_URL.test(image) ? [press(el("button", { className: "facebtn", ariaLabel: t("Avatar") }, picture("", colorOf(mine), image, "lg")), true)] : [];
  $("facegrid").replaceChildren(...own, ...[PERSON, ...PRESETS].map((name, i) => {
    const b = el("button", { className: "facebtn", ariaLabel: `${t("Avatar")} ${i + 1}`, onclick: () => run("set_profile", { avatar: name === PERSON ? "" : name }) }, picture("", colorOf(mine), name, "lg"));
    press(b, name === PERSON ? !image : image === name);
    return b;
  }));
}
function renderApiReference() {
  if ($("apiref").children.length) return;
  invoke("api_reference").then((list) => {
    $("apiref").replaceChildren(...list.map((e) => el("details", { className: "apiep" }, el("summary", { textContent: e.title }), el("div", { className: "apibody" }, ...apiReferenceBlocks(e.lines).map((b) => b.code ? el("pre", { textContent: b.text }) : el("p", { className: "muted small", textContent: b.text }))))));
  }).catch(fail);
}
function loadStorage() {
  invoke("data_info").then((info) => { storageInfo = info; storageFailed = false; }, (e) => { warn(e); storageFailed = true; }).then(() => { delete sigs.storage; renderStorage(); });
}
function recvText(s) {
  const recv = receiving(s);
  return recv && !recv.stuck ? progressText(recv) : "";
}
function renderStorage() {
  const d = st.disk;
  const used = storageInfo ? storageInfo.used_bytes : 0;
  const totalBytes = d.total || 1;
  const other = Math.max(0, totalBytes - d.free - used);
  const sets = st.sets.filter((s) => !s.live);
  const sig = [used, !!storageInfo, storageFailed, d, sets.map((s) => [s.id, s.progress.have, s.progress.total, s.shared, recvText(s)]), [...picked], lang];
  if (!changed("storage", sig)) return;
  $("storedir").textContent = storageInfo ? storageInfo.dir : "";
  const seg = (cls, n) => { const i = el("i", { className: cls }); i.style.width = (n / totalBytes) * 100 + "%"; return i; };
  $("usage").replaceChildren(seg("app", used), seg("other", other));
  const free = d.hours_left != null ? t("Free {free} · about {h} h of recording", { free: bytes(d.free), h: Math.floor(d.hours_left) }) : t("Free {free}", { free: bytes(d.free) });
  const state = loadState(!!storageInfo, storageFailed, "", loadStorage);
  $("legend").replaceChildren(...(state ? [state] : [el("span", {}, el("i", { className: "app" }), `${st.app} ${bytes(used)}`), el("span", {}, el("i", { className: "other" }), t("Other files {size}", { size: bytes(other) })), el("span", { className: "sp" }), el("span", {}, el("i", { className: "free" }), free)]));
  $("selold").textContent = t("All but newest {n}", { n: KEEP_NEWEST });
  for (const id of [...picked]) if (!sets.some((s) => s.id === id)) picked.delete(id);
  const biggest = Math.max(1, ...sets.map((s) => s.progress.have));
  const rows = sets.map((s) => {
    const box = el("input", { type: "checkbox", checked: picked.has(s.id), disabled: !canDelete(s), ariaLabel: sessionName(s.id) });
    box.onchange = () => { if (box.checked) picked.add(s.id); else picked.delete(s.id); delete sigs.storage; renderStorage(); };
    const notes = noteCount(s);
    return el("tr", { className: picked.has(s.id) ? "sel" : "" }, el("td", {}, box), el("td", {}, el("b", { textContent: sessionName(s.id), title: fullWhen(sessionStart(s.id)) }), " ", el("span", { className: "muted small", textContent: noPicture(s) ? t("voice only") : t(notes === 1 ? "{n} note" : "{n} notes", { n: notes }) })), el("td", { textContent: setLength(s) }), el("td", { textContent: String(s.players.length) }), el("td", {}, onThisPc(s) ? [el("span", { className: "sizebar" }, el("i", { style: `width:${(s.progress.have / biggest) * 100}%` })), bytes(s.progress.have)] : el("span", { className: "muted", textContent: "—" })), el("td", { className: "muted small", textContent: recvText(s) }));
  });
  const head = el("tr", {}, ...["", "Recorded", "Length", "People", "Size", ""].map((h) => el("th", { textContent: h ? t(h) : "" })));
  $("storetable").replaceChildren(rows.length ? el("table", { className: "table" }, el("thead", {}, head), el("tbody", {}, ...rows)) : el("div", { className: "empty", textContent: t("No recordings yet") }));
  const size = sets.filter((s) => picked.has(s.id)).reduce((n, s) => n + s.progress.have, 0);
  $("selsum").textContent = picked.size ? t("{n} selected · {size}", { n: picked.size, size: bytes(size) }) : "";
  $("deletesel").disabled = !picked.size;
}
function confirmDelete(ids) {
  const sets = ids.map(setById).filter((s) => s && canDelete(s));
  if (!sets.length) return;
  const preview = deletePreview(sets);
  const size = preview.reduce((n, p) => n + p.bytes, 0);
  $("deltitle").textContent = t(sets.length === 1 ? "Delete this recording?" : "Delete {n} recordings?", { n: sets.length });
  $("dellist").replaceChildren(...preview.map((p) => el("div", { className: "delcard" },
    el("div", { className: "row" }, el("b", { textContent: sessionName(p.id) }), el("span", { className: "sp" }), el("span", { textContent: bytes(p.bytes) })),
    el("div", { className: "who" }, ...p.players.map((x) => el("span", {}, icon("film", "sm"), x.owner === st.id ? t("{name} (you)", { name: nameOf(x.owner) }) : nameOf(x.owner), el("span", { className: "muted", textContent: bytes(x.bytes) })))),
    el("div", { className: "small" + (p.form === "Others still have it" ? " muted" : " warn"), textContent: t(p.form) }),
    p.debriefs.length ? el("div", { className: "small muted", textContent: t("Debriefs deleted with it: {names}", { names: p.debriefs.map((d) => sessionName(d)).join(", ") }) }) : null)));
  $("delyes").replaceChildren(icon("trash", "sm"), size ? t("Delete {size}", { size: bytes(size) }) : t("Delete"));
  $("delyes").onclick = async () => {
    $("deletedlg").close();
    for (const s of sets) await invoke("delete_set", { room: s.room, set: s.id }).catch(fail);
    for (const s of sets) picked.delete(s.id);
    if (sets.some((s) => s.id === selected)) selectSet(null);
    flash(t("Deleted"));
    refresh().then(loadStorage);
  };
  $("deletedlg").showModal();
}
async function deleteSession(set) {
  if (set) confirmDelete([set.id]);
}

function openDiagnostics() {
  if (!$("diagdlg").open) $("diagdlg").showModal();
  delete sigs.diag;
  renderDiagnostics();
}
const DIAG_FEED_MS = 2000;
function renderDiagnostics() {
  if (!$("diagdlg").open) return;
  if (Date.now() - flowAt >= DIAG_FEED_MS) {
    flowAt = Date.now();
    invoke("debug_flow").then((f) => { flow = f; delete sigs.diag; renderDiagnostics(); }).catch(warn);
  }
  if (!changed("diag", [st.peers, st.capture.sharing, flow, lang])) return;
  const peers = st.peers;
  const what = () => [st.capture.sharing ? t("Live picture and voice") : null, t("Recorded files"), t("Log")].filter(Boolean).join(" · ");
  const node = (id, sub, cls = "") => el("div", { className: "node " + cls }, avatar(id, "lg", null, false, true), stack(nameOf(id), sub));
  const links = peers.map((p) => el("div", { className: "flink " + (p.connected ? p.path : "off") }, el("span", { className: "line" }), p.connected && p.path === "relay" ? el("span", { className: "relaybox" }, el("b", { textContent: t("Relay server") }), el("small", { textContent: p.relay || t("content not visible") })) : el("span", { className: "label", textContent: p.connected ? `${t("Direct")} · ${t("encrypted")} · ${p.rtt_ms} ms` : t("Offline") }), node(p.id, "", p.connected ? "" : "dim")));
  const diagram = el("div", { className: "flowmap" }, node(st.id, ""), el("div", { className: "links" }, ...(links.length ? links : [el("div", { className: "none", textContent: t("No other members") })])));
  const head = el("tr", {}, ...["Peer", "Path", "What is sent", "Sent", "Received"].map((h) => el("th", { textContent: t(h) })));
  const rows = peers.map((p) => el("tr", { className: p.connected ? "" : "dim" }, el("td", {}, el("b", { textContent: nameOf(p.id) })), el("td", { className: p.connected ? (p.path === "relay" ? "warn" : "ok") : "faint", textContent: p.connected ? (p.path === "relay" ? [t("Relay"), p.relay].filter(Boolean).join(" ") : t("Direct")) : t("Offline") }), el("td", { textContent: p.connected ? what() : "—" }), el("td", { textContent: p.sent != null ? bytes(p.sent) : "—" }), el("td", { textContent: p.received != null ? bytes(p.received) : "—" })));
  $("diagbody").replaceChildren(el("div", { className: "okbox" }, ...flowRows(st, flow).map(okLine)), diagram, el("div", { className: "tablebox" }, el("table", { className: "table" }, el("thead", {}, head), el("tbody", {}, ...rows))));
}
function diagnosticsText() {
  return invoke("debug_report", { save: false });
}

function openReset() {
  $("keeprooms").checked = true;
  $("resetdlg").showModal();
}
function renderOpenDialogs() {
  renderSettings();
  renderDiagnostics();
}

for (const [id, max] of [["startname", LIMITS.name], ["name", LIMITS.name], ["roomnameinput", LIMITS.room], ["joincode", LIMITS.code], ["quicktext", LIMITS.note], ["relays", LIMITS.relays * (LIMITS.relay + 1)]]) $(id).maxLength = max;
$("limit").max = 100000;
$("startname").oninput = () => renderStart();
$("startname").onchange = () => { run("set_name", { name: cleanText($("startname").value, LIMITS.name) }); if (cleanText($("startname").value, LIMITS.name)) askName = false; };
$("nameedit").onclick = () => { askName = true; renderStart(); $("startname").focus(); };
$("newroom").onclick = single(() => run("set_name", { name: typedName() }, createRoom));
$("joincode").oninput = () => renderStart();
$("join").onclick = single(() => run("set_name", { name: typedName() }, () => joinWith($("joincode"))));
$("joincode").onkeydown = (e) => { if (e.key === "Enter" && !composing(e) && !$("join").disabled) $("join").click(); };
$("pendingcancel").onclick = () => st.room && run("remove_room", { id: st.room.id });
$("roomnameinput").oninput = showIconDraft;
$("roomiconpick").onclick = () => pickImage((image) => { iconDraft = image; showIconDraft(); });
roomPresetBtns.push(...ROOM_PRESETS.map((name, i) => [el("button", { className: "facebtn", ariaLabel: `${t("Room icon")} ${i + 1}`, onclick: () => { iconDraft = presetImage(name); showIconDraft(); } }, presetTile(name, "lg")), name]));
$("roomfacegrid").replaceChildren(...roomPresetBtns.map(([b]) => b));
$("roomiconclear").onclick = () => { iconDraft = ""; showIconDraft(); };
$("roomnameinput").onkeydown = (e) => { if (e.key === "Enter" && !composing(e) && !$("roomsave").disabled) $("roomsave").click(); };
$("roomsave").onclick = single(async () => {
  const name = cleanText($("roomnameinput").value, LIMITS.room);
  $("roomdlg").close();
  try {
    if (name !== st.room.name) await invoke("rename_room", { name });
    if (iconDraft !== st.room.icon) await invoke("set_room_icon", { icon: iconDraft });
    flash(t("Saved"));
    refresh();
  } catch (e) { fail(e); }
});
for (const b of $("setnav").querySelectorAll("button")) b.onclick = () => { settingsPane = b.dataset.pane; showPane(); };
$("prompttext").oninput = () => editPrompt(false);
$("promptreset").onclick = () => editPrompt(true);
$("promptcopy").onclick = () => navigator.clipboard.writeText(aiCopyText($("prompttext").value, promptToken)).then(() => { $("promptdlg").close(); flash(t("Prompt copied")); }).catch(fail);
$("apidoc").onclick = () => dialog("open_link", { url: `${apiUrl(st.api.address)}/llms.txt` }).catch(fail);
$("apion").onchange = () => run("set_api", { on: $("apion").checked });
$("notifyjoins").onchange = () => run("set_notify_joins", { on: $("notifyjoins").checked });
$("apicopy").onclick = () => invoke("api_info").then((info) => navigator.clipboard.writeText(info.token)).then(() => flash(t("Token copied"))).catch(fail);
$("apinew").onclick = async () => {
  if (await ask(t("Make a new token?"), t("New token"), false)) run("new_api_token", null, () => flash(t("New token made")));
};
$("name").onchange = () => { const name = cleanText($("name").value, LIMITS.name); run("set_name", { name: name || st.name }); };
$("avatarimage").onclick = () => pickImage((image) => run("set_profile", { avatar: image }));
$("language").onchange = () => run("set_language", { language: $("language").value });
function pressVoice() {
  pick($("voiceside"), voiceSide, (v) => { voiceSide = v; store("voiceSide", voiceSide); pressVoice(); applyAudio(); });
  pick($("voicetone"), voiceToneChoice, (v) => { voiceToneChoice = v; store("voiceTone", voiceToneChoice); pressVoice(); applyAudio(); });
}
$("sessionsound").onchange = () => { sessionSound = $("sessionsound").checked; store("sessionSound", sessionSound); };
$("copyid").onclick = () => navigator.clipboard.writeText(st.id).then(() => flash(t("ID copied"))).catch(fail);
$("opendata").onclick = $("storeopen").onclick = () => dialog("open_data_folder").catch(fail);
$("relays").onchange = () => run("set_relays", { urls: $("relays").value.split("\n").map((u) => cleanText(u, LIMITS.relay)) });
$("limit").onchange = () => run("set_limit", { mbps: $("limit").value ? Number($("limit").value) : null });
$("checkupdate").onclick = () => checkUpdate(true);
$("selnone").onclick = () => { picked.clear(); delete sigs.storage; renderStorage(); };
$("selold").onclick = () => { picked = new Set(oldSessions(st.sets.filter((s) => !s.live))); delete sigs.storage; renderStorage(); };
$("deletesel").onclick = () => confirmDelete([...picked]);
function renderCleanup(plan, picked) {
  const path = (p) => el("div", { className: "row small" }, el("span", { className: "mono ellip", textContent: p.path }), el("span", { className: "sp" }), el("span", { className: "muted", textContent: bytes(p.bytes) }));
  const box = (on, flip) => { const b = el("input", { type: "checkbox", checked: on }); b.onchange = () => { flip(b.checked); renderCleanup(plan, picked); }; return b; };
  const pick = (set, id) => (on) => { if (on) set.add(id); else set.delete(id); };
  const groups = plan.groups.map((g) => el("details", { className: "delcard" },
    el("summary", {}, box(picked.groups.has(g.id), pick(picked.groups, g.id)), " ", el("b", { textContent: cleanupHead(CLEANUP_GROUPS[g.id], g.count, g.bytes) }), g.id === "temp" ? el("div", { className: "small muted", textContent: t("Deleted, not moved to the Recycle Bin") }) : null),
    ...g.items.map(path), g.more ? el("div", { className: "small muted", textContent: t("+{n} more", { n: g.more }) }) : null));
  const folders = plan.folders.length || plan.kept.length ? [el("div", { className: "delcard" },
    el("b", { textContent: cleanupHead("Old folders", plan.folders.length, plan.folders.reduce((s, f) => s + f.bytes, 0)) }),
    ...plan.folders.map((f) => el("label", { className: "row small" }, box(picked.folders.has(f.path), pick(picked.folders, f.path)), el("span", { className: "mono ellip", textContent: f.path }), f.key ? el("span", { className: "warn", textContent: t("Contains an old ID key") }) : null, el("span", { className: "sp" }), el("span", { className: "muted", textContent: bytes(f.bytes) }))),
    ...plan.kept.map((p) => el("div", { className: "row small muted" }, el("span", { className: "mono ellip", textContent: p }), el("span", { className: "sp" }), el("span", { textContent: t("Kept: it holds files the app did not make") }))))] : [];
  $("cleanlist").replaceChildren(...groups, ...folders);
  const total = cleanupTotal(plan, picked);
  $("cleantotal").textContent = total.text;
  $("cleanyes").disabled = !total.n;
}
$("cleanup").onclick = async () => {
  const plan = await invoke("cleanup_plan").catch(fail);
  if (!plan) return;
  if (!plan.groups.length && !plan.folders.length && !plan.kept.length) { flash(t("Nothing to clean up")); return; }
  const picked = cleanupPicked(plan);
  renderCleanup(plan, picked);
  $("cleanyes").onclick = async () => {
    $("cleandlg").close();
    const done = await invoke("cleanup_run", { groups: [...picked.groups], folders: [...picked.folders] }).catch(fail);
    if (done) flash(cleanupDone(done));
    loadStorage();
  };
  $("cleandlg").showModal();
};
$("diagcopy").onclick = () => diagnosticsText().then((text) => navigator.clipboard.writeText(text)).then(() => flash(t("Copied"))).catch(fail);
$("diagsave").onclick = () => diagnosticsText().then((text) => dialog("save_text", { name: "ez-hanseikai-diagnostics.txt", text })).then((saved) => { if (saved) flash(t("Saved")); }).catch(fail);
$("resetyes").onclick = () => {
  $("resetdlg").close();
  const keepRooms = $("keeprooms").checked;
  run("reset", { keep_rooms: keepRooms }, null);
};
$("rfolder").onclick = () => dialog("open_data_folder").catch(fail);
$("rreport").onclick = () => run("debug_report", { save: true }, (p) => { $("rinfo").textContent = p; });
$("rreset").onclick = openReset;
$("rcontinue").onclick = () => run("dismiss_recovery");
for (const d of document.querySelectorAll("dialog")) {
  d.addEventListener("click", (e) => {
    const r = d.getBoundingClientRect();
    const outside = e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom;
    if ((e.target === d && outside) || e.target.closest(".close")) d.close();
  });
}

const menuActions = {
  "export-folder": () => exportFolder(playSet()),
  "settings": () => openSettings(),
  "copy-invite": copyInvite,
  "cancel-invite": () => { if (st.invites) cancelInvite(); },
  "rename-room": openRoomSettings,
  "leave-room": closeRoom,
  "add-note": openQuick,
  "delete-session": () => deleteSession(playSet()),
  "page-live": () => navigate("live"),
  "page-board": () => navigate("board"),
  "page-members": () => navigate("members"),
  "page-extensions": () => navigate("extensions"),
  "sidebar": toggleSidebar,
  "fullscreen": toggleFull,
  "shortcuts": () => openSettings("shortcuts"),
  "update": () => checkUpdate(true),
  "diagnostics": () => openDiagnostics(),
  "reset": openReset,
  "toggle-recording": toggleRecording,
};
listenMain("menu", (e) => { if (menuActions[e.payload]) menuActions[e.payload](); });
listenMain("joined", (e) => { flash(t("{name} joined", { name: e.payload })); if (document.hasFocus()) chime("join"); });
listenMain("session", (e) => {
  const debrief = e.payload.action === "end" ? debriefOpen : null;
  debriefOpen = null;
  flash(sessionNotice(e.payload.action, e.payload.by), "info", debrief && { label: t("Summarize with AI"), run: () => summarizeDebrief(debrief) });
  chime(e.payload.action);
  if (e.payload.action === "start" && !e.payload.voice) showRoomTab("board");
  const ended = e.payload.action === "end" && setById(e.payload.set);
  if (ended && !ended.of && page === "player" && showingNow()) openFromStart(ended.id);
});
listenMain("flash", (e) => flash(e.payload));

fillIcons();
if (boardWindow) {
  document.body.classList.add("boardwin");
  page = "room";
  roomTab = "board";
  $("boardpane").hidden = false;
  window.__TAURI__.event.listen("log", () => refresh());
  refresh();
} else {
  reportCodecs();
  setInterval(() => { if (!document.hidden) tick(); }, TICK_MS);
  setInterval(() => { if (!document.hidden) refresh().catch(warn); }, REFRESH_MS);
  setInterval(pollPlugins, PLUGINS_MS);
  refresh();
  setTimeout(() => checkUpdate(false), UPDATE_CHECK_DELAY_MS);
  setInterval(() => checkUpdate(false), UPDATE_EVERY_MS);
  window.__TAURI__.event.listen("shown", () => { if (Date.now() - checkedAt > UPDATE_RECHECK_MS) checkUpdate(false); });
}

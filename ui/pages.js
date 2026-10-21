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
  ["Full screen / leave", ["F", "Esc"]],
  ["All players / one player", ["0", "1–6", "Tab"]],
  ["Previous / next recording", ["Shift+P", "Shift+N"]],
  ["Add note", ["C"]],
  ["Previous / next note", ["Q", "E"]],
  ["Previous / next match (or event)", ["A", "D"]],
  ["Clear drawing", ["X"]],
  ["Undo", ["Ctrl+Z"]],
  ["Shortcuts", ["?"]],
];

let askName = false;
let joining = false;
let roomTab = "members";
let drawerFor = null;
let checks = {};
let checking = {};
let speed = null;
let settingsPane = "general";
let storageInfo = null;
let picked = new Set();
let diagTab = "summary";
let diagFeed = null;
let diagFeedAt = 0;

function renderStart() {
  if (inRoom()) return;
  $("brand").textContent = st.app;
  const pendingRoom = st.room && st.room.pending;
  if (!st.name) askName = true;
  $("namefield").hidden = !askName;
  $("nameshow").hidden = askName;
  if (!askName && changed("startname", [st.name, look(st.id)])) { $("nameav").replaceChildren(avatar(st.id, "lg")); $("nametext").textContent = st.name; }
  if (document.activeElement !== $("startname")) $("startname").value = st.name;
  const named = !!st.name || !!$("startname").value.trim();
  $("startpending").hidden = !pendingRoom;
  $("startform").hidden = !!pendingRoom;
  $("pendingtext").textContent = pendingRoom ? (st.room.waiting ? t("Waiting for approval") : t("Connecting…")) : "";
  $("newroom").disabled = !named;
  $("join").disabled = !named || !$("joincode").value.trim();
  const others = st.rooms.filter((r) => !r.id.startsWith("pending-")).sort((a, b) => (b.used_us || 0) - (a.used_us || 0));
  $("roomsbox").hidden = !others.length;
  $("roomsor").hidden = !others.length;
  if (!changed("rooms", [others.map((r) => [r.id, r.name, stamp(r.icon), r.used_us]), lang])) return;
  $("roomlist").replaceChildren(...others.map((r, i) => {
    const enter = () => run("enter_room", { id: r.id }, openLive);
    const more = iconButton("more", t("More"), (e) => { e.stopPropagation(); openMenu([{ label: t("Delete…"), icon: "trash", danger: true, run: () => deleteRoom(r) }], e.currentTarget); }, "icon sm more");
    const row = el("div", { className: "rrow", tabIndex: 0, role: "button" }, roomIcon(r, "lg"), el("b", { textContent: r.name }), el("span", { className: "sp" }), r.used_us ? el("span", { className: "muted small", textContent: dayLabel(dayOf(new Date(r.used_us / 1000)), new Date(), lang) }) : null, el("button", { className: "sm" + (i ? "" : " primary"), textContent: t("Enter"), onclick: (e) => { e.stopPropagation(); enter(); } }), more);
    row.onclick = enter;
    row.onkeydown = (e) => { if (e.key === "Enter") enter(); };
    return row;
  }));
}
async function deleteRoom(room) {
  if (await ask(t("Leave {room} and delete its recordings and notes on this PC? You need an invite code to come back. Other members keep theirs.", { room: room.name }), t("Delete"), true)) run("remove_room", { id: room.id });
}
const leaveRoom = () => st.room && run("close_room", { id: st.room.id }, openLive);
function joinWith(input) {
  if (joining) return;
  joining = true;
  run("join_room", { code: input.value }, () => { input.value = ""; refresh(); }).finally(() => { joining = false; });
}
const copyInvite = () => invoke("create_invite", { reuse: true }).then((code) => navigator.clipboard.writeText(code)).then(() => { flash(t("Invite code copied")); refresh(); }).catch(fail);
const cancelInvite = () => run("cancel_invites", null, () => { flash(t("Invite cancelled")); refresh(); });
const createRoom = () => invoke("create_room", { name: "" }).then(() => { openLive(); return refresh().then(copyInvite); }).catch(fail);
async function removeMember(id) {
  if (await ask(t("Remove {name} from the room?", { name: nameOf(id) }), t("Remove from room"), true, t("Everyone stops sending to them. What they already have stays with them."))) { drawerFor = null; run("remove_peer", { id }); }
}
let iconDraft = "";
function roomDirty() {
  const name = $("roomnameinput").value.trim();
  $("roomsave").disabled = !name || (name === st.room.name && iconDraft === st.room.icon);
}
function showIconDraft() {
  $("roomiconshow").replaceChildren(roomIcon({ name: $("roomnameinput").value, icon: iconDraft }, "xl"));
  $("roomiconclear").hidden = !iconDraft;
  roomDirty();
}
function openRoomSettings() {
  if (!st.room) return;
  $("roomnameinput").value = st.room.name;
  $("roomnameinput").maxLength = ROOM_NAME_MAX;
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

const ROOM_TITLES = { members: "Members", extensions: "Extensions", board: "Game plan" };

function renderRoom() {
  if (page !== "room" || !inRoom()) return;
  $("homename").textContent = t(ROOM_TITLES[roomTab]);
  if (roomTab !== "board") $("homesub").textContent = roomTab === "members" ? t(people().length === 1 ? "{n} member" : "{n} members", { n: people().length }) : "";
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
    out.push(el("div", { className: "askbox" }, picture(a.name, null, "", "lg"), el("div", { className: "t" }, el("b", { textContent: t("{name} wants to join", { name: a.name || shortId(a.id) }) }), el("small", { className: "mono", textContent: shortId(a.id) })), el("span", { className: "sp" }), el("button", { className: "primary", textContent: t("Approve"), onclick: answer(true) }), el("button", { textContent: t("Decline"), onclick: answer(false) })));
  }
  if (st.invites) out.push(el("div", { className: "invitebox" }, icon("link", "sm"), el("span", { textContent: t("An invite code is open") }), el("span", { className: "sp" }), el("button", { className: "sm", onclick: copyInvite }, icon("copy", "sm"), t("Copy")), el("button", { className: "sm", onclick: cancelInvite, textContent: t("Cancel invite") })));
  const head = el("tr", {}, ...["Name", "ID", "Status", "Connection", "Round trip", "Version", ""].map((h) => el("th", { textContent: h ? t(h) : "" })));
  const body = rows.map(({ id, p }) => {
    const me = id === st.id;
    const state = memberOf(id).status;
    const conn = me || !p.connected ? el("span", { className: "faint", textContent: "—" }) : el("span", { className: "net " + (p.path === "relay" ? "warn" : "ok"), onclick: (e) => { e.stopPropagation(); openDiagnostics("flow"); } }, icon("relay", "sm"), p.path === "relay" ? t("Relay") : t("Direct"));
    const version = me ? st.version : p && p.version;
    const tr = el("tr", { className: "pick" + (drawerFor === id ? " sel" : "") + (state.kind === "off" ? " dim" : ""), tabIndex: 0 },
      el("td", {}, el("span", { className: "who" }, me ? el("button", { className: "plain", ariaLabel: t("Avatar"), onclick: (e) => { e.stopPropagation(); openSettings("general"); } }, avatar(id, "", "on")) : avatar(id, "", p && p.connected ? "on" : null), el("b", { textContent: nameOf(id) }), me ? el("span", { className: "pill gray", textContent: t("You") }) : null)),
      el("td", {}, el("span", { className: "mono", textContent: `${id.slice(0, 4)}·${id.slice(4, 8)}·${id.slice(8, 12)}` }), iconButton("copy", t("Copy ID"), (e) => { e.stopPropagation(); navigator.clipboard.writeText(id).then(() => flash(t("ID copied"))); })),
      el("td", { className: "state " + state.kind }, el("i", { className: "sdot" }), state.text),
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
function checkMember(id) {
  if (checking[id]) return;
  checking[id] = true;
  delete sigs.drawer;
  renderDrawer();
  invoke("check_peer", { id }).then((lines) => { checks[id] = Array.isArray(lines) ? lines : [{ level: "fail", text: t("No answer") }]; }).catch(() => { checks[id] = [{ level: "fail", text: t("No answer") }]; }).finally(() => { checking[id] = false; delete sigs.drawer; renderDrawer(); });
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
  if (!checks[id] && !checking[id] && (me || (p && p.connected))) checkMember(id);
  const receivingFrom = st.sets.filter((s) => !s.live).flatMap((s) => s.players.filter((x) => x.owner === id && x.total && x.have < x.total).map((x) => ({ s, x })));
  const sig = [id, p && [p.connected, p.path, p.rtt_ms, p.sent, p.received, p.offset_us, p.version], checks[id], checking[id], speed, receivingFrom.map((r) => [r.s.id, r.x.have]), st.names, look(id), lang];
  if (!changed("drawer", sig)) return;
  const kv = [];
  const add = (k, v) => kv.push(el("dt", { textContent: k }), el("dd", { textContent: v }));
  let path = null;
  if (p && p.connected) {
    path = p.path === "relay" ? el("div", { className: "pathbox relay" }, el("b", {}, icon("relay", "sm"), t("Via relay server")), el("small", { textContent: [p.relay, t("end-to-end encrypted")].filter(Boolean).join(" · ") })) : el("div", { className: "pathbox direct" }, el("b", {}, icon("relay", "sm"), t("Direct")), el("small", { textContent: t("end-to-end encrypted") }));
    add(t("Round trip"), `${p.rtt_ms} ms`);
    if (p.sent !== null && p.sent !== undefined) add(t("Sent"), bytes(p.sent));
    if (p.received !== null && p.received !== undefined) add(t("Received"), bytes(p.received));
    if (p.offset_us !== null && p.offset_us !== undefined) add(t("Clock offset"), `${p.offset_us >= 0 ? "+" : "−"}${(Math.abs(p.offset_us) / 1e6).toFixed(2)} s`);
  } else if (!me) path = el("div", { className: "pathbox off" }, el("b", { textContent: memberOf(id).status.text }));
  add(t("Version"), (me ? st.version : p && p.version) || "—");
  const measuring = !!speed && speed.id === id && !speed.line;
  if (speed && speed.id === id && speed.line) add(t("Speed"), speed.line);
  const lines = checks[id];
  const checkBox = [el("div", { className: "h3", textContent: t("Checks") }), ...(lines ? lines.filter((l) => l.level !== "info").map((l) => el("div", { className: "line lv-" + l.level }, icon({ ok: "ok", warn: "alert", fail: "fail" }[l.level] || "info", "sm"), el("span", { textContent: t(l.text, l.args) }))) : checking[id] ? [el("div", { className: "line" }, icon("spinner", "sm spin"), t("Checking…"))] : []), el("div", { className: "row" }, el("button", { className: "sm", disabled: !!checking[id] || (!me && !(p && p.connected)), onclick: () => checkMember(id) }, icon("refresh", "sm"), t("Check again")), me ? "" : el("button", { className: "sm", disabled: measuring || !(p && p.connected), onclick: () => measureSpeed(id) }, icon("activity", "sm"), measuring ? t("Measuring…") : t("Measure speed")))];
  box.replaceChildren(
    el("div", { className: "dhead" }, avatar(id, "xl"), el("div", { className: "t" }, el("b", { textContent: nameOf(id) }), el("small", { className: "mono", textContent: `${id.slice(0, 4)}·${id.slice(4, 8)}·${id.slice(8, 12)}` })), iconButton("x", t("Close"), () => { drawerFor = null; delete sigs.roomtab; renderRoom(); })),
    path || "",
    el("dl", { className: "kv" }, ...kv),
    receivingFrom.length ? el("div", { className: "h3", textContent: t("Receiving") }) : "",
    ...receivingFrom.map(({ s, x }) => el("div", { className: "recvrow" }, el("div", { className: "row" }, el("span", { textContent: sessionName(s.id) }), el("span", { className: "sp" }), el("span", { className: "muted", textContent: `${Math.floor((x.have / x.total) * 100)}%` })), el("div", { className: "bar" }, el("i", { style: `width:${(x.have / x.total) * 100}%` })))),
    el("div", { className: "checks" }, ...checkBox),
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
  $("relays").value = st.relays.join("\n");
  $("hotkey").textContent = st.hotkey;
  $("hotkeyerr").hidden = true;
  $("myid").textContent = st.id.match(/.{4}/g).slice(0, 4).join("·");
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
function renderSettings() {
  if (!$("settings").open) return;
  const q = st.capture.quality;
  for (const b of $("quality").children) press(b, b.dataset.q === q.id);
  $("qinfo").textContent = [q.width ? `${q.width} × ${q.height}` : "", `${q.fps} fps`, `${(q.kbps / 1000).toFixed(1)} Mbit/s`, String(q.codec).toUpperCase()].filter(Boolean).join(" · ");
  if (document.activeElement !== $("limit")) $("limit").value = st.limit_mbps || "";
  for (const b of $("channel").children) press(b, b.dataset.c === st.channel);
  if (document.activeElement !== $("apion")) $("apion").checked = st.api.on;
  $("apirows").hidden = !st.api.on;
  $("apiaddr").textContent = apiUrl(st.api.address);
  for (const b of $("apiaccess").children) press(b, (b.dataset.lan === "1") === st.api.lan);
  $("apilanbox").hidden = !st.api.lan;
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
  const own = image && !FACES[image] ? [press(el("button", { className: "facebtn", ariaLabel: t("Avatar") }, picture("", colorOf(mine), image, "lg")), true)] : [];
  $("facegrid").replaceChildren(...own, ...Object.keys(FACES).map((name, i) => {
    const b = el("button", { className: "facebtn", ariaLabel: `${t("Avatar")} ${i + 1}`, onclick: () => run("set_profile", { avatar: name }) }, picture("", colorOf(mine), name, "lg"));
    press(b, image === name);
    return b;
  }));
  $("avatarclear").hidden = !image;
}
function renderApiReference() {
  if (st.api.lan) invoke("api_info").then((info) => { $("apihosts").textContent = info.hosts.map(apiUrl).join("  "); }).catch(() => {});
  else $("apihosts").textContent = "";
  if ($("apiref").children.length) return;
  invoke("api_reference").then((list) => {
    $("apiref").replaceChildren(...list.map((e) => el("details", { className: "apiep" }, el("summary", { textContent: e.title }), el("div", { className: "apibody" }, ...apiReferenceBlocks(e.lines).map((b) => b.code ? el("pre", { textContent: b.text }) : el("p", { className: "muted small", textContent: b.text }))))));
  }).catch(fail);
}
function loadStorage() {
  invoke("data_info").then((info) => { storageInfo = info; delete sigs.storage; renderStorage(); }).catch(fail);
}
function recvText(s) {
  const recv = receiving(s);
  return recv && !recv.stuck ? progressText(recv) : "";
}
function renderStorage() {
  const d = st.disk;
  const used = storageInfo ? storageInfo.used_bytes : 0;
  const old = storageInfo ? storageInfo.old_bytes : 0;
  const totalBytes = d.total || 1;
  const other = Math.max(0, totalBytes - d.free - used);
  const sets = st.sets.filter((s) => !s.live);
  const sig = [used, old, d, sets.map((s) => [s.id, s.progress.have, s.progress.total, s.shared, recvText(s)]), [...picked], lang];
  if (!changed("storage", sig)) return;
  $("storedir").textContent = storageInfo ? storageInfo.dir : "";
  $("oldfiles").hidden = !old;
  $("oldfiles").lastChild.textContent = t("Delete old files ({size})", { size: bytes(old) });
  const seg = (cls, n) => { const i = el("i", { className: cls }); i.style.width = (n / totalBytes) * 100 + "%"; return i; };
  $("usage").replaceChildren(seg("app", used), seg("other", other));
  const free = d.hours_left !== null && d.hours_left !== undefined ? t("Free {free} · about {h} h of recording", { free: bytes(d.free), h: Math.floor(d.hours_left) }) : t("Free {free}", { free: bytes(d.free) });
  $("legend").replaceChildren(el("span", {}, el("i", { className: "app" }), `${st.app} ${bytes(used)}`), el("span", {}, el("i", { className: "other" }), t("Other files {size}", { size: bytes(other) })), el("span", { className: "sp" }), el("span", {}, el("i", { className: "free" }), free));
  $("selold").textContent = t("All but newest {n}", { n: KEEP_NEWEST });
  for (const id of [...picked]) if (!sets.some((s) => s.id === id)) picked.delete(id);
  const biggest = Math.max(1, ...sets.map((s) => s.progress.have));
  const rows = sets.map((s) => {
    const box = el("input", { type: "checkbox", checked: picked.has(s.id), disabled: !canDelete(s), ariaLabel: sessionName(s.id) });
    box.onchange = () => { if (box.checked) picked.add(s.id); else picked.delete(s.id); delete sigs.storage; renderStorage(); };
    const notes = noteCount(s);
    return el("tr", { className: picked.has(s.id) ? "sel" : "" }, el("td", {}, box), el("td", {}, el("b", { textContent: sessionName(s.id) }), " ", el("span", { className: "muted small", textContent: noPicture(s) ? t("voice only") : t(notes === 1 ? "{n} note" : "{n} notes", { n: notes }) })), el("td", { textContent: setLength(s) }), el("td", { textContent: String(s.players.length) }), el("td", {}, onThisPc(s) ? [el("span", { className: "sizebar" }, el("i", { style: `width:${(s.progress.have / biggest) * 100}%` })), bytes(s.progress.have)] : el("span", { className: "muted", textContent: t("Not on this PC") })), el("td", { className: "muted small", textContent: recvText(s) }));
  });
  const head = el("tr", {}, ...["", "Recorded", "Length", "People", "On this PC", ""].map((h) => el("th", { textContent: h ? t(h) : "" })));
  $("storetable").replaceChildren(el("table", { className: "table" }, el("thead", {}, head), el("tbody", {}, ...rows)));
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

function openDiagnostics(tab) {
  if (tab) diagTab = tab;
  if (!$("diagdlg").open) $("diagdlg").showModal();
  delete sigs.diag;
  renderDiagnostics();
}
const DIAG_FEED_MS = 2000;
const DIAG_LOG_LINES = 300;
const TRANSFER_KINDS = { init: "Setup", "live picture": "Live picture", "live voice": "Live voice", "history picture": "Recorded picture", "history voice": "Recorded voice" };
const TRANSFER_RESULTS = { ok: "Done", retry: "Retried", partial: "Partial", "hash fail": "Checksum failed" };
function pullDiag() {
  const tab = { traffic: "network", log: "log", flow: "flow", jobs: "jobs" }[diagTab];
  if (!tab || Date.now() - diagFeedAt < DIAG_FEED_MS) return;
  diagFeedAt = Date.now();
  invoke("debug_view", { tab, since: 0, set: null }).then((data) => { diagFeed = { tab: diagTab, data }; delete sigs.diag; renderDiagnostics(); }).catch(warn);
}
function diagTable(head, rows, empty) {
  if (!rows.length) return el("div", { className: "empty", textContent: empty });
  return el("div", { className: "tablebox" }, el("table", { className: "table" }, el("thead", {}, el("tr", {}, ...head.map((h) => el("th", { textContent: h })))), el("tbody", {}, ...rows.map((cells) => el("tr", {}, ...cells.map((c) => el("td", { textContent: c })))))));
}
function trafficView(data) {
  const kind = (k) => t(TRANSFER_KINDS[k] || k);
  const dir = (d) => (d === "in" ? t("Received") : t("Sent"));
  const active = diagTable([t("Direction"), t("Kind"), t("Size")], data.active.map((r) => [dir(r.dir), kind(r.kind), bytes(r.bytes || 0)]), t("Nothing is moving right now"));
  const groups = new Map();
  for (const r of data.rows) {
    const key = [r.dir, r.kind, r.result].join("|");
    const g = groups.get(key) || { r, n: 0, size: 0 };
    g.n += 1;
    g.size += r.bytes || 0;
    groups.set(key, g);
  }
  const recent = diagTable([t("Direction"), t("Kind"), t("Result"), t("Count"), t("Size")], [...groups.values()].map((g) => [dir(g.r.dir), kind(g.r.kind), t(TRANSFER_RESULTS[g.r.result] || g.r.result), String(g.n), bytes(g.size)]), t("Nothing yet"));
  return [el("div", { className: "h3 first", textContent: t("Moving now") }), active, el("div", { className: "h3", textContent: t("Since the app started") }), recent];
}
function logView(data) {
  const lines = String(data.text || "").split("\n").filter(Boolean).slice(-DIAG_LOG_LINES);
  return [el("pre", { className: "logview", textContent: lines.length ? lines.join("\n") : t("Nothing yet") })];
}
function jobsView(data) {
  const jobs = (data && data.jobs) || [];
  const progress = (j) => (typeof j.progress === "number" ? ` ${Math.floor(j.progress * 100)}%` : "");
  return [diagTable([t("Extension"), t("Recorded"), t("State"), t("Run by")], jobs.map((j) => [j.plugin ? pluginName(j.plugin) : "—", j.set ? sessionName(j.set) : "—", `${j.state || "—"}${progress(j)}`, j.by || "—"]), t("Nothing yet"))];
}
const failedCount = () => st.sets.filter((s) => (s.analysis || []).some((a) => a.state === "failed")).length;
function renderDiagnostics() {
  if (!$("diagdlg").open) return;
  for (const b of $("diagdlg").querySelectorAll("[data-dtab]")) b.classList.toggle("on", b.dataset.dtab === diagTab);
  pullDiag();
  const failed = failedCount();
  const lines = summaryLines(st, failed);
  const fed = diagFeed && diagFeed.tab === diagTab ? diagFeed.data : null;
  if (!changed("diag", [diagTab, lines, st.peers, st.hw, st.clock, st.api, st.channel, fed, lang])) return;
  if (diagTab === "summary") {
    $("diagbody").replaceChildren(el("div", { className: "okbox" }, ...lines.map((l) => el("div", { className: "ok-line lv-" + l.level }, icon({ ok: "ok", warn: "alert", fail: "fail" }[l.level] || "info", "sm"), el("b", { textContent: t(l.key) }), el("span", { textContent: l.text }), l.key === "Extensions" && failed ? el("button", { className: "sm", textContent: t("Open extensions"), onclick: () => { $("diagdlg").close(); showRoomTab("extensions"); } }) : null))));
    return;
  }
  if (diagTab === "traffic" || diagTab === "log") {
    $("diagbody").replaceChildren(...(fed ? (diagTab === "traffic" ? trafficView(fed) : logView(fed)) : [el("div", { className: "empty", textContent: t("Loading…") })]));
    if (diagTab === "log") $("diagbody").scrollTop = $("diagbody").scrollHeight;
    return;
  }
  if (diagTab === "jobs") {
    $("diagbody").replaceChildren(...(fed ? jobsView(fed) : [el("div", { className: "empty", textContent: t("Loading…") })]));
    return;
  }
  if (diagTab === "pcs") {
    $("diagbody").replaceChildren(...pcsView(st));
    return;
  }
  const peers = st.peers;
  const sendsLive = st.capture.sharing;
  const what = (p) => [sendsLive ? t("Live picture and voice") : null, t("Recorded files"), t("Log")].filter(Boolean).join(" · ");
  const node = (id, sub, cls = "") => el("div", { className: "node " + cls }, avatar(id, "lg"), el("div", { className: "t" }, el("b", { textContent: nameOf(id) }), sub ? el("small", { textContent: sub }) : null));
  const links = peers.map((p) => el("div", { className: "flink " + (p.connected ? p.path : "off") }, el("span", { className: "line" }), p.connected && p.path === "relay" ? el("span", { className: "relaybox" }, el("b", { textContent: t("Relay server") }), el("small", { textContent: p.relay || t("content not visible") })) : el("span", { className: "label", textContent: p.connected ? `${t("Direct")} · ${t("encrypted")} · ${p.rtt_ms} ms` : t("Offline") }), node(p.id, "", p.connected ? "" : "dim")));
  const diagram = el("div", { className: "flowmap" }, node(st.id, t("This PC")), el("div", { className: "links" }, ...(links.length ? links : [el("div", { className: "none", textContent: t("No other members") })])));
  const head = el("tr", {}, ...["Peer", "Path", "What is sent", "Sent", "Received"].map((h) => el("th", { textContent: t(h) })));
  const rows = peers.map((p) => el("tr", { className: p.connected ? "" : "dim" }, el("td", {}, el("b", { textContent: nameOf(p.id) })), el("td", { className: p.connected ? (p.path === "relay" ? "warn" : "ok") : "faint", textContent: p.connected ? (p.path === "relay" ? [t("Relay"), p.relay].filter(Boolean).join(" ") : t("Direct")) : t("Offline") }), el("td", { textContent: p.connected ? what(p) : "—" }), el("td", { textContent: p.sent != null ? bytes(p.sent) : "—" }), el("td", { textContent: p.received != null ? bytes(p.received) : "—" })));
  const sent = flowRows(st, fed, (k) => (k === "ffmpeg" ? t("Video tools") : pluginName(k)));
  $("diagbody").replaceChildren(el("div", { className: "okbox" }, ...sent.map(okLine)), diagram, el("div", { className: "tablebox" }, el("table", { className: "table" }, el("thead", {}, head), el("tbody", {}, ...rows))));
}
async function diagnosticsText() {
  const head = summaryLines(st, failedCount()).map((l) => `[${l.level}] ${l.key}: ${l.text}`).join("\n");
  return invoke("debug_report", { save: false, head });
}

function openReset() {
  $("keeprooms").checked = true;
  $("resetdlg").showModal();
}
function renderOpenDialogs() {
  renderSettings();
  renderDiagnostics();
}

$("startname").oninput = () => renderStart();
$("startname").onchange = () => { run("set_name", { name: $("startname").value }); if ($("startname").value.trim()) askName = false; };
$("nameedit").onclick = () => { askName = true; renderStart(); $("startname").focus(); };
$("newroom").onclick = () => run("set_name", { name: $("startname").value || st.name }, createRoom);
$("joincode").oninput = () => renderStart();
$("join").onclick = () => run("set_name", { name: $("startname").value || st.name }, () => joinWith($("joincode")));
$("joincode").onkeydown = (e) => { if (e.key === "Enter" && !composing(e) && !$("join").disabled) $("join").click(); };
$("pendingcancel").onclick = () => st.room && run("remove_room", { id: st.room.id });
$("roomnameinput").oninput = showIconDraft;
$("roomiconpick").onclick = () => pickImage((image) => { iconDraft = image; showIconDraft(); });
$("roomiconclear").onclick = () => { iconDraft = ""; showIconDraft(); };
$("roomnameinput").onkeydown = (e) => { if (e.key === "Enter" && !composing(e) && !$("roomsave").disabled) $("roomsave").click(); };
$("roomsave").onclick = async () => {
  $("roomdlg").close();
  try {
    if ($("roomnameinput").value.trim() !== st.room.name) await invoke("rename_room", { name: $("roomnameinput").value });
    if (iconDraft !== st.room.icon) await invoke("set_room_icon", { icon: iconDraft });
    flash(t("Saved"));
    refresh();
  } catch (e) { fail(e); }
};
$("roomno").onclick = () => $("roomdlg").close();
for (const b of $("setnav").querySelectorAll("button")) b.onclick = () => { settingsPane = b.dataset.pane; showPane(); };
$("promptcopy").onclick = () => navigator.clipboard.writeText(promptFull).then(() => { $("promptdlg").close(); flash(t("Prompt copied")); }).catch(fail);
$("aibtn").onclick = (e) => openAiMenu(e.currentTarget, newestRecording());
$("aiplay").onclick = (e) => openAiMenu(e.currentTarget, selected);
for (const b of document.querySelectorAll(".apibadge")) b.onclick = () => openSettings("api");
for (const b of $("apiaccess").children) b.onclick = () => run("set_api_lan", { lan: b.dataset.lan === "1" });
$("apidoc").onclick = () => dialog("open_link", { url: `${apiUrl(st.api.address)}/llms.txt` }).catch(fail);
$("apion").onchange = () => run("set_api", { on: $("apion").checked });
$("apicopy").onclick = () => invoke("api_info").then((info) => navigator.clipboard.writeText(info.token)).then(() => flash(t("Token copied"))).catch(fail);
$("apinew").onclick = async () => {
  if (await ask(t("Make a new token?"), t("New token"), false, t("Programs using the old token stop working."))) run("new_api_token", null, () => flash(t("New token made")));
};
$("name").onchange = () => run("set_name", { name: $("name").value });
$("avatarimage").onclick = () => pickImage((image) => run("set_profile", { avatar: image }));
$("avatarclear").onclick = () => run("set_profile", { avatar: "" });
$("language").onchange = () => run("set_language", { language: $("language").value });
$("sessionsound").onchange = () => { sessionSound = $("sessionsound").checked; store("sessionSound", sessionSound); };
$("copyid").onclick = () => navigator.clipboard.writeText(st.id).then(() => flash(t("ID copied")));
$("exportid").onclick = () => dialog("identity", { export: true }).catch(warn);
$("importid").onclick = () => dialog("identity", { export: false }).catch(warn);
$("opendata").onclick = $("storeopen").onclick = () => dialog("open_data_folder").catch(fail);
for (const b of $("quality").children) b.onclick = () => run("set_quality", { id: b.dataset.q });
$("relays").onchange = () => run("set_relays", { urls: $("relays").value.split("\n") });
$("limit").onchange = () => run("set_limit", { mbps: $("limit").value ? Number($("limit").value) : null });
for (const b of $("channel").children) b.onclick = () => run("set_channel", { channel: b.dataset.c }, () => { refresh(); checkUpdate(false); });
$("checkupdate").onclick = () => checkUpdate(true);
$("selnone").onclick = () => { picked.clear(); delete sigs.storage; renderStorage(); };
$("selold").onclick = () => { picked = new Set(oldSessions(st.sets.filter((s) => !s.live))); delete sigs.storage; renderStorage(); };
$("deletesel").onclick = () => confirmDelete([...picked]);
$("oldfiles").onclick = async () => {
  if (await ask(t("Delete old files?"), t("Delete"), true)) invoke("delete_old_files").catch(fail).finally(loadStorage);
};
$("delno").onclick = () => $("deletedlg").close();
for (const b of $("diagdlg").querySelectorAll("[data-dtab]")) b.onclick = () => openDiagnostics(b.dataset.dtab);
$("diagcopy").onclick = () => diagnosticsText().then((text) => navigator.clipboard.writeText(text)).then(() => flash(t("Copied"))).catch(fail);
$("diagsave").onclick = () => diagnosticsText().then((text) => dialog("save_text", { name: "ez-hanseikai-diagnostics.txt", text })).then((saved) => { if (saved) flash(t("Saved")); }).catch(fail);
$("resetno").onclick = () => $("resetdlg").close();
$("resetyes").onclick = () => {
  $("resetdlg").close();
  const keepRooms = $("keeprooms").checked;
  run("reset", { keep_rooms: keepRooms }, null);
};
$("hotkey").onclick = () => {
  listening = true;
  $("hotkey").classList.add("listening");
  $("hotkey").textContent = t("Press the keys…");
};
$("hotkey").onblur = () => { if (listening) { listening = false; $("hotkey").classList.remove("listening"); $("hotkey").textContent = st.hotkey; } };
$("hotkey").onkeydown = (e) => {
  if (!listening) return;
  e.preventDefault();
  e.stopPropagation();
  if (e.key === "Escape") { $("hotkey").blur(); return; }
  const keys = keysOf(e);
  if (!keys) return;
  listening = false;
  $("hotkey").classList.remove("listening");
  $("hotkey").textContent = keys;
  invoke("set_hotkey", { keys }).then(() => { $("hotkeyerr").hidden = true; refresh(); }).catch((err) => { $("hotkeyerr").textContent = t(String(err)); $("hotkeyerr").hidden = false; $("hotkey").textContent = st.hotkey; });
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
  "leave-room": leaveRoom,
  "add-note": openQuick,
  "delete-session": () => deleteSession(playSet()),
  "page-live": () => navigate("live"),
  "page-board": () => navigate("board"),
  "page-members": () => navigate("members"),
  "page-extensions": () => navigate("extensions"),
  "sidebar": toggleSidebar,
  "fullscreen": toggleFull,
  "all-players": () => setView(current(), 0),
  "shortcuts": () => openSettings("shortcuts"),
  "update": () => checkUpdate(true),
  "diagnostics": () => openDiagnostics(),
  "reset": openReset,
  "toggle-recording": toggleRecording,
};
listenMain("menu", (e) => { if (menuActions[e.payload]) menuActions[e.payload](); });
listenMain("joined", (e) => { flash(t("{name} joined", { name: e.payload })); chime("join"); });
listenMain("session", (e) => {
  const debrief = e.payload.action === "end" ? debriefOpen : null;
  debriefOpen = null;
  flash(sessionNotice(e.payload.action, e.payload.by), "info", debrief && { label: t("Summarize with AI"), run: () => summarizeDebrief(debrief) });
  chime(e.payload.action);
  if (e.payload.action === "start" && !e.payload.voice) openLive();
  const ended = e.payload.action === "end" && setById(e.payload.set);
  if (ended && !ended.of && page === "player" && showingNow()) { selectSet(null); openSession(ended.id); }
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

const BOARD_WIDTH = 1080;
const BOARD_PAD = 28;
const PREVIEW_PAD = 16;
const PREVIEW_MIN = 0.2;
const ZOOM_STEP = 0.1;
const ZOOM_MIN = 0.5;
const ZOOM_MAX = 2;
const SHORTCUTS = [
  ["Live / Game plan / Members / Plugins", ["Ctrl+1", "Ctrl+2", "Ctrl+3", "Ctrl+4"]],
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
let jobFilter = "all";
let boardEditing = false;
let boardDraft = "";
let boardScope = stored("boardScope", "room") === "personal" ? "personal" : "room";
let boardFull = false;
let zoom = stored("boardZoom", 0);
let settingsPane = "general";
let storageInfo = null;
let picked = new Set();
let diagTab = "summary";
let recFilter = "";
let diagFeed = null;
let diagFeedAt = 0;

function renderStart() {
  if (inRoom()) return;
  const pendingRoom = st.room && st.room.pending;
  if (!st.name) askName = true;
  $("namefield").hidden = !askName;
  $("nameshow").hidden = askName;
  if (!askName && changed("startname", [st.name, st.colors])) { $("nameav").replaceChildren(avatar(st.id, "lg")); $("nametext").textContent = st.name; }
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
  if (!changed("rooms", [others, lang])) return;
  $("roomlist").replaceChildren(...others.map((r, i) => {
    const face = (id) => { const a = el("span", { className: "av sm", textContent: initial(id === st.id ? st.name : r.names[id]) }); a.style.background = colorOf(r.colors[id]); return a; };
    const enter = () => run("switch_room", { id: r.id }, openLive);
    const more = iconButton("more", t("More"), (e) => { e.stopPropagation(); openMenu([{ label: t("Remove from this PC…"), icon: "trash", danger: true, run: () => removeRoom(r) }], e.currentTarget); }, "icon sm more");
    const row = el("div", { className: "rrow", tabIndex: 0, role: "button" }, el("b", { textContent: r.name }), el("span", { className: "avs" }, ...r.members.slice(0, AVATARS_SHOWN).map(face)), el("span", { className: "sp" }), r.used_us ? el("span", { className: "muted small", textContent: dayLabel(dayOf(new Date(r.used_us / 1000)), new Date(), lang) }) : null, el("button", { className: "sm" + (i ? "" : " primary"), textContent: t("Enter"), onclick: (e) => { e.stopPropagation(); enter(); } }), more);
    row.onclick = enter;
    row.onkeydown = (e) => { if (e.key === "Enter") enter(); };
    return row;
  }));
}
async function removeRoom(r) {
  if (await ask(t("Remove {room} from this PC?", { room: r.name }), t("Remove"), true, t("Its recordings and notes on this PC are deleted. Other members keep theirs."))) run("remove_room", { id: r.id }, () => { forgetPlan(r.id); refresh(); });
}
function joinWith(input) {
  if (joining) return;
  joining = true;
  run("join_room", { code: input.value }, () => { input.value = ""; refresh(); }).finally(() => { joining = false; });
}
const copyInvite = () => invoke("create_invite", { reuse: true }).then((code) => navigator.clipboard.writeText(code)).then(() => { flash(t("Invite code copied")); refresh(); }).catch(fail);
const cancelInvite = () => run("cancel_invites", null, () => { flash(t("Invite cancelled")); refresh(); });
const createRoom = () => invoke("create_room", { name: "" }).then(() => { openLive(); return refresh().then(copyInvite); }).catch(fail);
async function switchRoom() {
  if (!st.room) return;
  if (st.recording && !(await ask(t("Stop recording and leave {room}?", { room: st.room.name }), t("Switch room"), true))) return;
  run("leave_room", { id: st.room.id }, () => { endReview(); selectSet(null); openLive(); });
}
async function leaveRoom() {
  if (!st.room) return;
  const { id, name: room } = st.room;
  if (!(await ask(t(st.recording ? "Stop recording and leave {room}?" : "Leave {room}?", { room }), t("Leave room"), true, t("Its recordings and notes on this PC are deleted. Other members keep theirs.")))) return;
  run("remove_room", { id }, () => { forgetPlan(id); endReview(); selectSet(null); openLive(); });
}
async function removeMember(id) {
  if (await ask(t("Remove {name} from the room?", { name: nameOf(id) }), t("Remove from room"), true, t("Everyone stops sending to them. What they already have stays with them."))) { drawerFor = null; run("remove_peer", { id }); }
}
function openRoomSettings() {
  if (!st.room) return;
  $("roomnameinput").value = st.room.name;
  $("roomnameinput").maxLength = ROOM_NAME_MAX;
  $("roomsave").disabled = true;
  $("roomdlg").showModal();
  $("roomnameinput").select();
}
function showRoomTab(tab) {
  roomTab = tab;
  drawerFor = null;
  delete sigs.roomtab;
  go("room");
  renderRoom();
}

const ROOM_TITLES = { members: "Members", plugins: "Plugins", jobs: "Plugins", board: "Game plan", recordings: "Recordings" };

function renderRoom() {
  if (page !== "room" || !inRoom()) return;
  $("homename").textContent = t(ROOM_TITLES[roomTab]);
  if (roomTab !== "board") $("homesub").textContent = roomTab === "members" ? t("{n} members", { n: people().length }) : "";
  $("boardpane").hidden = roomTab !== "board";
  $("roomtab").parentElement.hidden = roomTab === "board";
  if (roomTab === "board") renderBoard();
  else if (roomTab === "members") renderMembers();
  else if (roomTab === "plugins") renderPlugins();
  else if (roomTab === "recordings") renderRecordings();
  else renderJobs();
  renderDrawer();
}
function memberOf(id) {
  const me = id === st.id;
  const p = me ? null : peerOf(id);
  const status = memberStatus({ connected: me || !!(p && p.connected), sharing: me ? !!st.capture.source : !!(p && p.sharing), recording: recOf(id), receiving: receivingPercent(st.sets, id, nowUs(), st.id), seen_us: p && p.seen_us }, new Date());
  return { p, status };
}
function renderMembers() {
  const rows = people().map((id) => ({ id, p: id === st.id ? null : peerOf(id) }));
  const sig = ["members", rows.map((r) => [r.id, memberOf(r.id).status, r.p && [r.p.connected, r.p.path, r.p.rtt_ms, r.p.version, r.p.failures]]), st.asks, st.invites, drawerFor, st.names, st.colors, lang];
  if (!changed("roomtab", sig)) return;
  const out = [];
  for (const a of st.asks) {
    const answer = (approve) => () => run("answer_ask", { id: a.id, approve });
    out.push(el("div", { className: "askbox" }, el("span", { className: "av lg", textContent: initial(a.name), style: "background:var(--muted)" }), el("div", { className: "t" }, el("b", { textContent: t("{name} wants to join", { name: a.name || shortId(a.id) }) }), el("small", { className: "mono", textContent: shortId(a.id) })), el("span", { className: "sp" }), el("button", { className: "primary", textContent: t("Approve"), onclick: answer(true) }), el("button", { textContent: t("Decline"), onclick: answer(false) })));
  }
  if (st.invites) out.push(el("div", { className: "invitebox" }, icon("link", "sm"), el("span", { textContent: t("An invite code is open") }), el("span", { className: "sp" }), el("button", { className: "sm", onclick: copyInvite }, icon("copy", "sm"), t("Copy")), el("button", { className: "sm ghost", onclick: cancelInvite, textContent: t("Cancel invite") })));
  const head = el("tr", {}, ...["Name", "ID", "Status", "Connection", "Round trip", "Version", ""].map((h) => el("th", { textContent: h ? t(h) : "" })));
  const body = rows.map(({ id, p }) => {
    const me = id === st.id;
    const state = memberOf(id).status;
    const conn = me || !p.connected ? el("span", { className: "faint", textContent: "—" }) : el("span", { className: p.path === "relay" ? "warn" : "ok" }, icon("relay", "sm"), p.path === "relay" ? t("Relay") : t("Direct"));
    const version = me ? st.version : p && p.version;
    const tr = el("tr", { className: "pick" + (drawerFor === id ? " sel" : "") + (state.kind === "off" ? " dim" : ""), tabIndex: 0 },
      el("td", {}, el("span", { className: "who" }, avatar(id, "", me || (p && p.connected) ? (recOf(id) ? "rec" : "on") : null), el("b", { textContent: nameOf(id) }), me ? el("span", { className: "pill gray", textContent: t("You") }) : null)),
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
function renderDrawer() {
  const box = $("drawer");
  const id = roomTab === "members" ? drawerFor : null;
  box.hidden = !id || !people().includes(id);
  if (box.hidden) return;
  const me = id === st.id;
  const p = me ? null : peerOf(id);
  if (!checks[id] && !checking[id] && (me || (p && p.connected))) checkMember(id);
  const receivingFrom = st.sets.filter((s) => !s.live).flatMap((s) => s.players.filter((x) => x.owner === id && x.total && x.have < x.total).map((x) => ({ s, x })));
  const sig = [id, p && [p.connected, p.path, p.rtt_ms, p.sent, p.received, p.offset_us, p.version], checks[id], checking[id], receivingFrom.map((r) => [r.s.id, r.x.have]), st.names, lang];
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
  const lines = checks[id];
  const checkBox = [el("div", { className: "h3", textContent: t("Checks") }), ...(lines ? lines.filter((l) => l.level !== "info").map((l) => el("div", { className: "line lv-" + l.level }, icon({ ok: "ok", warn: "alert", fail: "fail" }[l.level] || "info", "sm"), el("span", { textContent: t(l.text, l.args) }))) : checking[id] ? [el("div", { className: "line" }, icon("spinner", "sm spin"), t("Checking…"))] : []), el("button", { className: "ghost sm", disabled: !!checking[id] || (!me && !(p && p.connected)), onclick: () => checkMember(id) }, icon("refresh", "sm"), t("Check again"))];
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
function exportMenu(set, anchor) {
  openMenu([
    { label: t("Export video…"), icon: "film", disabled: voiceOnly(set), run: () => exportSet(set) },
    { label: t("Export to folder…"), icon: "upload", run: () => exportFolder(set) },
  ], anchor);
}
function recordingCard(r) {
  const s = r.s;
  const here = onThisPc(s);
  const state = here ? sessionState(s) : null;
  const meta = state && (state.kind === "bar" || state.kind === "line")
    ? el("div", { className: "meta" }, icon(state.icon, "sm"), el("span", { className: state.cls || "", textContent: state.text }), el("span", { className: "sp" }), el("span", { className: "muted", textContent: state.right || "" }))
    : el("div", { className: "meta" }, el("span", { textContent: setLength(s) }), detailText(s) ? el("span", { className: "faint", textContent: detailText(s) }) : null, ...marksOf(s));
  const exportBtn = iconButton("upload", t("Export"), (e) => { e.stopPropagation(); exportMenu(s, e.currentTarget); });
  const deleteBtn = iconButton("trash", t("Delete…"), (e) => { e.stopPropagation(); confirmDelete([s.id]); });
  exportBtn.disabled = !here;
  deleteBtn.disabled = !canDelete(s);
  const acts = el("span", { className: "acts" }, exportBtn, deleteBtn);
  const thumb = thumbBox(s, s.players.map((p) => p.owner), "sm", CARD_FACES, false);
  thumb.append(acts);
  if (state && state.kind === "bar") thumb.append(el("div", { className: "bar" }, el("i", { style: `width:${Math.round(state.frac * 100)}%` })));
  const card = el("div", { className: "reccard" + (here ? "" : " gone"), role: "button", tabIndex: 0 }, thumb, el("div", { className: "rtxt" }, el("b", { textContent: sessionName(s.id) }), meta, whereLine(s)));
  card.onclick = () => { if (here) openSession(s.id); };
  card.onkeydown = (e) => { if (e.key === "Enter" && e.target === card && here) openSession(s.id); };
  card.oncontextmenu = (e) => { e.preventDefault(); sessionMenu(s.id, e); };
  return card;
}
const recBar = el("div", { className: "recbar" },
  el("div", { className: "selbox grow" }, el("span", { className: "lead" }, icon("search", "sm")), el("input", { id: "recfilter", type: "search", autocomplete: "off", oninput: (e) => { recFilter = e.target.value; delete sigs.roomtab; renderRoom(); } })),
  el("button", { id: "recimport", onclick: () => openImport() }, icon("download", "sm"), el("span", { textContent: t("Import…") })));
const recGrid = el("div", { className: "reccards" });
function renderRecordings() {
  const rows = recordingRows();
  const words = recordingWords(recFilter);
  const shown = rows.filter((r) => recordingMatches(r, words));
  const sig = ["recordings", shown.map(rowSignature), recFilter, st.active_set, st.peers.map((p) => p.connected), lang, st.colors, st.names, dayOf(new Date())];
  if (!changed("roomtab", sig)) return;
  if ($("roomtab").firstElementChild !== recBar) $("roomtab").replaceChildren(recBar, recGrid);
  $("recfilter").placeholder = t("Filter");
  $("recfilter").setAttribute("aria-label", t("Filter"));
  $("recimport").lastChild.textContent = t("Import…");
  const a = st.active_set;
  const live = a ? [el("div", { className: "reccard live", role: "button", tabIndex: 0, onclick: openNow, onkeydown: (e) => { if (e.key === "Enter") openNow(); } }, thumbBox(null, liveSet() ? liveSet().players.map((p) => p.owner) : [], "sm", CARD_FACES, true), el("div", { className: "rtxt" }, el("b", {}, el("i", { className: "recdot" }), t("Recording")), el("div", { className: "meta" }, a.voice ? pill("accent", "mic", t("Debrief")) : null, el("span", { textContent: t("by {name}", { name: a.by || "" }) }))))] : [];
  const cards = shown.map(recordingCard);
  recGrid.replaceChildren(...live, ...cards);
  if (!live.length && !cards.length) recGrid.replaceChildren(el("div", { className: "empty", textContent: rows.length ? t("Nothing here") : t("No recordings yet") }));
}
function jobTarget(j) {
  if (j.whole) return t("Whole recording");
  const set = setById(j.set);
  return set && isReview(set) ? t("{name}'s voice (debrief)", { name: nameOf(j.owner) }) : t("{name}'s voice", { name: nameOf(j.owner) });
}
function jobMenu(j, anchor) {
  const plugin = plugins.find((p) => p.id === j.plugin);
  const model = plugin && plugin.options.find((o) => o.key === "model");
  const running = jobInfo.running && jobInfo.running.job === j.job;
  openMenu([
    { label: t("Run next on this PC"), icon: "play", disabled: j.state !== "waiting" || !j.here || !plugin || !plugin.process_here || !plugin.ready || (plugin.runner && plugin.runner !== st.id), run: () => invoke("run_job_next", { job: j.job }).then(pollPlugins) },
    { label: t("Retry"), icon: "refresh", disabled: j.state !== "failed", run: () => invoke("retry_job", { job: j.job }).then(pollPlugins).catch(fail) },
    { label: t("Stop"), icon: "stop", disabled: !running, run: () => invoke("cancel_job").then(pollPlugins) },
    ...(model ? [{ separator: true }, { label: t("Rerun with another model"), icon: "cpu", more: true, run: (b) => openMenu(model.choices.map((c) => ({ label: c, check: plugin.settings.model === c, run: () => rerunWith(plugin, c) })), b, true) }] : []),
  ], anchor);
}
async function rerunWith(plugin, model) {
  if (plugin.settings.model === model) return;
  if (!(await ask(t("Use {model} for the room?", { model }), t("Use {model}", { model }), false, t("Every recording is analysed again with it, on whichever PC has it.")))) return;
  invoke("set_room_plugin", { id: plugin.id, on: true, settings: { ...plugin.settings, model } }).then(pollPlugins).catch(fail);
}
function renderJobs() {
  const jobs = jobInfo ? jobInfo.jobs : [];
  const running = jobInfo && jobInfo.running;
  const shown = jobs.filter((j) => jobFilter === "all" || (jobFilter === "failed" ? j.state === "failed" : j.state === "claimed" || j.state === "waiting"));
  if (!changed("roomtab", ["jobs", shown, running && [running.job, Math.floor((running.progress || 0) * 100)], jobFilter, st.names, lang])) return;
  const seg = el("div", { className: "seg" }, ...[["all", t("All jobs")], ["active", t("Active")], ["failed", t("Failed")]].map(([k, label]) => el("button", { className: jobFilter === k ? "on" : "", textContent: label, onclick: () => { jobFilter = k; renderRoom(); } })));
  const state = (j) => {
    if (running && running.job === j.job) { const n = Math.floor((running.progress || 0) * 100); return el("div", { className: "jobrun" }, el("span", { textContent: t("Running {n}%", { n }) }), el("div", { className: "bar" }, el("i", { style: `width:${n}%` }))); }
    if (j.state === "failed") return el("span", { className: "pill bad", title: j.error || "" }, icon("fail", "sm"), t("Failed"));
    if (j.state === "done") return el("span", { className: "pill ok" }, icon("check", "sm"), t("Done"));
    if (j.state === "claimed") return el("span", { className: "pill accent", textContent: t("Running") });
    return el("span", { className: "pill gray", textContent: j.next ? t("Next") : j.here ? t("Waiting") : t("Waiting for the file") });
  };
  const rows = shown.map((j) => {
    const model = j.settings && j.settings.model;
    const more = iconButton("more", t("More"), (e) => { e.stopPropagation(); jobMenu(j, e.currentTarget); });
    const tr = el("tr", {}, el("td", {}, el("b", { textContent: pluginName(j.plugin) }), model ? el("span", { className: "mono muted", textContent: " " + model }) : null), el("td", { textContent: sessionName(j.set) }), el("td", { textContent: jobTarget(j) }), el("td", {}, state(j)), el("td", { className: j.by ? "" : "muted", textContent: runBy(j) }), el("td", { className: "chev" }, more));
    tr.oncontextmenu = (e) => { e.preventDefault(); jobMenu(j, e); };
    const err = j.state === "failed" && j.error ? el("tr", { className: "err" }, el("td", { colSpan: 6 }, icon("info", "sm"), t("Failed: {error}", { error: j.error }))) : null;
    return [tr, err];
  }).flat().filter(Boolean);
  const head = el("tr", {}, ...["Plugin", "Recorded", "Target", "State", "Run by", ""].map((h) => el("th", { textContent: h ? t(h) : "" })));
  const table = shown.length ? el("div", { className: "tablebox" }, el("table", { className: "table" }, el("thead", {}, head), el("tbody", {}, ...rows))) : el("div", { className: "empty", textContent: jobs.length ? t("Nothing here") : t("Turn on a plugin to analyse recordings") });
  $("roomtab").replaceChildren(el("div", { className: "row" }, pluginTabs(), el("span", { className: "sp" }), seg), table);
}
function pollPlugins() {
  if (!st || !inRoom() || document.hidden) return;
  const gen = ++pluginsGen;
  Promise.all([invoke("room_plugins"), invoke("jobs")]).then(([list, info]) => {
    if (gen !== pluginsGen) return;
    plugins = list;
    watchDownloads();
    if (changed("tags", info.tags)) for (const k of ["events", "board", "reviewtab"]) delete sigs[k];
    jobInfo = info;
    renderRoom();
    renderNav();
    renderSessions();
  }).catch(warn);
}
window.__TAURI__.event.listen("job", (e) => {
  if (jobInfo) jobInfo.running = e.payload;
  renderSessions();
  if (page === "room" && roomTab === "jobs") renderRoom();
});

function boardEvent() {
  const at = (e) => e.id - ((peerOf(e.author) || {}).offset_us || 0);
  let out = null;
  for (const e of st.events) if (e.kind === "board" && (!e.values || !e.values.card || e.values.card === "team") && (!out || at(e) >= at(out))) out = e;
  return out;
}
const onBoard = () => page === "room" && roomTab === "board";
const personal = () => boardScope === "personal";
const planKey = (id) => `board/${id}`;
const personalKey = () => planKey(st.room ? st.room.id : "");
const forgetPlan = (id) => unstore((k) => k === planKey(id));
const starterOf = (templates) => templates[lang === "ja" ? "ja" : "en"];
function showBoard() {
  roomTab = "board";
  go("room");
  renderRoom();
}
function boardText() {
  if (personal()) return stored(personalKey(), "") || starterOf(PERSONAL_TEMPLATE);
  const e = boardEvent();
  return e ? e.text : starterOf(BOARD_TEMPLATE);
}
function setBoardScope(scope) {
  if (boardEditing || scope === boardScope) return;
  boardScope = scope;
  store("boardScope", scope);
  delete sigs.board;
  renderBoard();
}
function writeBoard(text) {
  delete sigs.board;
  if (personal()) {
    store(personalKey(), text.trim() ? text : "");
    renderBoard();
    return Promise.resolve();
  }
  return invoke("add_event", { kind: "board", text, target: {}, values: { card: "team" } }).then(() => refresh()).catch(fail);
}
const boardHooks = () => mdHooks({ onTask: (n) => { boardDraft = toggleTask(boardDraft, n); writeBoard(boardDraft); } });
function fitZoom() {
  const w = $("boardwrap").clientWidth - 2 * BOARD_PAD;
  return Math.max(ZOOM_MIN, Math.min(1, w / BOARD_WIDTH));
}
function placeBoard() {
  const z = zoom || fitZoom();
  $("canvas").style.zoom = String(z);
  $("zoomfit").textContent = `${Math.round(z * 100)}%`;
  $("zoomfit").title = t("Fit to the window");
}
function renderBoard() {
  if (!onBoard() || !inRoom()) return;
  $("boardwrap").hidden = boardEditing;
  $("editor").hidden = !boardEditing;
  $("boardview").hidden = boardEditing;
  $("boardedits").hidden = !boardEditing;
  $("boardbadge").hidden = !boardEditing;
  for (const b of $("boardscope").children) { press(b, b.dataset.scope === boardScope); b.disabled = boardEditing; }
  const e = boardEvent();
  const sub = personal() ? t("Only on this PC") : e ? t("Updated by {name} · {time}", { name: nameOf(e.author), time: new Date((e.local_us || e.id) / 1000).toLocaleString(lang === "ja" ? "ja-JP" : "en-US", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }) }) : "";
  $("homesub").textContent = boardEditing ? "" : sub;
  if (boardEditing) return;
  if (changed("board", [boardText(), boardScope, st.room.id, lang, st.names, jobInfo && jobInfo.tags])) {
    boardDraft = boardText();
    $("canvas").replaceChildren(boardDom(boardDraft, boardHooks()));
  }
  placeBoard();
}
function editBoard(scope) {
  if (scope) { boardScope = scope; store("boardScope", scope); }
  showBoard();
  boardEditing = true;
  $("boardtext").value = boardText();
  renderMini();
  renderBoard();
  $("boardtext").focus();
}
function renderMini() {
  $("minicanvas").replaceChildren(boardDom($("boardtext").value, mdHooks({ onTask: null })));
  const w = $("editor").querySelector(".preview").clientWidth - 2 * PREVIEW_PAD;
  $("minicanvas").style.transform = `scale(${Math.max(PREVIEW_MIN, w / BOARD_WIDTH)})`;
}
function saveBoard() {
  const text = $("boardtext").value;
  boardEditing = false;
  writeBoard(text).then(() => renderBoard());
}
function cancelBoard() {
  boardEditing = false;
  delete sigs.board;
  renderBoard();
}
function toggleBoardFull() {
  boardFull = !boardFull;
  document.body.classList.toggle("boardfull", boardFull);
  window.__TAURI__.window.getCurrentWindow().setFullscreen(boardFull).catch(warn);
  setTimeout(placeBoard, 50);
}
function setZoom(z) {
  zoom = z ? Math.max(ZOOM_MIN, Math.min(ZOOM_MAX, Math.round(z * 10) / 10)) : 0;
  store("boardZoom", zoom);
  placeBoard();
}
new ResizeObserver(() => { if (onBoard() && !boardEditing) placeBoard(); else if (boardEditing) renderMini(); }).observe($("boardpane"));

function openSettings(pane) {
  if (!st) return;
  if (pane) settingsPane = pane;
  $("name").value = st.name;
  $("language").value = st.language;
  $("sessionsound").checked = sessionSound;
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
function renderStorage() {
  const d = st.disk;
  const used = storageInfo ? storageInfo.used_bytes : 0;
  const totalBytes = d.total || 1;
  const other = Math.max(0, totalBytes - d.free - used);
  const sets = st.sets.filter((s) => !s.live);
  const sig = [used, d, sets.map((s) => [s.id, s.progress.have, s.progress.total, s.shared]), [...picked], st.warn_hours, lang];
  if (!changed("storage", sig)) return;
  $("storedir").textContent = storageInfo ? storageInfo.dir : "";
  const seg = (cls, n) => { const i = el("i", { className: cls }); i.style.width = (n / totalBytes) * 100 + "%"; return i; };
  $("usage").replaceChildren(seg("app", used), seg("other", other));
  const rate = d.per_hour ? t("About {size} per hour · room for about {h} h", { size: bytes(d.per_hour), h: Math.floor(d.hours_left) }) : "";
  $("legend").replaceChildren(el("span", {}, el("i", { className: "app" }), t("EZ Hanseikai {size}", { size: bytes(used) })), el("span", {}, el("i", { className: "other" }), t("Other files {size}", { size: bytes(other) })), el("span", {}, el("i", { className: "free" }), t("Free {free} / {total}", { free: bytes(d.free), total: bytes(d.total) })), el("span", { className: "sp" }), el("span", { textContent: rate }));
  if (changed("warnopts", [lang])) $("warnhours").replaceChildren(...[2, 5, 10, 20, 50].map((h) => el("option", { value: h, textContent: t("{n} h of recording", { n: h }) })));
  $("warnhours").value = st.warn_hours;
  $("selold").textContent = t("All but newest {n}", { n: KEEP_NEWEST });
  for (const id of [...picked]) if (!sets.some((s) => s.id === id)) picked.delete(id);
  const biggest = Math.max(1, ...sets.map((s) => s.progress.have));
  const rows = sets.map((s) => {
    const box = el("input", { type: "checkbox", checked: picked.has(s.id), disabled: !canDelete(s), ariaLabel: sessionName(s.id) });
    box.onchange = () => { if (box.checked) picked.add(s.id); else picked.delete(s.id); delete sigs.storage; renderStorage(); };
    const notes = noteCount(s);
    const recv = onThisPc(s) && s.progress.total && s.progress.have < s.progress.total ? t("Receiving {n}%", { n: Math.floor((s.progress.have / s.progress.total) * 100) }) : "";
    return el("tr", { className: picked.has(s.id) ? "sel" : "" }, el("td", {}, box), el("td", {}, el("b", { textContent: sessionName(s.id) }), " ", el("span", { className: "muted small", textContent: voiceOnly(s) ? t("voice only") : t(notes === 1 ? "{n} note" : "{n} notes", { n: notes }) })), el("td", { textContent: setLength(s) }), el("td", { textContent: String(s.players.length) }), el("td", {}, onThisPc(s) ? [el("span", { className: "sizebar" }, el("i", { style: `width:${(s.progress.have / biggest) * 100}%` })), bytes(s.progress.have)] : el("span", { className: "muted", textContent: t("Not on this PC") })), el("td", { className: "muted small", textContent: recv }));
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
    el("div", { className: "small" + (p.form === "Others still have it" ? " muted" : " warn"), textContent: t(p.form) }))));
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
  const tab = { traffic: "network", log: "log", flow: "flow" }[diagTab];
  if (!tab || Date.now() - diagFeedAt < DIAG_FEED_MS) return;
  diagFeedAt = Date.now();
  invoke("debug_view", { tab, since: 0, set: null, addresses: false }).then((data) => { diagFeed = { tab: diagTab, data }; delete sigs.diag; renderDiagnostics(); }).catch(warn);
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
function jobsView() {
  const jobs = jobInfo ? jobInfo.jobs : [];
  const state = (j) => ({ failed: t("Failed"), done: t("Done"), claimed: t("Running") }[j.state] || t("Waiting"));
  return [diagTable([t("Plugin"), t("Recorded"), t("Target"), t("State"), t("Run by")], jobs.map((j) => [pluginName(j.plugin), sessionName(j.set), jobTarget(j), state(j), runBy(j)]), t("Turn on a plugin to analyse recordings"))];
}
function renderDiagnostics() {
  if (!$("diagdlg").open) return;
  for (const b of $("diagdlg").querySelectorAll("[data-dtab]")) b.classList.toggle("on", b.dataset.dtab === diagTab);
  pullDiag();
  const failed = jobInfo ? jobInfo.jobs.filter((j) => j.state === "failed").length : 0;
  const lines = summaryLines(st, failed);
  const fed = diagFeed && diagFeed.tab === diagTab ? diagFeed.data : null;
  if (!changed("diag", [diagTab, lines, st.peers, st.hw, st.time_sync, st.api, st.channel, fed, jobInfo && jobInfo.jobs, lang])) return;
  if (diagTab === "summary") {
    $("diagbody").replaceChildren(el("div", { className: "okbox" }, ...lines.map((l) => el("div", { className: "ok-line lv-" + l.level }, icon({ ok: "ok", warn: "alert", fail: "fail" }[l.level] || "info", "sm"), el("b", { textContent: t(l.key) }), el("span", { textContent: l.text }), l.key === "Plugins" && failed ? el("button", { className: "sm", textContent: t("Open jobs"), onclick: () => { $("diagdlg").close(); showRoomTab("jobs"); } }) : null))));
    return;
  }
  if (diagTab === "traffic" || diagTab === "log") {
    $("diagbody").replaceChildren(...(fed ? (diagTab === "traffic" ? trafficView(fed) : logView(fed)) : [el("div", { className: "empty", textContent: t("Loading…") })]));
    if (diagTab === "log") $("diagbody").scrollTop = $("diagbody").scrollHeight;
    return;
  }
  if (diagTab === "jobs") {
    $("diagbody").replaceChildren(...jobsView());
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
  const failed = jobInfo ? jobInfo.jobs.filter((j) => j.state === "failed").length : 0;
  const head = summaryLines(st, failed).map((l) => `[${l.level}] ${l.key}: ${l.text}`).join("\n");
  return invoke("debug_report", { audioProcess: st.audio_process, save: false, head });
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
for (const b of $("boardscope").children) b.onclick = () => setBoardScope(b.dataset.scope);
$("roomnameinput").oninput = () => { const v = $("roomnameinput").value.trim(); $("roomsave").disabled = !v || v === st.room.name; };
$("roomnameinput").onkeydown = (e) => { if (e.key === "Enter" && !composing(e) && !$("roomsave").disabled) $("roomsave").click(); };
$("roomsave").onclick = () => { $("roomdlg").close(); run("rename_room", { name: $("roomnameinput").value }, () => { flash(t("Saved")); refresh(); }); };
$("roomno").onclick = () => $("roomdlg").close();
$("zoomin").onclick = () => setZoom((zoom || fitZoom()) + ZOOM_STEP);
$("zoomout").onclick = () => setZoom((zoom || fitZoom()) - ZOOM_STEP);
$("zoomfit").onclick = () => setZoom(0);
$("boardfull").onclick = toggleBoardFull;
$("boardedit").onclick = editBoard;
$("boardsave").onclick = saveBoard;
$("boardcancel").onclick = cancelBoard;
$("boardtext").oninput = renderMini;
$("boardtext").onkeydown = (e) => {
  e.stopPropagation();
  if (e.key === "Enter" && e.ctrlKey) { e.preventDefault(); saveBoard(); }
  if (e.key === "Escape") { e.preventDefault(); cancelBoard(); }
};
for (const b of $("setnav").querySelectorAll("button")) b.onclick = () => { settingsPane = b.dataset.pane; showPane(); };
$("promptcopy").onclick = () => navigator.clipboard.writeText(promptFull).then(() => { $("promptdlg").close(); flash(t("Prompt copied")); }).catch(fail);
$("aibtn").onclick = (e) => openAiMenu(e.currentTarget, newestRecording());
$("aiplay").onclick = (e) => openAiMenu(e.currentTarget, selected);
$("apibadge").onclick = () => openSettings("api");
for (const b of $("apiaccess").children) b.onclick = () => run("set_api_lan", { lan: b.dataset.lan === "1" });
$("apidoc").onclick = () => run("open_llms", null, null);
$("apion").onchange = () => run("set_api", { on: $("apion").checked });
$("apicopy").onclick = () => invoke("api_info").then((info) => navigator.clipboard.writeText(info.token)).then(() => flash(t("Token copied"))).catch(fail);
$("apinew").onclick = async () => {
  if (await ask(t("Make a new token?"), t("New token"), false, t("Programs using the old token stop working."))) run("new_api_token", null, () => flash(t("New token made")));
};
$("name").onchange = () => run("set_name", { name: $("name").value });
$("language").onchange = () => run("set_language", { language: $("language").value });
$("sessionsound").onchange = () => { sessionSound = $("sessionsound").checked; store("sessionSound", sessionSound); };
$("copyid").onclick = () => navigator.clipboard.writeText(st.id).then(() => flash(t("ID copied")));
$("exportid").onclick = () => quiet("identity", { export: true });
$("importid").onclick = () => quiet("identity", { export: false });
$("opendata").onclick = $("storeopen").onclick = () => run("open_data_folder", null, null);
for (const b of $("quality").children) b.onclick = () => run("set_quality", { id: b.dataset.q });
$("limit").onchange = () => run("set_limit", { mbps: $("limit").value ? Number($("limit").value) : null });
for (const b of $("channel").children) b.onclick = () => run("set_channel", { channel: b.dataset.c }, () => { refresh(); checkUpdate(false); });
$("checkupdate").onclick = () => checkUpdate(true);
$("warnhours").onchange = () => run("set_warn_hours", { hours: Number($("warnhours").value) });
$("selnone").onclick = () => { picked.clear(); delete sigs.storage; renderStorage(); };
$("selold").onclick = () => { picked = new Set(oldSessions(st.sets.filter((s) => !s.live))); delete sigs.storage; renderStorage(); };
$("deletesel").onclick = () => confirmDelete([...picked]);
$("delno").onclick = () => $("deletedlg").close();
for (const b of $("diagdlg").querySelectorAll("[data-dtab]")) b.onclick = () => openDiagnostics(b.dataset.dtab);
$("diagcopy").onclick = () => diagnosticsText().then((text) => navigator.clipboard.writeText(text)).then(() => flash(t("Copied"))).catch(fail);
$("diagsave").onclick = () => diagnosticsText().then((text) => invoke("save_text", { name: "ez-hanseikai-diagnostics.txt", text })).then((saved) => { if (saved) flash(t("Saved")); }).catch(fail);
$("resetno").onclick = () => $("resetdlg").close();
$("resetyes").onclick = () => {
  $("resetdlg").close();
  const keepRooms = $("keeprooms").checked;
  if (!keepRooms) unstore((k) => k.startsWith(planKey("")));
  run("reset", { keepRooms }, null);
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
$("rfolder").onclick = () => run("open_data_folder", null, null);
$("rreport").onclick = () => run("debug_report", { audioProcess: "", save: true }, (p) => { $("rinfo").textContent = p; });
$("rreset").onclick = openReset;
$("rcontinue").onclick = () => run("dismiss_recovery");
for (const d of document.querySelectorAll("dialog")) {
  d.addEventListener("click", (e) => {
    const r = d.getBoundingClientRect();
    const outside = e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom;
    if ((e.target === d && outside) || e.target.closest(".close")) d.close();
  });
}

$("importfolder").onclick = () => invoke("pick_import", { folder: true }).then((spec) => { if (spec) planImport(spec); }).catch(fail);
$("importfiles").onclick = () => invoke("pick_import", { folder: false }).then((spec) => { if (spec) planImport(spec); }).catch(fail);
$("importgo").onclick = startImport;
$("importtitle").onkeydown = (e) => { if (e.key === "Enter" && !composing(e) && !$("importgo").disabled) $("importgo").click(); };
$("folderno").onclick = () => $("folderdlg").close();
$("folderyes").onclick = startFolderExport;
window.__TAURI__.event.listen("tauri://drag-drop", (e) => {
  const paths = (e.payload && e.payload.paths) || [];
  if (paths.length && inRoom() && !importing) openImport({ files: paths });
});

const menuActions = {
  "import-recording": () => openImport(),
  "export-folder": () => exportFolder(playSet()),
  "settings": () => openSettings(),
  "copy-invite": copyInvite,
  "cancel-invite": cancelInvite,
  "rename-room": openRoomSettings,
  "switch-room": switchRoom,
  "leave-room": leaveRoom,
  "add-note": openQuick,
  "prev-session": () => stepSet(-1),
  "next-session": () => stepSet(1),
  "export-session": () => exportSet(playSet()),
  "delete-session": () => deleteSession(playSet()),
  "page-live": openLive,
  "page-board": showBoard,
  "page-members": () => showRoomTab("members"),
  "page-plugins": () => showRoomTab("plugins"),
  "sidebar": toggleSidebar,
  "events-panel": togglePanel,
  "fullscreen": toggleFull,
  "all-players": () => setView(current(), 0),
  "next-player": () => cycle(current(), 1),
  "prev-player": () => cycle(current(), -1),
  "shortcuts": () => openSettings("shortcuts"),
  "update": () => checkUpdate(true),
  "diagnostics": () => openDiagnostics(),
  "reset": openReset,
  "toggle-recording": toggleRecording,
};
window.__TAURI__.event.listen("menu", (e) => { if (menuActions[e.payload]) menuActions[e.payload](); });
window.__TAURI__.event.listen("joined", (e) => flash(t("{name} joined", { name: e.payload })));
window.__TAURI__.event.listen("session", (e) => { flash(sessionNotice(e.payload.action, e.payload.by)); chime(e.payload.action); });
window.__TAURI__.event.listen("flash", (e) => flash(e.payload));

fillIcons();
reportCodecs();
setInterval(() => { if (!document.hidden) tick(); }, TICK_MS);
setInterval(() => { if (!document.hidden) refresh().catch(warn); }, REFRESH_MS);
setInterval(pollPlugins, JOBS_MS);
refresh();
setTimeout(() => checkUpdate(false), UPDATE_CHECK_DELAY_MS);
setInterval(() => checkUpdate(false), UPDATE_EVERY_MS);
window.__TAURI__.event.listen("shown", () => { if (Date.now() - checkedAt > UPDATE_RECHECK_MS) checkUpdate(false); });

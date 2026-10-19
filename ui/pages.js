const BOARD_WIDTH = 1080;
const ZOOM_STEP = 0.1;
const ZOOM_MIN = 0.5;
const ZOOM_MAX = 2;
const BOARD_TEMPLATE = {
  en: "# This week\n**Move ==before the ring closes==**\n\n## Next match\n- Pick the drop before jumping\n- Call ==direction and count== on contact\n\n## Watch out\n- Don't push alone\n- Heal behind cover\n\n## Team rules\n- When unsure, high ground\n- Next move over blame\n\n## Moments to rewatch\n- Right-click a video to copy a moment link, then paste it here",
  ja: "# 今週の目標\n**==リングが閉じる前に==動く**\n\n## 次の試合でやること\n- 降りる場所を決めてから飛ぶ\n- 接敵したら==方角と人数==を伝える\n\n## 気をつけること\n- 1人で突っ込まない\n- 回復は遮蔽の裏で\n\n## チームの約束\n- 迷ったら高所\n- 責めるより次の動き\n\n## 見返す場面\n- 動画を右クリックして場面のリンクをコピーし、ここに貼る",
};
const SHORTCUTS = [
  ["Room / Live / Game plan", ["Ctrl+1", "Ctrl+2", "Ctrl+3"]],
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
  ["Developer tools", ["Ctrl+Shift+I"]],
];

let askName = false;
let joining = false;
let roomTab = "members";
let drawerFor = null;
let checks = {};
let checking = {};
let jobFilter = "all";
let boardEditing = false;
let boardFull = false;
let zoom = stored("boardZoom", 0);
let settingsPane = "general";
let storageInfo = null;
let picked = new Set();
let diagTab = "summary";

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
    const enter = () => run("switch_room", { id: r.id }, () => { go("room"); });
    const more = iconButton("more", t("More"), (e) => { e.stopPropagation(); openMenu([{ label: t("Remove from this PC…"), icon: "trash", danger: true, run: () => removeRoom(r) }], e.currentTarget); }, "icon sm more");
    const row = el("div", { className: "rrow", tabIndex: 0, role: "button" }, el("b", { textContent: r.name }), el("span", { className: "avs" }, ...r.members.slice(0, AVATARS_SHOWN).map(face)), el("span", { className: "sp" }), r.used_us ? el("span", { className: "muted small", textContent: dayLabel(dayOf(new Date(r.used_us / 1000)), new Date(), lang) }) : null, el("button", { className: "sm" + (i ? "" : " primary"), textContent: t("Enter"), onclick: (e) => { e.stopPropagation(); enter(); } }), more);
    row.onclick = enter;
    row.onkeydown = (e) => { if (e.key === "Enter") enter(); };
    return row;
  }));
}
async function removeRoom(r) {
  if (await ask(t("Remove {room} from this PC?", { room: r.name }), t("Remove"), true, t("Its recordings and notes on this PC are deleted. Other members keep theirs."))) run("remove_room", { id: r.id });
}
function joinWith(input) {
  if (joining) return;
  joining = true;
  run("join_room", { code: input.value }, () => { input.value = ""; refresh(); }).finally(() => { joining = false; });
}
const copyInvite = () => invoke("create_invite", { reuse: true }).then((code) => navigator.clipboard.writeText(code)).then(() => { flash(t("Invite code copied")); refresh(); }).catch(fail);
const cancelInvite = () => run("cancel_invites", null, () => { flash(t("Invite cancelled")); refresh(); });
const createRoom = () => invoke("create_room", { name: "" }).then(() => { go("room"); return refresh().then(copyInvite); }).catch(fail);
async function leaveRoom() {
  if (!st.room) return;
  if (st.recording && !(await ask(t("Stop recording and leave {room}?", { room: st.room.name }), t("Leave room"), true))) return;
  run("leave_room", { id: st.room.id }, () => { endReview(); selectSet(null); go("room"); });
}
async function removeMember(id) {
  if (await ask(t("Remove {name} from the room?", { name: nameOf(id) }), t("Remove from room"), true, t("Everyone stops sending to them. What they already have stays with them."))) { drawerFor = null; run("remove_peer", { id }); }
}
function openRename() {
  if (!st.room) return;
  $("renameinput").value = st.room.name;
  $("renamedlg").showModal();
  $("renameinput").select();
}

function renderRoom() {
  if (page !== "room" || !inRoom()) return;
  const all = people();
  const online = 1 + st.peers.filter((p) => p.connected).length;
  $("homename").textContent = st.room.name;
  $("homesub").textContent = t("{n} members · {online} online", { n: all.length, online });
  for (const b of document.querySelectorAll("#roompage .ptabs button")) { const on = b.dataset.tab === roomTab; b.classList.toggle("on", on); b.setAttribute("aria-selected", on); }
  const failed = jobInfo ? jobInfo.jobs.filter((j) => j.state !== "done").length : 0;
  $("jobcount").hidden = !failed;
  $("jobcount").textContent = String(failed);
  $("boardpane").hidden = roomTab !== "board";
  $("roomtab").parentElement.hidden = roomTab === "board";
  if (roomTab === "board") renderBoard();
  else if (roomTab === "members") renderMembers();
  else if (roomTab === "plugins") renderPlugins();
  else renderJobs();
  renderDrawer();
}
function renderMembers() {
  const rows = people().map((id) => ({ id, p: id === st.id ? null : peerOf(id) }));
  const sig = ["members", rows.map((r) => [r.id, r.p && [r.p.connected, r.p.path, r.p.rtt_ms, r.p.version, r.p.failures]]), st.asks, st.invites, drawerFor, st.names, st.colors, lang];
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
    const state = me ? null : memberState(p);
    const conn = me || !p.connected ? el("span", { className: "faint", textContent: "—" }) : el("span", { className: p.path === "relay" ? "warn" : "ok" }, icon("relay", "sm"), p.path === "relay" ? t("Relay") : t("Direct"));
    const version = me ? st.version : p && p.version;
    const tr = el("tr", { className: "pick" + (drawerFor === id ? " sel" : "") + (state && state.off ? " dim" : ""), tabIndex: 0 },
      el("td", {}, el("span", { className: "who" }, avatar(id, "", me || (p && p.connected) ? (recOf(id) ? "rec" : "on") : null), el("b", { textContent: nameOf(id) }), me ? el("span", { className: "pill gray", textContent: t("You") }) : null)),
      el("td", {}, el("span", { className: "mono", textContent: `${id.slice(0, 4)}·${id.slice(4, 8)}·${id.slice(8, 12)}` }), iconButton("copy", t("Copy ID"), (e) => { e.stopPropagation(); navigator.clipboard.writeText(id).then(() => flash(t("ID copied"))); })),
      el("td", { className: state && state.off ? "faint" : "", textContent: me ? t("Online") : p.connected ? t("Online") : state.text }),
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
  } else if (!me) path = el("div", { className: "pathbox off" }, el("b", { textContent: memberState(p).text }));
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
    { label: t("Run next on this PC"), icon: "play", disabled: j.state !== "waiting" || !j.here || !(plugin && plugin.process_here), run: () => invoke("run_job_next", { job: j.job }).then(pollPlugins) },
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
  const by = (j) => (j.by ? t("{name}'s PC", { name: nameOf(j.by) }) : "—");
  const rows = shown.map((j) => {
    const model = j.settings && j.settings.model;
    const more = iconButton("more", t("More"), (e) => { e.stopPropagation(); jobMenu(j, e.currentTarget); });
    const tr = el("tr", {}, el("td", {}, el("b", { textContent: pluginName(j.plugin) }), model ? el("span", { className: "mono muted", textContent: " " + model }) : null), el("td", { textContent: sessionName(j.set) }), el("td", { textContent: jobTarget(j) }), el("td", {}, state(j)), el("td", { textContent: by(j) }), el("td", { className: "chev" }, more));
    tr.oncontextmenu = (e) => { e.preventDefault(); jobMenu(j, e); };
    const err = j.state === "failed" && j.error ? el("tr", { className: "err" }, el("td", { colSpan: 6 }, icon("info", "sm"), t("Failed: {error}", { error: j.error }))) : null;
    return [tr, err];
  }).flat().filter(Boolean);
  const head = el("tr", {}, ...["Plugin", "Recorded", "Target", "State", "Run by", ""].map((h) => el("th", { textContent: h ? t(h) : "" })));
  const table = shown.length ? el("div", { className: "tablebox" }, el("table", { className: "table" }, el("thead", {}, head), el("tbody", {}, ...rows))) : el("div", { className: "empty", textContent: jobs.length ? t("Nothing here") : t("Turn on a plugin to analyse recordings") });
  $("roomtab").replaceChildren(el("div", { className: "row" }, seg), table);
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
function showBoard() {
  roomTab = "board";
  go("room");
}
const boardText = () => { const e = boardEvent(); return e ? e.text : BOARD_TEMPLATE[lang === "ja" ? "ja" : "en"]; };
function boardSections(text) {
  const out = { title: "", lead: [], sections: [] };
  let cur = null;
  for (const line of text.split("\n")) {
    const h1 = line.match(/^#\s+(.*)$/);
    const h2 = line.match(/^##\s+(.*)$/);
    if (h1 && !out.title && !out.sections.length) { out.title = h1[1]; continue; }
    if (h2) { cur = { title: h2[1], lines: [] }; out.sections.push(cur); continue; }
    if (!line.trim()) continue;
    (cur ? cur.lines : out.lead).push(line);
  }
  return out;
}
function boardDom(text) {
  const b = boardSections(text);
  const card = el("div", { className: "bcard" });
  if (b.title) card.append(el("h2", { className: "title" }, ...inline(b.title)));
  if (b.lead.length) card.append(el("div", { className: "lead" }, ...markdown(b.lead.join("\n"))));
  if (b.sections.length) card.append(el("div", { className: "bgrid" }, ...b.sections.map((s) => el("section", { className: "bsec" + (s.lines.some((l) => l.includes("ezh://")) ? " wide" : "") }, el("h3", {}, ...inline(s.title)), ...markdown(s.lines.join("\n"))))));
  return card;
}
function fitZoom() {
  const w = $("boardwrap").clientWidth - 2 * 28;
  return Math.max(ZOOM_MIN, Math.min(1, w / BOARD_WIDTH));
}
function placeBoard() {
  const z = zoom || fitZoom();
  $("canvas").style.transform = `scale(${z})`;
  $("canvas").style.marginBottom = `${($("canvas").offsetHeight * (z - 1))}px`;
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
  const e = boardEvent();
  const sub = e ? t("Updated by {name} · {time}", { name: nameOf(e.author), time: new Date((e.local_us || e.id) / 1000).toLocaleString(lang === "ja" ? "ja-JP" : "en-US", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }) }) : "";
  $("boardsub").textContent = boardEditing ? "" : sub;
  if (boardEditing) return;
  if (changed("board", [boardText(), lang, st.names, jobInfo && jobInfo.tags])) $("canvas").replaceChildren(boardDom(boardText()));
  placeBoard();
}
function editBoard() {
  showBoard();
  boardEditing = true;
  $("boardtext").value = boardText();
  renderMini();
  renderBoard();
  $("boardtext").focus();
}
function renderMini() {
  $("minicanvas").replaceChildren(boardDom($("boardtext").value));
  const w = $("editor").querySelector(".preview").clientWidth - 32;
  $("minicanvas").style.transform = `scale(${Math.max(0.2, w / BOARD_WIDTH)})`;
}
function saveBoard() {
  const text = $("boardtext").value;
  boardEditing = false;
  delete sigs.board;
  run("add_event", { kind: "board", text, target: {}, values: { card: "team" } }, () => refresh());
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
  $("apierrrow").hidden = !st.api.error;
  $("apierr").textContent = st.api.error ? t(st.api.error) : "";
  if (settingsPane === "storage") renderStorage();
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
  const sig = [used, d, sets.map((s) => [s.id, s.progress.have, s.progress.total]), st.gone.map((g) => g.id), [...picked], st.warn_hours, lang];
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
    const box = el("input", { type: "checkbox", checked: picked.has(s.id), disabled: !s.progress.have, ariaLabel: sessionName(s.id) });
    box.onchange = () => { if (box.checked) picked.add(s.id); else picked.delete(s.id); delete sigs.storage; renderStorage(); };
    const notes = noteCount(s);
    const recv = s.progress.total && s.progress.have < s.progress.total ? t("Receiving {n}%", { n: Math.floor((s.progress.have / s.progress.total) * 100) }) : "";
    return el("tr", { className: picked.has(s.id) ? "sel" : "" }, el("td", {}, box), el("td", {}, el("b", { textContent: sessionName(s.id) }), " ", el("span", { className: "muted small", textContent: voiceOnly(s) ? t("voice only") : t(notes === 1 ? "{n} note" : "{n} notes", { n: notes }) })), el("td", { textContent: setLength(s) }), el("td", { textContent: String(s.players.length) }), el("td", {}, el("span", { className: "sizebar" }, el("i", { style: `width:${(s.progress.have / biggest) * 100}%` })), bytes(s.progress.have)), el("td", { className: "muted small", textContent: recv }));
  });
  const gone = st.gone.map((g) => el("tr", { className: "dim" }, el("td"), el("td", {}, el("b", { textContent: sessionName(g.id) })), el("td", { textContent: g.end_us ? fmt((g.end_us - g.start_us) / 1e6) : "" }), el("td", { textContent: String(g.players.length) }), el("td", { textContent: "—" }), el("td", { className: "small", textContent: t("Not on this PC") })));
  const head = el("tr", {}, ...["", "Recorded", "Length", "People", "On this PC", ""].map((h) => el("th", { textContent: h ? t(h) : "" })));
  $("storetable").replaceChildren(el("table", { className: "table" }, el("thead", {}, head), el("tbody", {}, ...rows, ...gone)));
  const size = sets.filter((s) => picked.has(s.id)).reduce((n, s) => n + s.progress.have, 0);
  $("selsum").textContent = picked.size ? t("{n} selected · {size}", { n: picked.size, size: bytes(size) }) : "";
  $("deletesel").disabled = !picked.size;
}
function confirmDelete(ids) {
  const sets = ids.map(setById).filter((s) => s && !s.live);
  if (!sets.length) return;
  const preview = deletePreview(sets, st.id);
  const size = preview.reduce((n, p) => n + p.bytes, 0);
  $("deltitle").textContent = t(sets.length === 1 ? "Delete the videos of this recording?" : "Delete the videos of {n} recordings?", { n: sets.length });
  $("delsub").textContent = t("Frees {size} on this PC", { size: bytes(size) });
  $("dellist").replaceChildren(...preview.map((p) => {
    const alone = p.players.filter((x) => x.alone).map((x) => nameOf(x.owner));
    return el("div", { className: "delcard" }, el("div", { className: "row" }, el("b", { textContent: sessionName(p.id) }), el("span", { className: "sp" }), el("span", { textContent: bytes(p.bytes) })), el("div", { className: "mono muted small", textContent: `rooms\\${st.room.id}\\…` }), el("div", { className: "who" }, ...p.players.map((x) => el("span", {}, icon("film", "sm"), x.owner === st.id ? t("{name} (you)", { name: nameOf(x.owner) }) : nameOf(x.owner), el("span", { className: "muted", textContent: bytes(x.bytes) })))), alone.length ? el("div", { className: "warn small" }, icon("alert", "sm"), t("Nobody else has {names}'s part", { names: alone.join(", ") })) : "");
  }));
  $("delyes").replaceChildren(icon("trash", "sm"), t("Delete {size}", { size: bytes(size) }));
  $("delyes").onclick = async () => {
    $("deletedlg").close();
    for (const s of sets) await invoke("delete_set", { room: s.room, set: s.id }).catch(fail);
    for (const s of sets) picked.delete(s.id);
    if (sets.some((s) => s.id === selected)) selectSet(null);
    flash(t("Deleted from this PC"));
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
function renderDiagnostics() {
  if (!$("diagdlg").open) return;
  for (const b of $("diagdlg").querySelectorAll("[data-dtab]")) b.classList.toggle("on", b.dataset.dtab === diagTab);
  const failed = jobInfo ? jobInfo.jobs.filter((j) => j.state === "failed").length : 0;
  const lines = summaryLines(st, failed);
  if (!changed("diag", [diagTab, lines, st.peers, lang])) return;
  if (diagTab === "summary") {
    $("diagbody").replaceChildren(el("div", { className: "okbox" }, ...lines.map((l) => el("div", { className: "ok-line lv-" + l.level }, icon({ ok: "ok", warn: "alert", fail: "fail" }[l.level] || "info", "sm"), el("b", { textContent: t(l.key) }), el("span", { textContent: l.text }), l.key === "Plugins" && failed ? el("button", { className: "sm", textContent: t("Open jobs"), onclick: () => { $("diagdlg").close(); roomTab = "jobs"; go("room"); } }) : null))));
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
  const other = [["globe", "Address lookup", t("Publishes only the relay address")], ["refresh", "Update check", t("github.com · at start and every 4 h · {channel} channel", { channel: st.channel === "beta" ? t("Beta") : t("Stable") })], ["download", "Downloads", t("Video tools, plugin tools and models, only when needed")]];
  $("diagbody").replaceChildren(diagram, el("div", { className: "tablebox" }, el("table", { className: "table" }, el("thead", {}, head), el("tbody", {}, ...rows))), el("div", { className: "okbox" }, el("div", { className: "h3", textContent: t("Other connections") }), ...other.map(([i, k, v]) => el("div", { className: "ok-line" }, icon(i, "sm"), el("b", { textContent: t(k) }), el("span", { textContent: v })))));
}
async function diagnosticsText() {
  const failed = jobInfo ? jobInfo.jobs.filter((j) => j.state === "failed").length : 0;
  const head = summaryLines(st, failed).map((l) => `[${l.level}] ${l.key}: ${l.text}`).join("\n");
  const report = await invoke("debug_report", { includeAddresses: $("diagaddr").checked, everyone: false, audioProcess: st.audio_process, save: false });
  return `EZ Hanseikai ${st.version}\n${head}\n\n${report}`;
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
$("renamebtn").onclick = openRename;
$("invitebtn").onclick = copyInvite;
$("leavebtn").onclick = leaveRoom;
for (const b of document.querySelectorAll("#roompage .ptabs button")) b.onclick = () => { roomTab = b.dataset.tab; delete sigs.roomtab; renderRoom(); if (roomTab !== "members") $("drawer").hidden = true; };
$("renamesave").onclick = () => run("rename_room", { name: $("renameinput").value }, () => { $("renamedlg").close(); refresh(); });
$("renameinput").onkeydown = (e) => { if (e.key === "Enter" && !composing(e)) $("renamesave").click(); };
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
$("aibtn").onclick = (e) => openAiMenu(e.currentTarget, newestRecording());
$("aiplay").onclick = (e) => openAiMenu(e.currentTarget, selected);
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
$("resetyes").onclick = () => { $("resetdlg").close(); run("reset", { keepRooms: $("keeprooms").checked }, null); };
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
$("rreport").onclick = () => run("save_report", { includeAddresses: false, audioProcess: "" }, (p) => { $("rinfo").textContent = p; });
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
  "settings": () => openSettings(),
  "copy-invite": copyInvite,
  "cancel-invite": cancelInvite,
  "rename-room": openRename,
  "leave-room": leaveRoom,
  "add-note": openQuick,
  "prev-session": () => stepSet(-1),
  "next-session": () => stepSet(1),
  "export-session": () => exportSet(playSet()),
  "delete-session": () => deleteSession(playSet()),
  "page-room": () => goPage("room"),
  "page-now": () => goPage("now"),
  "page-board": () => goPage("board"),
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

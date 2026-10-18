const STEP = { small: 1, normal: 5, big: 10, huge: 60 };
const VOLUME_STEP = 0.05;
const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3];
const MAX_KEYED = 6;
const TICK_MS = 150;
const REFRESH_MS = 1000;
const IDLE_MS = 2000;
const OSD_MS = 2500;
const TOAST_MS = 4000;
const UNDO_MS = 8000;
const UPDATE_CHECK_DELAY_MS = 4000;
const UPDATE_EVERY_MS = 4 * 3600 * 1000;
const UPDATE_RECHECK_MS = 10 * 60 * 1000;
const DOUBLE_CLICK_MS = 250;
const JUMP_MIN = 1;
const METER_MIN_DB = -60;
const METER_TICKS = [-60, -40, -20, -10, 0];
const METER_YELLOW_DB = -20;
const METER_RED_DB = -9;
const PEAK_HOLD_MS = 1500;
const CLIP_HOLD_MS = 3000;
const HEARD_MS = 10000;
const OFFLINE_FAILURES = 2;
const NOTE_STEP_GAP = 0.5;
const JOBS_MS = 5000;
const TAG_HINTS = 5;
const AVATARS_SHOWN = 5;
const CODEC_PROBE = { width: 1280, height: 720, bitrate: 6000000, framerate: 30 };
const GAP = 6;

const tileGrid = (box, kind) => ({ el: box, kind, tiles: {}, keys: [], mode: "grid", slots: 0, view: 0, last: 1, free: [] });
const rec = tileGrid($("livetiles"), "record");
const play = tileGrid($("playtiles"), "player");
const clock = { playing: false, m: 0, at: 0 };
const preview = { box: $("preview"), slot: null };
const meters = { app: { v: 0, peak: 0, at: 0, clip: 0, heard: 0 }, mic: { v: 0, peak: 0, at: 0, clip: 0, heard: 0 } };
let st = null;
let idx = null;
let ready = false;
let lang = "";
let page = "room";
let nowView = false;
let nowSet = null;
let selected = null;
let rate = 1;
let volume = stored("volume", 1);
let muted = false;
let live = false;
let watchParty = stored("watchParty", true);
let partyLast = null;
let partyLayout = null;
let drag = null;
let items = [];
let osdTimer = null;
let toastTimer = null;
let idleTimer = null;
let undo = null;
let available = null;
let installAfter = false;
let askingUpdate = false;
let checkedAt = 0;
let updateText = "";
let menuReturn = null;
let exporting = null;
let side = "events";
let scope = "session";
let subtitles = stored("subtitles", true);
let receive = stored("receive", {});
let sidebar = stored("sidebar", true);
let panel = stored("panel", true);
let panelOpen = false;
let clickTimer = null;
let quick = null;
let partyRoom = null;
let askName = false;
let jobInfo = null;
let plugins = [];
let pluginsGen = 0;
let downloading = {};
let refreshing = null;
let refreshAgain = false;
let checking = false;
let checks = {};
let dismissed = new Set();
let full = false;
let joining = false;
let listening = false;

const peerOf = (id) => st.peers.find((p) => p.id === id);
const nameOf = (id) => (id === st.id ? st.name || t("You") : st.names[id] || String(id).slice(0, 10));
const people = () => [st.id, ...st.peers.map((p) => p.id)];
const liveSet = () => st.sets.find((s) => s.live) || null;
const inRoom = () => !!st && !!st.room && !st.room.pending;
const fail = (e) => flash(e, "error");
const run = (cmd, args, then = refresh) => invoke(cmd, args).then(then).catch(fail);
const quiet = (cmd, args) => invoke(cmd, args).catch(warn);
const showingNow = () => nowView || !playSet();
const clockOf = (us) => { const d = new Date(us / 1000); return `${d.getHours()}:${String(d.getMinutes()).padStart(2, "0")}`; };

function ordered(present, saved) {
  const keep = (saved || []).filter((k) => present.includes(k));
  return [...keep, ...present.filter((k) => !keep.includes(k))];
}

function initial(name) {
  return (String(name || "?").trim().charAt(0) || "?").toUpperCase();
}

function avatar(id, size, dot) {
  const a = el("span", { className: "av" + (size ? " " + size : ""), textContent: initial(id === st.id ? st.name : st.names[id]), title: nameOf(id) });
  a.style.background = colorOf(colorIndex(id));
  if (dot) a.append(el("i", { className: dot }));
  return a;
}

function avatars(ids, size = "sm") {
  return el("span", { className: "avs" }, ...ids.slice(0, AVATARS_SHOWN).map((id) => avatar(id, size)));
}

function iconButton(name, label, onclick, cls = "icon sm") {
  const b = el("button", { className: cls, ariaLabel: label, title: label, onclick }, icon(name, "sm"));
  return b;
}

function tile(s, key) {
  if (s.tiles[key]) return s.tiles[key];
  const tl = { key, media: slot(el("video", { muted: true, preload: "auto" })), mic: null, name: el("span", { className: "nm" }), av: el("span"), dot: el("i", { className: "rd", hidden: true }), who: el("div", { className: "who" }), what: el("div", { className: "what" }), sub: el("div", { className: "subt", hidden: true }) };
  tl.lab = el("div", { className: "nl", draggable: true }, tl.av, tl.name, tl.dot);
  tl.box = el("div", { className: "tile", tabIndex: 0 }, tl.media.video, el("div", { className: "waitbox" }, tl.who, tl.what), tl.sub, tl.lab);
  const focus = () => { const i = s.keys.indexOf(key) + 1; setView(s, s.view === i ? 0 : i); };
  tl.box.onkeydown = (e) => { if (e.key === "Enter") focus(); };
  if (s === play) {
    tl.box.onpointerdown = (e) => inkDown(tl, e);
    tl.box.onpointermove = inkMove;
    tl.box.onpointerup = (e) => { if (inkUp(e)) tl.drew = true; };
    tl.box.onpointercancel = tl.box.onlostpointercapture = inkUp;
    tl.box.onclick = (e) => {
      if (e.target.closest(".nl")) return;
      if (tl.drew) { tl.drew = false; return; }
      clearTimeout(clickTimer);
      clickTimer = setTimeout(togglePlay, DOUBLE_CLICK_MS);
    };
    tl.box.ondblclick = (e) => { if (!e.target.closest(".nl")) { clearTimeout(clickTimer); focus(); } };
    tl.lab.onclick = (e) => { e.stopPropagation(); openReceive(key, tl.lab); };
    tl.box.oncontextmenu = (e) => { e.preventDefault(); stageMenu(e); };
  } else {
    tl.box.onclick = focus;
    tl.box.ondblclick = toggleFull;
  }
  tl.lab.ondragstart = (e) => e.dataTransfer.setData("text/plain", key);
  tl.box.ondragover = (e) => { e.preventDefault(); tl.box.classList.add("over"); };
  tl.box.ondragleave = () => tl.box.classList.remove("over");
  tl.box.ondrop = (e) => { e.preventDefault(); tl.box.classList.remove("over"); const from = e.dataTransfer.getData("text/plain"); if (from && from !== key && s.tiles[from]) swap(s, from, key); };
  s.el.append(tl.box);
  return (s.tiles[key] = tl);
}
function nameTile(tl, key, recording) {
  tl.name.textContent = nameOf(key);
  tl.who.textContent = nameOf(key);
  tl.dot.hidden = !recording;
  const sig = JSON.stringify([nameOf(key), colorIndex(key), st.palette]);
  if (tl.avSig !== sig) { tl.avSig = sig; tl.av.replaceChildren(avatar(key, "sm")); }
}
function prune(s, keys) {
  for (const [key, tl] of Object.entries(s.tiles)) if (!keys.includes(key)) { drop(tl.media); drop(tl.mic); tl.box.remove(); delete s.tiles[key]; }
}
function effectiveMode(s) {
  return layoutFits(s.mode, Math.max(s.keys.length, s.slots || 0)) ? s.mode : "grid";
}
function place(s) {
  const n = s.keys.length;
  if (s.view > n) s.view = 0;
  const focus = s.view ? s.keys[s.view - 1] : null;
  const order = focus ? [focus, ...s.keys.filter((k) => k !== focus)] : s.keys;
  const slots = focus ? n : Math.max(n, s.slots || 0);
  const mode = focus ? (n > 1 ? (s.mode === "side" ? "side" : "bottom") : "grid") : effectiveMode(s);
  const r = rects(n, mode, s.el.clientWidth, s.el.clientHeight, GAP, slots);
  const at = (e, b) => Object.assign(e.style, { left: b.x + "px", top: b.y + "px", width: b.w + "px", height: b.h + "px" });
  order.forEach((key, i) => {
    const tl = s.tiles[key];
    if (!tl) return;
    at(tl.box, r.tiles[i]);
    tl.box.classList.toggle("active", key === focus);
  });
  while (s.free.length > r.free.length) s.free.pop().remove();
  while (s.free.length < r.free.length) s.free.push(s.el.appendChild(el("div", { className: "tile blank" })));
  r.free.forEach((b, i) => at(s.free[i], b));
  if (s === play && st) renderInk();
}
for (const s of [rec, play]) new ResizeObserver(() => place(s)).observe(s.el);

function setView(s, view) {
  s.view = Math.max(0, Math.min(view, s.keys.length));
  if (s.view) s.last = s.view;
  place(s);
  if (s === play) { applyAudio(); renderViews(); logLayout(); share(); }
}
function swap(s, a, b) {
  const list = [...s.keys];
  const i = list.indexOf(a), j = list.indexOf(b);
  [list[i], list[j]] = [list[j], list[i]];
  s.keys = list;
  saveLayout(s, s.mode, s.slots);
}
function saveLayout(s, mode, slots) {
  s.mode = mode;
  s.slots = slots;
  if (s === rec) {
    const old = (st.layout && st.layout.order) || [];
    st.layout = { mode, slots, order: [...s.keys, ...old.filter((id) => !s.keys.includes(id))] };
    quiet("set_layout", { layout: st.layout });
  } else {
    const set = playSet();
    if (set && watchParty) { partyLayout = { set: set.id, mode, slots, order: s.keys }; share(); }
    else if (set) store("layout/" + set.id, { mode, slots, order: s.keys });
  }
  place(s);
  renderLayoutButton();
  if (s === play) logLayout();
}
const fromSaved = (l) => ({ mode: (l && l.mode) || "grid", slots: (l && l.slots) || 0, order: (l && l.order) || [] });
const playLayout = (set) => fromSaved(stored("layout/" + set.id, null) || set.layout);
function logLayout() {
  const set = playSet();
  const size = (key, sideName) => Math.round(play.tiles[key] ? play.tiles[key].box[sideName] : 0);
  if (set) quiet("log_layout", { layout: { set: set.id, mode: play.view ? "focus" : play.mode, slots: play.slots, tiles: play.keys.map((key, i) => ({ id: key, index: i, w: size(key, "clientWidth"), h: size(key, "clientHeight") })) } });
}

function recOf(id) {
  const set = liveSet();
  return !!(set && set.players.some((p) => p.owner === id && p.recording));
}
function sending() {
  return [...(st.capture.sharing ? [st.id] : []), ...st.peers.filter((p) => p.connected && p.sharing).map((p) => p.id)];
}
function livePlayers() {
  const files = { [st.id]: st.capture.live };
  for (const p of st.peers) if (p.live) files[p.id] = p.live;
  const keys = [st.id, ...st.peers.filter((p) => p.connected || files[p.id]).map((p) => p.id)];
  return keys.map((key) => ({ key, file: files[key] && files[key].size > 0 ? files[key] : null, recording: recOf(key) && (key !== st.id || st.capture.working) }));
}
function renderLive() {
  const idle = page === "player" && showingNow() && !playSet();
  $("livetiles").hidden = !idle;
  if (!idle) { prune(rec, []); return; }
  const list = livePlayers();
  const lay = fromSaved(st.layout);
  rec.keys = ordered(list.map((p) => p.key), lay.order);
  rec.mode = lay.mode;
  rec.slots = lay.slots;
  prune(rec, rec.keys);
  for (const p of list) {
    const tl = tile(rec, p.key);
    nameTile(tl, p.key, p.recording);
    tl.what.replaceChildren(p.key === st.id ? (st.capture.source ? (st.share ? t("Starting…") : t("Not sent to the room")) : t("Choose what to capture")) : t("Not sending"));
    tl.box.classList.toggle("wait", !p.file);
    if (p.file) feed(tl.media, p.file, true);
  }
  place(rec);
}
function renderPreview() {
  const own = page === "capture" && st.capture.live && st.capture.live.size > 0 ? st.capture.live : null;
  if (own) {
    if (!preview.slot) preview.slot = slot(preview.box.appendChild(el("video", { muted: true, preload: "auto" })));
    feed(preview.slot, own, true);
  } else if (preview.slot) { drop(preview.slot); preview.slot = null; }
}

function memberState(p) {
  if (!p.connected) return { text: p.failures > OFFLINE_FAILURES ? t("Offline") : t("Connecting…"), off: true };
  if (p.other_version) return { text: t("Other version"), warn: true };
  return { text: `${p.rtt_ms} ms`, warn: p.path === "relay" };
}
function worst(lines) {
  const levels = (lines || []).map((l) => l.level);
  return levels.includes("fail") ? "fail" : levels.includes("warn") ? "warn" : lines ? "ok" : null;
}

function renderShell() {
  const room = inRoom();
  $("start").hidden = room;
  $("app").hidden = !room;
  $("roompage").hidden = page !== "room";
  $("capturepage").hidden = page !== "capture";
  $("playerpage").hidden = page !== "player";
  document.body.classList.toggle("noside", !sidebar);
  document.body.classList.toggle("nopanel", !panel || (page === "player" && !playSet()));
  document.body.classList.toggle("panelopen", panelOpen && !!playSet());
  document.body.classList.toggle("full", full);
}

function renderSide() {
  if (!inRoom()) return;
  $("roomname").textContent = st.room.name;
  $("navroom").classList.toggle("on", page === "room");
  renderNow();
  renderSessions();
  renderMembers();
  renderYou();
}

function renderNow() {
  const a = st.active_set;
  const box = $("now");
  box.classList.toggle("rec", !!a);
  box.classList.toggle("on", page === "player" && showingNow());
  const who = sending();
  if (changed("nowicon", [!!a, who.length > 0])) $("nowicon").replaceChildren(icon(a ? "rec" : "radio"));
  $("nowicon").classList.toggle("live", !a && who.length > 0);
  const b = $("rec");
  if (a) {
    $("nowtitle").textContent = fmt((nowUs() - a.start_us) / 1e6);
    if (changed("nowsub", [a.by, a.start_us, lang])) $("nowsub").replaceChildren([a.by, clockOf(a.start_us)].filter(Boolean).join(" · "));
    if (changed("recbtn", ["stop", lang])) { b.className = "sm rbtn dangerfill"; b.replaceChildren(t("End session")); }
    b.disabled = false;
    b.title = "";
  } else {
    $("nowtitle").textContent = t("Live");
    if (changed("nowsub", [who, st.names, st.colors, lang])) $("nowsub").replaceChildren(who.length ? avatars(who) : el("span", { className: "faint", textContent: t("Nobody is sending") }));
    if (changed("recbtn", ["start", lang])) { b.className = "sm rbtn"; b.replaceChildren(el("i", { className: "recdot" }), t("Start session")); }
    b.disabled = !st.capture.source;
    b.title = b.disabled ? t("Choose what to capture") : "";
  }
}

function sessionState(s) {
  const watching = watchParty && st.party.on && st.party.shared.set === s.id ? st.party.watching.filter((id) => id !== st.id).length : 0;
  const running = jobInfo && jobInfo.running;
  const runningJob = running && jobInfo.jobs.find((j) => j.job === running.job);
  if (exporting && exporting.set === s.id) return { icon: "film", text: `${Math.floor(exporting.percent)}%` };
  if (watching) return { icon: "users", text: String(watching + 1), cls: "acc" };
  if (s.progress.total && s.progress.have < s.progress.total) {
    const n = Math.floor((s.progress.have / s.progress.total) * 100);
    return s.progress.stuck ? { icon: "alert", text: t("Stopped at {n}%", { n }), cls: "warn", title: lackingText(s) } : { icon: "download", text: `${n}%`, title: lackingText(s) };
  }
  if (runningJob && runningJob.set === s.id) return { icon: "puzzle", text: `${Math.floor((running.progress || 0) * 100)}%`, title: pluginName(running.plugin) };
  if (jobInfo && jobInfo.jobs.some((j) => j.set === s.id && j.state === "failed")) return { icon: "fail", text: "", cls: "warn", title: t("Analysis failed") };
  return null;
}
function lackingText(s) {
  return (s.lacking || []).map((l) => t("{name}: {n} files left", { name: nameOf(l.id), n: l.n })).join(" · ");
}
function noteCount(s) {
  return s.events.filter((k) => { const e = idx.byKey.get(k); return e && e.kind === "note"; }).length;
}
function renderSessions() {
  const list = st.sets.filter((s) => !s.live);
  const now = new Date();
  const rows = list.map((s) => ({ id: s.id, day: sessionDay(s.id), len: setLength(s), who: s.players.map((p) => p.owner), notes: noteCount(s), state: sessionState(s), on: page === "player" && !showingNow() && selected === s.id }));
  if (!changed("sessions", [rows, lang, st.colors, st.names, dayOf(now)])) return;
  if (!rows.length) { $("sessions").replaceChildren(el("div", { className: "none", textContent: t("No sessions yet") })); return; }
  let day = null;
  const out = [];
  for (const r of rows) {
    if (r.day !== day) { day = r.day; out.push(el("div", { className: "day", textContent: dayLabel(day, now, lang) })); }
    const more = iconButton("more", t("More"), (e) => { e.stopPropagation(); sessionMenu(r.id, e.currentTarget); }, "icon sm more");
    const stateEl = r.state ? el("span", { className: "st " + (r.state.cls || ""), title: r.state.title || "" }, icon(r.state.icon, "sm"), r.state.text) : null;
    const row = el("div", { className: "srow" + (r.on ? " on" : "") + (r.state && r.state.cls === "acc" ? " wp" : ""), role: "option", tabIndex: 0, ariaSelected: r.on },
      el("span", { className: "t", textContent: sessionClock(r.id) }), el("span"), el("span", { className: "d", textContent: r.len }),
      el("div", { className: "sub" }, avatars(r.who), r.notes ? el("span", { textContent: t(r.notes === 1 ? "{n} note" : "{n} notes", { n: r.notes }) }) : null, stateEl), more);
    row.onclick = () => openSession(r.id);
    row.onkeydown = (e) => { if (e.key === "Enter") openSession(r.id); };
    row.oncontextmenu = (e) => { e.preventDefault(); sessionMenu(r.id, e); };
    out.push(row);
  }
  $("sessions").replaceChildren(...out);
}

function renderMembers() {
  const catching = new Set(st.party.on ? (st.party.status || []).filter((s) => s.kind === "catching").map((s) => s.who) : []);
  const rows = st.peers.map((p) => ({ id: p.id, name: nameOf(p.id), state: memberState(p), rec: recOf(p.id), check: worst(checks[p.id]), catching: catching.has(p.id) }));
  if (!changed("members", [rows, st.asks, st.invites, st.colors, lang])) return;
  const out = [];
  for (const a of st.asks) {
    const answer = (approve) => () => run("answer_ask", { id: a.id, approve });
    out.push(el("div", { className: "mrow ask" }, el("span", { className: "av", textContent: initial(a.name), style: "background:var(--muted)" }), el("span", { className: "nm" }, el("b", { textContent: a.name || a.id.slice(0, 10) }), " ", el("span", { className: "muted", textContent: t("wants to join") })),
      el("div", { className: "acts" }, el("button", { className: "primary sm", textContent: t("Approve"), onclick: answer(true) }), el("button", { className: "sm", textContent: t("Decline"), onclick: answer(false) }))));
  }
  for (const r of rows) {
    const end = el("span", { className: "end" + (r.state.warn ? " warn" : "") }, r.catching ? el("span", { title: t("Catching up"), className: "spin" }, icon("spinner", "sm")) : null, r.check === "fail" ? icon("fail", "sm bad") : r.state.warn ? icon("alert", "sm") : null, r.state.text);
    const row = el("div", { className: "mrow" + (r.state.off ? " off" : ""), tabIndex: 0, role: "button" }, avatar(r.id, r.state.off ? "dim" : "", r.rec ? "rec" : null), el("span", { className: "nm", textContent: r.name }), end);
    row.onclick = () => memberPop(r.id, row);
    row.onkeydown = (e) => { if (e.key === "Enter") memberPop(r.id, row); };
    row.oncontextmenu = (e) => { e.preventDefault(); memberPop(r.id, row); };
    out.push(row);
  }
  if (st.invites) out.push(el("div", { className: "mrow invite" }, icon("link", "sm"), el("span", { className: "nm", textContent: t("Invite code open") }), el("span", { className: "end" }, iconButton("copy", t("Copy invite code"), copyInvite), iconButton("x", t("Cancel invite"), cancelInvite))));
  $("members").replaceChildren(...out);
}

function sourceLabel(c) {
  if (!c.source) return t("Choose what to capture");
  return c.source.kind === "window" ? c.source.title : t("Display {n}", { n: c.source.index });
}
function captureProblem(c) {
  if (!c.source) return t("Choose what to capture");
  if (c.source_lost) return t("Capture source closed");
  if (c.status) return t("Capture stopped. Retrying…");
  return "";
}
function renderYou() {
  const c = st.capture;
  const problem = captureProblem(c);
  const dot = problem ? "warn" : st.recording && c.working ? "rec" : c.sharing ? "live" : null;
  if (changed("youav", [st.name, dot, st.colors])) $("youav").replaceChildren(avatar(st.id, "lg", dot));
  $("youname").textContent = st.name;
  $("you").classList.toggle("on", page === "capture");
  if (changed("yousrc", [sourceLabel(c), problem, lang])) {
    $("yousrc").className = problem ? "warn" : "";
    $("yousrc").replaceChildren(icon(problem ? "alert" : c.source && c.source.kind === "display" ? "monitor" : "window", "sm"), el("span", { textContent: problem && c.source ? problem : sourceLabel(c) }));
  }
  const mute = c.mix.mic.muted;
  if (changed("youmic", [mute, lang])) {
    $("youmic").replaceChildren(icon(mute ? "mic-off" : "mic", "sm"));
    $("youmic").classList.toggle("warn", mute);
    $("youmic").title = mute ? t("Unmute microphone") : t("Mute microphone");
  }
}

function renderStart() {
  if (inRoom()) return;
  const pendingRoom = st.room && st.room.pending;
  if (!st.name) askName = true;
  $("namefield").hidden = !askName;
  if (document.activeElement !== $("startname")) $("startname").value = st.name;
  const named = !!st.name || !!$("startname").value.trim();
  $("startpending").hidden = !pendingRoom;
  $("startform").hidden = !!pendingRoom;
  $("pendingtext").textContent = pendingRoom ? (st.room.waiting ? t("Waiting for approval") : t("Connecting…")) : "";
  $("newroom").disabled = !named;
  $("join").disabled = !named || !$("joincode").value.trim();
  const others = st.rooms.filter((r) => !r.id.startsWith("pending-"));
  $("roomsbox").hidden = !others.length;
  if (!changed("rooms", [others, lang])) return;
  $("roomlist").replaceChildren(...others.map((r) => {
    const ids = r.members;
    const face = (id) => { const a = el("span", { className: "av sm", textContent: initial(id === st.id ? st.name : r.names[id]) }); a.style.background = colorOf(r.colors[id]); return a; };
    const more = iconButton("more", t("More"), (e) => { e.stopPropagation(); openMenu([{ label: t("Leave room…"), icon: "logout", danger: true, run: () => leaveRoom(r) }], e.currentTarget); }, "icon sm more");
    const row = el("div", { className: "rrow", tabIndex: 0, role: "button" }, el("b", { textContent: r.name }), el("span", { className: "avs" }, ...ids.slice(0, AVATARS_SHOWN).map(face)), el("span", { className: "sp" }), el("span", { className: "muted small", textContent: t(ids.length === 1 ? "{n} member" : "{n} members", { n: ids.length }) }), more);
    row.onclick = () => run("switch_room", { id: r.id }, () => { go("room"); });
    row.onkeydown = (e) => { if (e.key === "Enter") row.click(); };
    return row;
  }));
}

function playSet() { return (st && st.sets.find((s) => s.id === selected)) || null; }
function players() {
  const set = playSet();
  if (!set) return [];
  return set.players.map((p) => ({ key: p.owner, start: p.start_us, off: p.offset_us || 0, file: p, recording: p.recording }));
}
function base(list = players()) {
  return list.length ? Math.min(...list.map((p) => p.start - p.off)) : nowUs();
}
function received(list = players()) {
  const b = base(list);
  const ends = [];
  for (const p of list) {
    if (p.file.end_us) ends.push((p.file.end_us - p.off - b) / 1e6);
    const tl = play.tiles[p.key];
    if (tl && tl.media.part && isFinite(tl.media.video.duration)) ends.push(tl.media.video.duration - shiftOf(p, tl.media.part, b));
  }
  return ends.length ? Math.max(...ends) : 0;
}
function total(list = players()) {
  return Math.max(received(list), list.some((p) => p.recording) ? (nowUs() - base(list)) / 1e6 : 0);
}
const nowM = () => (clock.playing ? clock.m + ((performance.now() - clock.at) / 1000) * rate : clock.m);
const wallOf = (m) => Math.round(base() + m * 1e6);
function setClock(playing, m) {
  clock.playing = playing;
  clock.m = Math.max(0, m);
  clock.at = performance.now();
  tick();
}
const recv = (key) => ({ app: { volume: 1, muted: false }, voice: { volume: 1, muted: false }, ...(receive[key] || {}) });
function applyAudio() {
  const n = play.keys.length;
  const away = page !== "player";
  play.keys.forEach((key, i) => {
    const tl = play.tiles[key];
    if (!tl) return;
    const heard = play.view === i + 1 || (play.view === 0 && n === 1);
    const mine = recv(key);
    for (const [s, on, mix] of [[tl.media, heard, mine.app], [tl.mic, true, mine.voice]]) if (s) for (const v of [s.video, s.pending && s.pending.video]) if (v) {
      v.muted = away || muted || !on || mix.muted;
      v.volume = volume * mix.volume;
      v.playbackRate = rate;
    }
  });
  if (changed("mute", [muted || volume === 0])) $("mute").replaceChildren(icon(muted || volume === 0 ? "volume-x" : "volume-2"));
  if (document.activeElement !== $("volume")) $("volume").value = volume;
  rangeFill($("volume"));
  $("speed").value = rate;
}
function rangeFill(r) {
  r.style.setProperty("--v", ((Number(r.value) - Number(r.min)) / (Number(r.max) - Number(r.min))) * 100 + "%");
}
function openReceive(key, anchor) {
  const row = (kind, label) => {
    const mix = recv(key)[kind];
    const range = el("input", { type: "range", min: 0, max: 1, step: VOLUME_STEP, value: mix.volume, ariaLabel: label });
    let off = mix.muted;
    const mute = el("button", { className: "icon sm", ariaLabel: t("Mute") });
    const save = () => { receive[key] = { ...recv(key), [kind]: { volume: Number(range.value), muted: off } }; store("receive", receive); mute.replaceChildren(icon(off ? "volume-x" : "volume-2", "sm")); rangeFill(range); applyAudio(); };
    range.oninput = () => { off = false; save(); };
    mute.onclick = () => { off = !off; save(); };
    save();
    return el("div", { className: "poprow" }, el("span", { textContent: label }), range, mute);
  };
  showPop([el("div", { className: "head" }, avatar(key, "sm"), el("b", { textContent: nameOf(key) })), row("app", t("App sound")), row("voice", t("Voice"))], anchor, true);
}
function showPop(children, anchor, above) {
  const pop = $("pop");
  pop.replaceChildren(...children);
  pop.hidden = false;
  const r = anchor.getBoundingClientRect();
  const sideways = anchor.closest(".side");
  pop.style.left = Math.max(4, Math.min(sideways ? r.right + 8 : r.left, innerWidth - pop.offsetWidth - 4)) + "px";
  const top = sideways ? r.top : above ? r.top - pop.offsetHeight - 6 : r.bottom + 6;
  pop.style.top = Math.max(4, Math.min(top, innerHeight - pop.offsetHeight - 4)) + "px";
}

function memberPop(id, anchor) {
  const p = peerOf(id);
  if (!p) return;
  const m = memberState(p);
  const kv = [[t("Connection"), p.connected ? (p.path === "relay" ? t("Relay") : t("Direct")) : m.text], ...(p.connected ? [[t("Round trip"), `${p.rtt_ms} ms`]] : []), ...(p.version ? [[t("Version"), p.version]] : [])];
  const lines = checks[id];
  const remove = el("button", { className: "sm danger", textContent: t("Remove for everyone…"), onclick: () => { $("pop").hidden = true; removeMember(id); } });
  showPop([
    el("div", { className: "head" }, avatar(id, "lg"), el("b", { textContent: nameOf(id) }), p.connected ? el("span", { className: "chip" + (p.path === "relay" ? " warn" : "") }, icon(p.path === "relay" ? "relay" : "link", "sm"), p.path === "relay" ? t("Relay") : t("Direct")) : null),
    el("div", { className: "kv" }, ...kv.flatMap(([k, v]) => [el("span", { textContent: k }), el("span", { className: k === t("Connection") && p.path === "relay" ? "warn" : "", textContent: v })])),
    p.other_version ? el("div", { className: "warn small", textContent: t("{name} is on a different version. Both of you need the latest one.", { name: nameOf(id) }) }) : null,
    lines ? el("div", { className: "lines" }, ...lines.map((l) => el("div", { className: l.level }, icon({ ok: "ok", warn: "alert", fail: "fail" }[l.level] || "info", "sm"), el("span", { textContent: l.text })))) : null,
    el("div", { className: "row" }, remove),
  ].filter(Boolean), anchor);
}

function renderPlayer() {
  renderLive();
  const set = playSet();
  const idle = !set;
  const list = players();
  const shared = watchParty && set && partyLayout && partyLayout.set === set.id;
  const lay = set ? (shared ? { mode: partyLayout.mode || "grid", slots: partyLayout.slots || 0, order: partyLayout.order || [] } : playLayout(set)) : fromSaved(null);
  play.keys = ordered(list.map((p) => p.key), lay.order);
  play.mode = lay.mode;
  play.slots = lay.slots;
  if (shared && partyLayout.focus !== undefined) { play.view = partyLayout.focus ? play.keys.indexOf(partyLayout.focus) + 1 : 0; delete partyLayout.focus; }
  prune(play, play.keys);
  for (const p of list) {
    const tl = tile(play, p.key);
    nameTile(tl, p.key, p.recording);
    feedParts(tl, p, nowM(), base(list), clock.playing);
  }
  place(play);
  applyAudio();
  if (page !== "player") return;
  renderHeader(set);
  renderViews();
  $("playtiles").hidden = idle;
  $("transport").hidden = idle;
  $("views").hidden = $("layoutplay").hidden = $("toolsep").hidden = $("tools").hidden = idle;
  $("toolbar").hidden = idle;
  $("live").hidden = !set || !set.live;
  if (set && !set.live) live = false;
  press($("live"), live);
  press($("cc"), subtitles);
  $("cc").classList.toggle("on", subtitles);
  if (live && received() - nowM() > 2 * LIVE_LAG) setClock(true, received() - LIVE_LAG);
  const frac = set && set.progress.total ? Math.min(1, set.progress.have / set.progress.total) : 1;
  $("seek").querySelector(".got").style.width = frac * 100 + "%";
  $("seek").querySelector(".missing").hidden = !(set && set.progress.stuck);
  $("seek").querySelector(".missing").style.left = frac * 100 + "%";
}
function renderHeader(set) {
  const sig = [set && set.id, set && set.live, set ? set.players.map((p) => p.owner) : sending(), st.names, st.colors, lang, st.active_set && st.active_set.start_us];
  if (changed("header", sig)) {
    if (!set) { $("ptitle").textContent = t("Live"); $("psub").replaceChildren(avatars(sending())); }
    else if (set.live) { $("ptitle").textContent = t("Recording"); $("psub").replaceChildren(clockOf(set.start_us), " · ", avatars(set.players.map((p) => p.owner))); }
    else { $("ptitle").textContent = sessionName(set.id); $("psub").replaceChildren(avatars(set.players.map((p) => p.owner))); }
  }
  $("wp").hidden = !set;
  const party = watchParty && st.party.on;
  press($("wp"), party);
  $("wp").classList.toggle("on", party);
  const others = party && set && st.party.shared.set === set.id ? st.party.watching.filter((id) => id !== st.id) : [];
  $("shared").classList.toggle("wp", !!(party && set));
  $("wplabel").hidden = !others.length;
  if (changed("wplabel", [others, st.names, st.colors])) $("wplabel").replaceChildren(avatars(others), others.map(nameOf).join(", "));
  $("panelbtn").hidden = !set;
  $("panelbtn").classList.toggle("on", panelShown());
  if (changed("fullicon", [full])) $("full").replaceChildren(icon(full ? "minimize" : "maximize"));
}
const panelShown = () => (innerWidth > 1240 ? panel : panelOpen);
function renderViews() {
  const names = play.keys.map(nameOf);
  if (!changed("views", [play.view, names, lang])) return;
  const button = (i, text) => {
    const b = el("button", { className: play.view === i ? "on" : "", title: i <= MAX_KEYED ? `${text} (${i})` : text, ariaPressed: play.view === i, textContent: text });
    b.onclick = () => setView(play, i);
    return b;
  };
  $("views").replaceChildren(button(0, t("All")), ...names.map((n, i) => button(i + 1, n)));
}
function renderTools() {
  for (const b of $("tools").querySelectorAll("[data-tool]")) press(b, ink.tool === b.dataset.tool);
}

function sessionMenu(id, anchor) {
  const set = st.sets.find((s) => s.id === id);
  if (!set) return;
  const failed = jobInfo ? jobInfo.jobs.filter((j) => j.set === id && j.state === "failed") : [];
  openMenu([
    { heading: `${sessionName(id)} · ${t("{size} here", { size: bytes(set.progress.have) })}` },
    { label: t("Export video…"), icon: "film", disabled: set.live, run: () => exportSet(set) },
    { label: t("Open folder"), icon: "folder", run: () => run("open_set_folder", { room: set.room, set: set.id }, null) },
    { label: t("Copy link"), icon: "link", run: () => navigator.clipboard.writeText(momentLink(set.id, 0, null)).then(() => flash(t("Link copied"))).catch(fail) },
    ...(failed.length ? [{ label: t("Retry analysis"), icon: "rotate-cw", run: () => Promise.all(failed.map((j) => invoke("retry_job", { job: j.job }))).then(pollPlugins).catch(fail) }] : []),
    { separator: true },
    { label: t("Delete from this PC…"), icon: "trash", danger: true, disabled: set.live, run: () => deleteSession(set) },
  ], anchor);
}
async function deleteSession(set) {
  if (!set || set.live) return;
  if (await ask(t("Delete this session from this PC?"), t("Delete from this PC"), true)) run("delete_set", { room: set.room, set: set.id });
}
function openSession(id) {
  nowView = false;
  if (id !== selected) openSet(id);
  go("player");
}
function openNow() {
  nowView = true;
  const l = liveSet();
  if (l) {
    if (selected !== l.id) openSet(l.id);
    nowSet = l.id;
    live = true;
  }
  go("player");
}
function openSet(id, m = 0) {
  selectSet(id);
  if (m) setClock(false, m);
  share();
  refresh();
}
function selectSet(id) {
  if (id === selected) return;
  closeQuick(false);
  selected = id;
  clock.playing = false;
  clock.m = 0;
  live = false;
  delete sigs.events;
  delete sigs.views;
  ink.live.clear();
  ink.press = null;
  play.view = 0;
  prune(play, []);
  quiet("focus_set", { set: id });
}
function stepSet(dir) {
  const list = st.sets.filter((s) => !s.live);
  const i = list.findIndex((s) => s.id === selected);
  const next = i < 0 ? list[0] : list[i - dir];
  if (next) { openSession(next.id); osd(sessionName(next.id)); }
}

function tick() {
  if (!st) return;
  const feeds = [...Object.values(rec.tiles).map((tl) => tl.media), preview.slot].filter(Boolean);
  for (const s of feeds) follow(s);
  if (page === "capture") for (const kind of ["app", "mic"]) if (!meters[kind].v) drawMeter(kind, 0);
  const list = players();
  const tot = total(list);
  let m = nowM();
  if (clock.playing && tot && m >= tot) { clock.playing = false; clock.m = tot; m = tot; }
  const b = base(list);
  const away = page !== "player";
  for (const p of list) {
    const tl = play.tiles[p.key];
    if (!tl) continue;
    const wall = b + m * 1e6;
    if (pathOf(tl.media.part) !== pathOf(partFor(p, "v", wall)) || pathOf(tl.mic && tl.mic.part) !== pathOf(partFor(p, "m", wall))) feedParts(tl, p, m, b, clock.playing && !away);
    const v = tl.media.video;
    const target = tl.media.part ? m + shiftOf(p, tl.media.part, b) : 0;
    const have = tl.media.part && v.readyState > 0 && isFinite(v.duration) ? v.duration : 0;
    const short = !tl.media.part || (!!p.file.total && p.file.have < p.file.total && target > have);
    tl.box.classList.toggle("wait", short);
    if (short && p.file.total) {
      const n = Math.floor((p.file.have / p.file.total) * 100);
      if (changed("what/" + p.key, [n, p.file.stuck, lang])) tl.what.replaceChildren(icon(p.file.stuck ? "alert" : "download", "sm"), p.file.stuck ? t("{name} stopped at {n}%", { name: nameOf(p.key), n }) : t("{name} {n}% here", { name: nameOf(p.key), n }));
    }
    for (const s of [tl.media, tl.mic]) if (s && s.part && !switching(s)) sync(s.video, m + shiftOf(p, s.part, b), clock.playing && !away);
  }
  if (st.active_set) $("nowtitle").textContent = fmt((nowUs() - st.active_set.start_us) / 1e6);
  if (away) return;
  $("clocknow").textContent = fmt(m);
  $("clocktotal").textContent = ` / ${fmt(tot)}`;
  const frac = tot ? Math.min(1, m / tot) : 0;
  $("seek").querySelector(".fill").style.width = frac * 100 + "%";
  $("seek").querySelector(".thumb").style.left = frac * 100 + "%";
  $("seek").classList.toggle("paused", !clock.playing);
  $("seek").setAttribute("aria-valuemax", Math.round(tot));
  $("seek").setAttribute("aria-valuenow", Math.round(m));
  $("seek").setAttribute("aria-valuetext", fmt(m));
  if (changed("playicon", [clock.playing])) $("play").replaceChildren(icon(clock.playing ? "pause" : "play"));
  $("play").setAttribute("aria-label", clock.playing ? t("Pause") : t("Play"));
  renderMoment(m);
}

function osd(text, undoIt) {
  $("osdtext").textContent = text;
  if (undoIt) setUndo(undoIt);
  $("osdundo").hidden = !undoIt;
  $("osd").hidden = false;
  clearTimeout(osdTimer);
  osdTimer = setTimeout(() => { $("osd").hidden = true; }, OSD_MS);
}
function setUndo(fn) {
  undo = { fn, at: Date.now() };
}
function runUndo() {
  if (!undo || Date.now() - undo.at > UNDO_MS) { osd(t("Nothing to undo")); return; }
  const u = undo;
  undo = null;
  $("osd").hidden = true;
  u.fn();
}
function bezel(playing) {
  const b = $("bezel");
  b.replaceChildren(icon(playing ? "play" : "pause"));
  b.hidden = true;
  void b.offsetWidth;
  b.hidden = false;
}
function apply(cmd, m, who, prev = { playing: clock.playing, m: nowM() }) {
  live = false;
  if (cmd === "play") setClock(true, m);
  else if (cmd === "pause") setClock(false, m);
  else if (cmd === "seek") setClock(clock.playing, m);
  else return;
  if (cmd !== "seek") bezel(cmd === "play");
  const text = cmd === "play" ? t("{name} played", { name: who }) : cmd === "pause" ? t("{name} paused", { name: who }) : t("{name} moved to {time}", { name: who, time: fmt(m) });
  osd(text, () => act(prev.playing ? "play" : "pause", prev.m));
  renderInk();
}
function view() {
  const set = playSet();
  return { set: set ? set.id : null, playing: clock.playing, wall_us: wallOf(nowM()), at_us: nowUs(), rate, focus: play.view ? play.keys[play.view - 1] : null, mode: play.mode, slots: play.slots, order: play.keys };
}
function share() {
  if (watchParty && st && st.party.on) invoke("party_act", { view: view() }).catch(fail);
}
function joinParty() {
  partyLast = null;
  partyLayout = null;
  return invoke("party_on", { on: watchParty, view: view() }).catch(fail);
}
const newer = (a, b) => !b || (a.seq !== b.seq ? a.seq > b.seq : a.stamp !== b.stamp ? a.stamp > b.stamp : a.origin > b.origin);
function applyParty(p) {
  if (p.set && p.set !== selected && !st.sets.some((s) => s.id === p.set)) return;
  const first = !partyLast;
  partyLast = p;
  const mine = p.origin === st.id;
  if (quick && !mine) quick.moved = true;
  const who = nameOf(p.origin);
  if (p.set && p.set !== selected) {
    selectSet(p.set);
    if (nowView && !(liveSet() && liveSet().id === p.set)) nowView = false;
    if (!mine && !first) osd(t("{name} opened {set}", { name: who, set: sessionName(p.set) }));
  }
  partyLayout = { set: p.set, mode: p.mode, slots: p.slots, order: p.order, focus: p.focus };
  if (p.rate && p.rate !== rate) setRate(p.rate);
  const set = playSet();
  if (!set || !p.set || typeof p.wall_us !== "number") return;
  const m = Math.max(0, (p.wall_us - base()) / 1e6 + (p.playing ? ((nowUs() - p.at_us) / 1e6) * rate : 0));
  if (mine || first || p.seq === 0) { if (p.playing !== clock.playing || Math.abs(m - nowM()) > JUMP_MIN) setClock(p.playing, m); }
  else if (p.playing !== clock.playing) apply(p.playing ? "play" : "pause", m, who);
  else if (Math.abs(m - nowM()) > JUMP_MIN) apply("seek", m, who);
  else setClock(p.playing, m);
}
function act(cmd, m, prev) {
  if (!playSet()) return;
  apply(cmd, m, nameOf(st.id), prev);
  share();
}
window.__TAURI__.event.listen("party", () => refresh());
const togglePlay = () => act(clock.playing ? "pause" : "play", nowM());
const seekTo = (m) => act("seek", Math.max(0, m));
const jump = (delta) => seekTo(Math.min(total(), nowM() + delta));

function indexOf(state) {
  const byKey = new Map(state.events.map((e) => [e.key, e]));
  const setOfKey = new Map();
  for (const s of state.sets) for (const k of s.events) if (!setOfKey.has(k)) setOfKey.set(k, s);
  return { byKey, setOfKey, items: new Map(), all: null };
}
const setOf = (e) => idx.setOfKey.get(e.key) || null;
const setLength = (s) => fmt(((s.end_us || nowUs()) - s.start_us) / 1e6);
const pluginName = (id) => t((plugins.find((p) => p.id === id) || {}).name || id);
function itemOf(e, set) {
  const r = role(e);
  const at = (us) => (us === null || us === undefined || !set ? null : (us - set.start_us) / 1e6);
  return { ...e, role: r, setId: set ? set.id : null, m: at(e.local_us), end: at(e.end_us), who: e.plugin ? pluginName(e.plugin.id) : nameOf(e.author), body: e.kind === "mark" ? t("Marked") : eventText(e), speaker: e.file ? nameOf(e.file) : nameOf(e.author) };
}
function matches(n, words) {
  const text = `${n.body} ${n.who} ${n.kind}`.toLowerCase();
  return words.every((w) => (w.startsWith("#") ? (n.tags || []).some((tg) => "#" + tg.toLowerCase() === w) : text.includes(w)));
}
function setItems(set) {
  if (!set) return [];
  if (!idx.items.has(set.id)) {
    const keys = new Set(set.events);
    idx.items.set(set.id, st.events.filter((e) => keys.has(e.key)).map((e) => itemOf(e, set)));
  }
  return idx.items.get(set.id);
}
function allItems() {
  if (!idx.all) idx.all = st.events.filter((e) => idx.setOfKey.has(e.key)).map((e) => itemOf(e, setOf(e)));
  return idx.all;
}
const listed = (r) => r === "moment" || r === "chapter" || r === "drawing";
function chapters(tot) {
  const set = playSet();
  const matchList = matchChapters(matchEnds(items), tot, !!(set && set.live)).map((c) => ({ key: "match" + c.n, m: c.m, end: c.end, body: t("Match {n}", { n: c.n }), match: true }));
  return [...matchList, ...chaptersOf(items)].sort((a, b) => a.m - b.m);
}
function renderEvents() {
  items = setItems(playSet());
  const words = $("filter").value.toLowerCase().split(/\s+/).filter(Boolean);
  const pool = scope === "all" ? allItems() : items;
  const order = (x, y) => (x.setId === y.setId ? (x.m ?? -1) - (y.m ?? -1) : x.setId < y.setId ? 1 : -1);
  const shown = groupDrawings(pool.filter((n) => listed(n.role) && matches(n, words)).sort(order));
  const speech = items.filter((n) => n.role === "speech").sort((x, y) => x.m - y.m);
  renderPanelHead(shown.length, speech.length);
  renderTimeline(Math.round(total()));
  renderEventList(shown, words);
  renderConvo(speech);
}
function renderPanelHead(shown, speech) {
  $("sideconvo").hidden = !speech;
  if (!speech) side = "events";
  if (changed("sidehead", [shown, lang])) $("sideevents").replaceChildren(t("Events"), el("span", { textContent: shown }));
  $("sideevents").classList.toggle("on", side === "events");
  $("sideconvo").classList.toggle("on", side === "convo");
  $("notelist").hidden = side !== "events";
  $("convolist").hidden = side !== "convo";
  $("filterrow").hidden = side !== "events";
  $("scope").value = scope;
}
function renderTimeline(tot) {
  const list = chapters(tot);
  if (changed("chapters", [list.map((n) => [n.key, n.m, n.end, n.body]), tot])) renderChapters(list, tot);
  if (changed("lane", [items.filter((n) => n.role === "moment").map((n) => [n.key, n.m, n.body, n.author, n.kind]), tot, st.colors])) renderLane(tot);
}
function renderEventList(shown, words) {
  const set = playSet();
  if ($("notelist").querySelector("input") || !changed("events", [shown, lang, selected, st.colors, scope])) return;
  if (!shown.length) { $("notelist").replaceChildren(el("div", { className: "none", textContent: words.length ? t("Nothing found") : t("No events yet") })); return; }
  const out = [];
  let group = set ? set.id : null;
  for (const n of shown) {
    if (scope === "all" && n.setId !== group) { group = n.setId; out.push(el("div", { className: "grp", textContent: sessionName(n.setId) })); }
    out.push(eventRow(n));
  }
  $("notelist").replaceChildren(...out);
}
function renderConvo(speech) {
  if (!changed("convo", [speech.map((n) => [n.key, n.m, n.speaker, n.body]), lang, st.colors])) return;
  $("convolist").replaceChildren(...speech.map((n) => {
    const row = el("div", { className: "ev", tabIndex: 0 }, el("span", { className: "at", textContent: fmt(n.m) }), el("span", { className: "tx" }, el("span", { className: "who", style: `color:${colorOf(colorIndex(n.file || n.author))}`, textContent: n.speaker }), n.body));
    row.dataset.key = n.key;
    row.onclick = () => seekTo(n.m);
    return row;
  }));
}
function timeText(n) {
  if (n.m === null) return t("Session");
  return n.end !== null && n.role === "chapter" ? `${fmt(n.m)}–${fmt(n.end)}` : fmt(n.m);
}
function goTo(n) {
  if (n.m === null) return;
  if (n.setId !== selected) { openSession(n.setId); openSet(n.setId, n.m); }
  else seekTo(n.m);
}
function eventRow(n) {
  const at = el("button", { className: "at", textContent: timeText(n), disabled: !n.seekable || n.m === null });
  at.onclick = () => goTo(n);
  const sys = !!n.plugin || n.kind === "mark";
  const body = n.role === "drawing" ? [t("Drawing") + (n.count > 1 ? ` ×${n.count}` : "")] : inline(n.body);
  const tags = n.kind === "note" ? [] : (n.tags || []).map((tg) => el("button", { className: "tag", textContent: tagName(tg), onclick: (e) => { e.stopPropagation(); filterTag(tg); } }));
  const tx = el("span", { className: "tx" }, sys ? icon("flag", "sm") : null, ...body, ...tags);
  const dot = el("i", { className: "dt", style: `background:${n.plugin ? "var(--muted)" : colorOf(colorIndex(n.author))}` });
  const by = el("span", { className: "by" }, dot, n.who);
  const row = el("div", { className: "ev" + (sys ? " sys" : "") + (n.role === "chapter" ? " chapter" : "") }, at, tx, by);
  if (n.mine && n.kind !== "mark") {
    const acts = el("span", { className: "acts" });
    if (n.role !== "drawing") acts.append(iconButton("edit", t("Edit"), () => editNote(n, row)));
    if (n.role !== "drawing" || n.count === 1) acts.append(iconButton("trash", t("Delete"), () => deleteNote(n)));
    row.append(acts);
  }
  return row;
}
function deleteNote(n) {
  run("event_action", { kind: "delete", id: n.id, text: "" });
  const restore = () => run("event_action", { kind: "restore", id: n.id, text: "" });
  setUndo(restore);
  flash(t("Deleted"), "info", { label: t("Undo"), run: runUndo });
}
function renderChapters(list, tot) {
  $("chapters").hidden = !list.length;
  $("chapters").replaceChildren(...(tot ? list : []).map((n) => {
    const c = el("button", { className: "chap", title: `${timeText({ ...n, role: "chapter" })} ${n.body}`, textContent: n.body, tabIndex: -1 });
    c.style.left = `calc(${Math.max(0, (n.m / tot) * 100)}% + 1px)`;
    c.style.width = `calc(${Math.max(0, ((Math.min(n.end, tot) - n.m) / tot) * 100)}% - 2px)`;
    c.dataset.m = n.m;
    c.dataset.end = n.end;
    c.onclick = () => seekTo(n.m);
    return c;
  }));
}
function renderLane(tot) {
  $("lane").replaceChildren(...(tot ? items.filter((n) => n.role === "moment" && n.m !== null) : []).map((n) => {
    const mark = el("button", { className: "mark" + (n.plugin ? " plugin" : n.kind === "mark" ? " flag" : ""), title: `${fmt(n.m)} ${n.body}`, ariaLabel: `${fmt(n.m)} ${n.body}`, tabIndex: -1 });
    mark.style.left = Math.max(0, Math.min(100, (n.m / tot) * 100)) + "%";
    if (!n.plugin && n.kind !== "mark") mark.style.background = colorOf(colorIndex(n.author));
    mark.onclick = () => seekTo(n.m);
    return mark;
  }));
}
const chapterAt = (m) => chapters(total()).filter((n) => n.m <= m && m <= n.end).pop();
function renderMoment(m) {
  if (!playSet()) return;
  let match = null;
  for (const c of $("chapters").children) {
    const on = Number(c.dataset.m) <= m && m < Number(c.dataset.end);
    c.classList.toggle("cur", on);
    if (on) match = c.textContent;
  }
  $("matchchip").hidden = !match;
  if (match && $("matchchip").textContent !== match) $("matchchip").textContent = match;
  const notes = activeAt(items.filter((n) => n.role === "moment" && n.text && !n.plugin), m).slice(-OVERLAY_MAX);
  const typists = (watchParty && st.party.on ? st.party.typing || [] : []).filter((id) => id !== st.id);
  if (changed("overlay", [notes.map((n) => [n.key, n.m, n.body, n.author]), typists.map(nameOf), st.colors, lang])) $("overlay").replaceChildren(
    ...notes.map((n) => el("p", {}, avatar(n.author, "sm"), ...inline(n.body))),
    ...typists.map((id) => el("p", { className: "typing", textContent: t("{name} is typing…", { name: nameOf(id) }) })));
  const speech = subtitles ? activeAt(items.filter((n) => n.role === "speech"), m) : [];
  for (const key of play.keys) {
    const tl = play.tiles[key];
    if (!tl) continue;
    const lines = speech.filter((n) => (n.file || n.author) === key).map((n) => n.body);
    tl.sub.hidden = !lines.length;
    if (tl.sub.textContent !== lines.join("\n")) tl.sub.textContent = lines.join("\n");
  }
  const current = speech.length ? speech[speech.length - 1].key : null;
  for (const row of $("convolist").children) {
    const on = row.dataset.key === current;
    if (on && !row.classList.contains("cur") && clock.playing) row.scrollIntoView({ block: "nearest" });
    row.classList.toggle("cur", on);
  }
}
function filterTag(tag) {
  if (!playSet()) return;
  $("filter").value = "#" + tag;
  side = "events";
  showPanel(true);
  if (page !== "player") go("player");
  renderEvents();
}
function openLink(link) {
  if (!st.sets.some((s) => s.id === link.set)) { flash(t("That session is not on this PC.")); return; }
  openSession(link.set);
  openSet(link.set, link.m);
  if (link.view) setTimeout(() => setView(play, play.keys.indexOf(link.view) + 1), TICK_MS);
}
function editNote(n, row) {
  const input = el("input", { value: n.text, ariaLabel: t("Edit") });
  let done = false;
  const close = (save) => {
    if (done) return;
    done = true;
    if (save && input.value.trim() && input.value !== n.text) run("event_action", { kind: "edit", id: n.id, text: input.value });
    delete sigs.events;
    renderEvents();
  };
  input.onkeydown = (e) => {
    e.stopPropagation();
    if (composing(e)) return;
    if (e.key === "Enter") close(true);
    if (e.key === "Escape") { e.preventDefault(); close(false); }
  };
  input.onblur = () => close(true);
  row.replaceChildren(input);
  input.focus();
}
function stepTo(times, dir) {
  const next = stepTarget(times, nowM(), dir, NOTE_STEP_GAP);
  if (next !== undefined) seekTo(next);
  else osd(dir > 0 ? t("No next one") : t("No previous one"));
}
const stepNote = (dir) => stepTo(items.filter((n) => n.kind === "note" && n.role === "moment" && n.m !== null).map((n) => n.m), dir);
function stepMatch(dir) {
  const ends = matchEnds(items);
  if (ends.length) { stepTo(matchTargets(ends), dir); return; }
  stepTo(items.filter((n) => ((n.role === "moment" && n.kind !== "note") || n.role === "chapter") && n.m !== null).map((n) => n.m), dir);
}

function openQuick() {
  const set = playSet();
  if (!set) return;
  if (page !== "player") go("player");
  if (!quick) {
    quick = { resume: clock.playing, m: nowM(), wall: wallOf(nowM()), set: set.id, timed: true, moved: false };
    if (clock.playing) act("pause", nowM());
  }
  $("quick").hidden = false;
  renderQuick();
  $("quicktext").focus();
  quiet("party_typing", { on: true });
}
function renderQuick() {
  $("quicktime").textContent = quick.timed ? fmt(quick.m) : t("Whole session");
  const word = ($("quicktext").value.slice(0, $("quicktext").selectionStart).match(/#([\p{L}\p{N}_-]*)$/u) || [])[1];
  const known = [...new Set(st.events.flatMap((e) => e.tags || []))];
  const hints = word === undefined ? [] : known.filter((tg) => tg.toLowerCase().startsWith(word.toLowerCase()) && tg.toLowerCase() !== word.toLowerCase()).slice(0, TAG_HINTS);
  $("taghints").replaceChildren(...hints.map((tg, i) => el("button", { type: "button", className: "tag" + (i ? "" : " on"), textContent: "#" + tg, onmousedown: (e) => e.preventDefault(), onclick: () => pickTag(tg) })));
  $("taghints").hidden = !hints.length;
  return hints;
}
function pickTag(tag) {
  const box = $("quicktext");
  const at = box.selectionStart;
  const before = box.value.slice(0, at).replace(/#[\p{L}\p{N}_-]*$/u, "#" + tag + " ");
  box.value = before + box.value.slice(at);
  box.setSelectionRange(before.length, before.length);
  renderQuick();
}
function closeQuick(resume) {
  if (!quick) return;
  const q = quick;
  quick = null;
  $("quick").hidden = true;
  quiet("party_typing", { on: false });
  if (resume && q.resume && !q.moved) act("play", nowM());
}
function submitQuick() {
  const text = $("quicktext").value.trim();
  const q = quick;
  if (!text) { closeQuick(true); return; }
  $("quicktext").value = "";
  closeQuick(true);
  run("add_event", { kind: "note", text, target: q.timed ? { t_us: q.wall } : { set: q.set }, values: null }, () => { osd(q.timed ? t("Note added at {time}", { time: fmt(q.m) }) : t("Note added")); refresh(); });
}
$("quicktext").oninput = renderQuick;
$("quicktext").onkeydown = (e) => {
  e.stopPropagation();
  if (composing(e)) return;
  if (e.key === "Escape") { e.preventDefault(); $("quicktext").value = ""; closeQuick(true); }
  else if (e.key === "Tab") {
    e.preventDefault();
    const hints = renderQuick();
    if (hints.length) pickTag(hints[0]);
    else { quick.timed = !quick.timed; renderQuick(); }
  } else if (e.key === "Enter") { e.preventDefault(); submitQuick(); }
};
$("quicktime").onclick = () => { quick.timed = !quick.timed; renderQuick(); $("quicktext").focus(); };
$("quicktext").onblur = () => setTimeout(() => { if (quick && document.activeElement !== $("quicktext") && !$("quicktext").value.trim()) closeQuick(false); }, DOUBLE_CLICK_MS);

function layoutIcon(id, n) {
  const l = LAYOUTS.find((x) => x.id === id);
  const count = Math.min(l.max, Math.max(l.min, n));
  const r = rects(count, id, 20, 14, 1);
  return svg("svg", { viewBox: "0 0 20 14", class: "lic" }, ...r.tiles.map((b, i) => svg("rect", { x: b.x, y: b.y, width: b.w, height: b.h, rx: 1, class: i === 0 && id !== "grid" && id !== "row" ? "big" : "" })));
}
function renderLayoutButton() {
  const n = Math.max(1, play.keys.length, play.slots || 0);
  const mode = effectiveMode(play);
  if (!changed("layoutplay", [mode, n, lang])) return;
  $("layoutplay").replaceChildren(icon("grid"));
  $("layoutplay").title = t(LAYOUTS.find((l) => l.id === mode).name);
}
function pickLayout(s, anchor) {
  const present = Math.max(1, s.keys.length);
  const show = (count) => {
    const box = el("div", { className: "layoutpick" });
    const counts = el("div", { className: "seg full" }, ...[...Array(MAX_SLOTS).keys()].map((i) => {
      const n = i + 1;
      const b = el("button", { textContent: n, disabled: n < present, className: n === count ? "on" : "", title: t("{n} players", { n }) });
      b.onclick = () => show(n);
      return b;
    }));
    const kinds = el("div", { className: "kinds" }, ...LAYOUTS.filter((l) => layoutFits(l.id, count)).map((l) => {
      const b = el("button", { className: "kind" + (effectiveMode(s) === l.id && Math.max(present, s.slots || 0) === count ? " on" : "") }, layoutIcon(l.id, count), el("span", { className: "label", textContent: t(l.name) }));
      b.onclick = () => { closeMenu(); saveLayout(s, l.id, count > present ? count : 0); };
      return b;
    }));
    box.append(counts, kinds);
    openMenu([{ element: box }], anchor, true);
  };
  show(Math.max(present, s.slots || 0));
}

function openMenu(list, at, keepFocus) {
  const m = $("menu");
  if (!keepFocus || m.hidden) menuReturn = document.activeElement;
  m.replaceChildren(...list.map((it) => {
    if (it.separator) return el("hr");
    if (it.heading) return el("div", { className: "heading", textContent: it.heading });
    if (it.element) return it.element;
    const lead = it.check !== undefined ? el("span", { className: "ck" }, it.check ? icon("check", "sm") : null) : it.icon ? icon(it.icon, "sm") : null;
    const b = el("button", { role: "menuitem", disabled: !it.run || !!it.disabled, className: it.danger ? "danger" : "" }, lead, el("span", { className: "label", textContent: it.label }), it.right !== undefined ? el("span", { className: "right", textContent: it.right }) : null);
    b.onclick = () => { closeMenu(); it.run(); };
    return b;
  }));
  m.hidden = false;
  const r = at instanceof Event && at.clientX ? { left: at.clientX, bottom: at.clientY, top: at.clientY } : (at instanceof Event ? at.currentTarget || at.target : at).getBoundingClientRect();
  m.style.left = Math.max(4, Math.min(r.left, innerWidth - m.offsetWidth - 4)) + "px";
  const below = r.bottom + 4;
  m.style.top = (below + m.offsetHeight > innerHeight - 4 ? Math.max(4, r.top - m.offsetHeight - 4) : below) + "px";
  const first = m.querySelector("button:not(:disabled)");
  if (first && !keepFocus) first.focus();
}
function closeMenu() {
  if ($("menu").hidden) return;
  $("menu").hidden = true;
  $("roomsw").classList.remove("open");
  if (menuReturn && menuReturn.isConnected) menuReturn.focus();
}
document.addEventListener("mousedown", (e) => {
  if (!$("menu").contains(e.target)) closeMenu();
  if (!$("pop").contains(e.target)) $("pop").hidden = true;
});
function stageMenu(e) {
  const set = playSet();
  if (!set) return;
  openMenu([
    { label: t("Add note"), icon: "note", right: "C", run: openQuick },
    { label: t("Copy link to this moment"), icon: "link", run: () => navigator.clipboard.writeText(momentLink(set.id, nowM(), play.view ? play.keys[play.view - 1] : null)).then(() => osd(t("Link copied"))).catch(fail) },
    { label: t("Clear drawing"), icon: "eraser", right: "X", run: () => clearInk(false) },
  ], e);
}
function roomMenu() {
  $("roomsw").classList.add("open");
  openMenu([
    { heading: t("Rooms") },
    ...st.rooms.filter((r) => !r.id.startsWith("pending-")).map((r) => ({ label: r.name, check: r.active, right: String(r.members.length), run: r.active ? () => go("room") : () => run("switch_room", { id: r.id }, () => { go("room"); }) })),
    { separator: true },
    { label: t("Create room"), icon: "plus", run: createRoom },
    { label: t("Join room…"), icon: "link", run: openJoin },
  ], $("roomsw"));
}
function homeMenu(anchor) {
  openMenu([
    { label: t("Rename room…"), icon: "edit", run: openRename },
    { label: t("Copy invite code"), icon: "user-plus", run: copyInvite },
    { separator: true },
    { label: t("Leave room…"), icon: "logout", danger: true, run: () => leaveRoom(st.room) },
  ], anchor);
}

function flash(msg, kind = "info", action = null) {
  const text = typeof msg === "object" && msg && msg.code ? codeText(msg) : t(String(msg));
  $("toasttext").textContent = text;
  $("toast").classList.toggle("error", kind === "error");
  $("toastaction").hidden = !action;
  if (action) { $("toastaction").textContent = action.label; $("toastaction").onclick = () => { $("toast").hidden = true; action.run(); }; }
  $("toast").hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { $("toast").hidden = true; }, TOAST_MS);
}
function codeText(m) {
  if (m.code === "removed") return t("{by} removed {name}", m);
  if (m.code === "removed_you") return t("{by} removed you from the room", m);
  return m.code;
}
function notices() {
  const list = [];
  if (st.ffmpeg === "downloading") list.push({ key: "ffmpeg", level: "info", icon: "spinner", spin: true, text: t("Downloading video tools…"), sticky: true });
  if (st.ffmpeg === "failed") list.push({ key: "ffmpegfail", level: "error", icon: "fail", text: t("Couldn't download the video tools. Check your connection."), action: { label: t("Retry"), run: () => run("retry_ffmpeg", null) } });
  if (st.disk.low) list.push({ key: "disk", level: "warn", icon: "alert", text: t("Low disk space: {free} free", { free: bytes(st.disk.free) }) });
  if (st.notice) list.push({ key: "n:" + st.notice, level: "warn", icon: "alert", text: t(st.notice) });
  for (const p of st.peers.filter((x) => x.connected && x.other_version)) list.push({ key: "v:" + p.id, level: "warn", icon: "alert", text: t("{name} is on a different version. Both of you need the latest one.", { name: nameOf(p.id) }) });
  if (available && installAfter) list.push({ key: "after", level: "info", icon: "download", text: t("Version {version} installs after the session", { version: available.version }), action: { label: t("Cancel"), run: () => { installAfter = false; renderNotices(); } }, sticky: true });
  else if (available) list.push({ key: "update", level: "info", icon: "download", text: t("Version {version} is available", { version: available.version }), action: st.recording ? { label: t("Install after the session"), run: () => { installAfter = true; renderNotices(); } } : { label: t("Install"), primary: true, run: installUpdate } });
  if (updateText) list.push({ key: "updating", level: "info", icon: "spinner", spin: true, text: updateText, sticky: true });
  return list.filter((n) => !dismissed.has(n.key));
}
function renderNotices() {
  const list = notices();
  if (!changed("notices", [list.map((n) => [n.key, n.text, n.level, n.action && n.action.label]), lang])) return;
  $("notices").replaceChildren(...list.map((n) => el("div", { className: "notice " + n.level, role: n.level === "error" ? "alert" : "status" },
    icon(n.icon, n.spin ? "spin" : ""), el("span", { className: "text", textContent: n.text }),
    n.action ? el("button", { className: "sm" + (n.action.primary ? " primary" : ""), textContent: n.action.label, onclick: n.action.run }) : null,
    n.sticky ? null : iconButton("x", t("Close"), () => { dismissed.add(n.key); delete sigs.notices; renderNotices(); }))));
}
function exportSet(set) {
  if (!set) { flash(t("Open a session first")); return; }
  invoke("export_set", { room: set.room, set: set.id }).then((r) => {
    if (r) { exporting = { set: set.id, percent: 0 }; if (r.skipped.length) flash(t("Exporting without {names}: not on this PC yet", { names: r.skipped.map(nameOf).join(", ") })); refresh(); }
  }).catch(fail);
}
window.__TAURI__.event.listen("export", (e) => {
  const x = e.payload;
  if (x.done) { exporting = null; if (x.error) fail(x.error); else flash(t("Export finished")); refresh(); return; }
  if (exporting) exporting.percent = x.percent;
  renderSessions();
});
async function checkUpdate(manual) {
  if (updateText) return;
  checkedAt = Date.now();
  try {
    available = await window.__TAURI__.updater.check();
  } catch (e) {
    warn(e);
    if (manual) fail(t("Couldn't check for updates"));
    return;
  }
  if (!available && manual) flash(t("You have the latest version"));
  dismissed.delete("update");
  renderNotices();
}
function busy() {
  const party = watchParty && st.party.on && st.party.watching.some((id) => id !== st.id);
  return st.recording || party || clock.playing || !!exporting || !!(jobInfo && jobInfo.running);
}
async function askUpdate() {
  askingUpdate = true;
  const yes = await ask(t("Install version {version} now? The app restarts.", { version: available.version }), t("Install"));
  askingUpdate = false;
  installAfter = false;
  if (yes && available) installUpdate();
  else renderNotices();
}
async function installUpdate() {
  const update = available;
  available = null;
  installAfter = false;
  updateText = t("Installing {version}… The app will restart.", { version: update.version });
  renderNotices();
  await invoke("install_update").catch((e) => { warn(e); updateText = ""; available = update; fail(e); renderNotices(); });
}

function renderCapture() {
  if (page !== "capture") return;
  renderPreview();
  const c = st.capture;
  const value = c.source ? JSON.stringify(c.source) : "";
  if (document.activeElement !== $("source")) {
    if (value && ![...$("source").options].some((o) => o.value === value)) $("source").append(el("option", { value, textContent: sourceLabel(c) }));
    $("source").value = value;
  }
  if (changed("sourceicon", [c.source && c.source.kind])) $("sourceicon").replaceChildren(icon(c.source && c.source.kind === "display" ? "monitor" : "window"));
  const problem = captureProblem(c);
  const starting = !problem && !(c.live && c.live.size > 0);
  $("prevstate").hidden = !problem && !starting;
  $("prevtext").textContent = problem || (c.sharing || st.recording ? t("Starting…") : t("Not sent to the room"));
  $("usedisplay").hidden = !c.source_lost;
  $("badgerec").hidden = !(st.recording && c.working);
  $("badgelive").hidden = !c.sharing;
  if (document.activeElement !== $("mic")) $("mic").value = [...$("mic").options].some((o) => o.value === c.mic) ? c.mic : "";
  for (const kind of ["app", "mic"]) {
    if (document.activeElement !== $(kind + "vol")) $(kind + "vol").value = c.mix[kind].volume;
    rangeFill($(kind + "vol"));
    const off = c.mix[kind].muted;
    if (changed(kind + "mute", [off])) $(kind + "mute").replaceChildren(icon(kind === "mic" ? (off ? "mic-off" : "mic") : off ? "volume-x" : "volume-2", "sm"));
    $(kind + "mute").classList.toggle("warn", off);
    $(kind + "mute").setAttribute("aria-pressed", off);
    $(kind + "meter").classList.toggle("muted", off);
  }
  if (document.activeElement !== $("proc") && $("proc").options.length) $("proc").value = st.audio_process;
  const q = c.quality;
  for (const b of $("quality").children) press(b, b.dataset.q === q.id);
  $("qinfo").textContent = [q.width ? `${q.width} × ${q.height}` : "", `${q.fps} fps`, `${(q.kbps / 1000).toFixed(1)} Mbit/s`, String(q.codec).toUpperCase()].filter(Boolean).join(" · ");
  if (document.activeElement !== $("share")) $("share").checked = st.share;
  if (document.activeElement !== $("limit")) $("limit").value = st.limit_mbps || "";
  renderChips();
}
function renderChips() {
  const c = st.capture;
  const now = performance.now();
  const screen = !c.source ? "info" : c.source_lost || c.status ? "warn" : c.live && c.live.size > 0 ? "ok" : "info";
  const app = c.mix.app.muted ? "info" : meters.app.heard && now - meters.app.heard < HEARD_MS ? "ok" : "info";
  const mic = c.mix.mic.muted ? "info" : meters.mic.heard && now - meters.mic.heard < HEARD_MS ? "ok" : "info";
  const enc = c.status ? "warn" : c.working ? "ok" : "info";
  const list = [[screen, t("Screen")], [app, t("App sound")], [mic, t("Microphone")], [enc, t("Encoder {codec}", { codec: String(c.quality.codec).toUpperCase() })]];
  if (!changed("chips", [list])) return;
  $("chips").replaceChildren(...list.map(([level, text]) => el("span", { className: "chip " + level }, icon({ ok: "ok", warn: "alert", info: "info" }[level], "sm"), text)));
}
async function fillSources() {
  const list = await invoke("list_sources");
  if (!changed("sources", [list, lang])) return;
  const option = (s) => el("option", { value: JSON.stringify(s.id), textContent: s.id.kind === "display" ? t("Display {n}", { n: s.id.index }) + (s.width ? ` · ${s.width} × ${s.height}` : "") : s.id.title });
  const wins = list.filter((s) => s.id.kind === "window");
  const disps = list.filter((s) => s.id.kind === "display");
  $("source").replaceChildren(el("option", { value: "", textContent: t("Choose what to capture"), disabled: true, hidden: true }), el("optgroup", { label: t("Windows") }, ...wins.map(option)), el("optgroup", { label: t("Displays") }, ...disps.map(option)));
  if (st) { delete sigs.sourceicon; renderCapture(); }
}
async function fillMics() {
  const list = await invoke("list_mics");
  if (!changed("mics", [list, lang])) return;
  $("mic").replaceChildren(el("option", { value: "", textContent: t("Default microphone") }), ...list.map((m) => el("option", { value: m.id, textContent: m.name })));
  if (st) renderCapture();
}
async function fillProcesses() {
  const names = await invoke("list_processes");
  const current = st ? st.audio_process : $("proc").value;
  $("proc").replaceChildren(el("option", { value: "", textContent: t("App sound: automatic") }), ...names.map((n) => el("option", { value: n, textContent: n })));
  if (current && !names.includes(current)) $("proc").append(el("option", { value: current, textContent: current }));
  $("proc").value = current;
}
const setMix = (kind, mute) => run("set_mix", { kind, volume: Number($(kind + "vol").value), muted: mute });
for (const kind of ["app", "mic"]) {
  $(kind + "vol").oninput = () => { rangeFill($(kind + "vol")); setMix(kind, st.capture.mix[kind].muted); };
  $(kind + "mute").onclick = () => setMix(kind, !st.capture.mix[kind].muted);
  $(kind + "scale").replaceChildren(...METER_TICKS.map((db) => el("span", { textContent: String(db).replace("-", "−"), style: `left:${meterPos(db) * 100}%` })));
  const box = $(kind + "meter");
  box.style.setProperty("--yellow", meterPos(METER_YELLOW_DB) * 100 + "%");
  box.style.setProperty("--red", meterPos(METER_RED_DB) * 100 + "%");
  new ResizeObserver(() => box.style.setProperty("--full", box.clientWidth + "px")).observe(box);
}
function meterPos(db) {
  return Math.max(0, Math.min(1, (db - METER_MIN_DB) / -METER_MIN_DB));
}
const toDb = (v) => (v > 0 ? 20 * Math.log10(v) : -Infinity);
$("source").onfocus = () => fillSources().catch(fail);
$("source").onmousedown = () => fillSources().catch(fail);
$("source").onchange = () => run("set_source", { source: JSON.parse($("source").value) });
$("mic").onfocus = () => fillMics().catch(fail);
$("mic").onchange = () => run("set_mic", { id: $("mic").value });
$("proc").onfocus = () => fillProcesses().catch(fail);
$("proc").onchange = () => run("set_audio_process", { name: $("proc").value });
for (const b of $("quality").children) b.onclick = () => run("set_quality", { id: b.dataset.q });
$("usedisplay").onclick = () => run("use_display", null, () => fillSources().then(refresh));
$("share").onchange = () => run("set_share", { on: $("share").checked });
$("limit").onchange = () => run("set_limit", { mbps: $("limit").value ? Number($("limit").value) : null });
function drawMeter(kind, v) {
  const now = performance.now();
  const m = meters[kind];
  m.v = v;
  if (v > 0.001) m.heard = now;
  if (v >= m.peak || now - m.at > PEAK_HOLD_MS) { m.peak = v; m.at = now; }
  if (v >= 1) m.clip = now;
  if (page !== "capture") return;
  const box = $(kind + "meter");
  box.style.setProperty("--v", meterPos(toDb(v)) * 100 + "%");
  box.style.setProperty("--p", meterPos(toDb(m.peak)) * 100 + "%");
  box.classList.toggle("clip", now - m.clip < CLIP_HOLD_MS);
  box.classList.toggle("quiet", !m.peak);
  const db = toDb(m.peak);
  $(kind + "db").textContent = db > METER_MIN_DB ? `${String(Math.round(db)).replace("-", "−")} dB` : "";
}
window.__TAURI__.event.listen("levels", (e) => { for (const kind of ["app", "mic"]) drawMeter(kind, Math.max(0, e.payload[kind] || 0)); });

function refresh() {
  if (refreshing) { refreshAgain = true; return refreshing; }
  refreshing = (async () => {
    try { do { refreshAgain = false; await update(); } while (refreshAgain); }
    finally { refreshing = null; }
  })();
  return refreshing;
}
const STEPS = [renderShell, renderStart, renderSide, renderNotices, renderCapture, renderPlayer, renderEvents, renderTools, renderRoom, renderLayoutButton, renderRecovery];
async function update() {
  st = await invoke("state");
  idx = indexOf(st);
  if (lang !== st.lang) { lang = st.lang; setLanguage(lang); for (const k of Object.keys(sigs)) delete sigs[k]; fillSources().catch(warn); fillMics().catch(warn); fillProcesses().catch(warn); }
  if (!ready) { ready = true; invoke("ui_ready"); pollPlugins(); }
  if (installAfter && available && !busy() && !askingUpdate) askUpdate();
  if (selected && !st.sets.some((s) => s.id === selected)) selected = null;
  const l = liveSet();
  if (nowView && l && selected !== l.id) { selectSet(l.id); nowSet = l.id; live = true; share(); }
  if (nowView && !l && nowSet && selected === nowSet) { nowView = false; nowSet = null; }
  const roomId = st.room ? st.room.id : "";
  if (partyRoom !== roomId) { partyRoom = roomId; await joinParty(); st = await invoke("state"); idx = indexOf(st); }
  if (!drag && watchParty && st.party.on && newer(st.party.shared, partyLast)) applyParty(st.party.shared);
  for (const step of STEPS) try { step(); } catch (e) { console.error(e); }
}
function renderRecovery() {
  $("recovery").hidden = !st.recovery;
  $("recoverymsg").textContent = st.recovery ? t(st.recovery) : "";
  $("rinfo").textContent = st.previous_version ? t("Version {version}, previous run {previous}", { version: st.version, previous: st.previous_version }) : t("Version {version}", { version: st.version });
}

function toggleFull() {
  if (page !== "player" && !full) go("player");
  full = !full;
  $("playerpage").classList.toggle("full", full);
  renderShell();
  delete sigs.fullicon;
  window.__TAURI__.window.getCurrentWindow().setFullscreen(full).catch(warn);
  wake();
}
function wake() {
  $("playerpage").classList.remove("idle");
  clearTimeout(idleTimer);
  if (full) idleTimer = setTimeout(() => { if (clock.playing) $("playerpage").classList.add("idle"); }, IDLE_MS);
}
document.addEventListener("mousemove", wake);
function go(p) {
  if (p !== page) closeQuick(false);
  if (p !== "player" && full) toggleFull();
  page = p;
  if (p !== "player") nowView = false;
  closeMenu();
  if (st) { delete sigs.sessions; renderShell(); renderSide(); place(rec); place(play); }
  refresh();
}
function goPage(name) {
  if (name === "room") go("room");
  else if (name === "now") openNow();
  else go("capture");
}
function showPanel(on) {
  if (innerWidth > 1240) { panel = on; store("panel", panel); } else panelOpen = on;
  renderShell();
  delete sigs.header;
  if (st) renderHeader(playSet());
  setTimeout(() => place(play), 0);
}
const togglePanel = () => showPanel(!panelShown());
function toggleSidebar() {
  sidebar = !sidebar;
  store("sidebar", sidebar);
  renderShell();
}

function ask(title, action, danger) {
  const d = $("confirm");
  $("confirmtitle").textContent = title;
  $("confirmyes").textContent = action;
  $("confirmyes").className = danger ? "dangerfill" : "primary";
  d.showModal();
  $("confirmno").focus();
  return new Promise((done) => {
    $("confirmyes").onclick = () => { d.close(); done(true); };
    $("confirmno").onclick = () => { d.close(); done(false); };
    d.onclose = () => done(false);
  });
}
const copyInvite = () => invoke("create_invite", { reuse: true }).then((code) => navigator.clipboard.writeText(code)).then(() => { flash(t("Invite code copied")); refresh(); }).catch(fail);
const cancelInvite = () => run("cancel_invites", null, () => { flash(t("Invite cancelled")); refresh(); });
const createRoom = () => invoke("create_room", { name: "" }).then(() => { go("room"); return refresh().then(copyInvite); }).catch(fail);
async function leaveRoom(r) {
  if (!r) return;
  if (await ask(t("Leave {room} on this PC?", { room: r.name }), t("Leave"), true)) run("leave_room", { id: r.id }, () => { go("room"); });
}
async function removeMember(id) {
  if (await ask(t("Remove {name} from the room for everyone?", { name: nameOf(id) }), t("Remove for everyone"), true)) run("remove_peer", { id });
}
function joinWith(input) {
  if (joining) return;
  joining = true;
  run("join_room", { code: input.value }, () => { input.value = ""; if ($("joindlg").open) $("joindlg").close(); refresh(); }).finally(() => { joining = false; });
}
function openJoin() {
  if (!inRoom()) { $("joincode").focus(); return; }
  $("joindlg").showModal();
  $("joincode2").focus();
}
function openRename() {
  if (!st.room) return;
  $("renameinput").value = st.room.name;
  $("renamedlg").showModal();
  $("renameinput").select();
}
async function checkRoom() {
  if (checking || !inRoom()) return;
  checking = true;
  $("checkbtn").classList.add("spin");
  try {
    const results = await invoke("check_room");
    checks = Object.fromEntries(results.map((r) => [r.id, r.lines || [{ level: "fail", text: t("No answer") }]]));
    const bad = results.filter((r) => r.id !== st.id && worst(checks[r.id]) === "fail").length;
    flash(bad ? t("{n} with problems", { n: bad }) : t("All connections checked"));
  } catch (e) { fail(e); }
  checking = false;
  $("checkbtn").classList.remove("spin");
  delete sigs.members;
  refresh();
}
function boardCards() {
  const at = (e) => e.id - ((peerOf(e.author) || {}).offset_us || 0);
  const out = {};
  for (const e of st.events) if (e.kind === "board" && e.values && (!out[e.values.card] || at(e) >= at(out[e.values.card]))) out[e.values.card] = e;
  return out;
}
function renderRoom() {
  if (page !== "room" || !inRoom()) return;
  const all = people();
  const online = 1 + st.peers.filter((p) => p.connected).length;
  $("homename").textContent = st.room.name;
  $("homesub").textContent = t("{n} members · {online} online", { n: all.length, online });
  if (!$("boardcards").querySelector("textarea") && changed("board", [st.events.filter((e) => e.kind === "board").map((e) => e.key), all, lang, st.names, st.colors, st.name, jobInfo && jobInfo.tags])) renderBoard();
  renderPlugins();
}
function renderBoard() {
  const cards = [{ card: "team", title: t("Team") }, ...people().map((id) => ({ card: id, title: nameOf(id), color: colorIndex(id) }))];
  const newest = boardCards();
  $("boardcards").replaceChildren(...cards.map((c) => {
    const e = newest[c.card] || null;
    const body = el("div", { className: "cardbody" }, ...(e && e.text ? markdown(e.text) : [el("div", { className: "empty", textContent: "—" })]));
    const edit = iconButton("edit", t("Edit"), null);
    const box = el("article", { className: "card" + (c.card === "team" ? " team" : "") }, el("h3", {}, c.color !== undefined ? el("i", { className: "swatch", style: `background:${colorOf(c.color)}` }) : null, c.title, edit), body);
    edit.onclick = () => {
      const area = el("textarea", { value: e ? e.text : "", rows: 8 });
      const save = el("button", { className: "primary sm", textContent: t("Save") });
      const cancel = el("button", { className: "sm", textContent: t("Cancel") });
      const done = () => { delete sigs.board; renderBoard(); };
      save.onclick = () => run("add_event", { kind: "board", text: area.value, target: {}, values: { card: c.card } }, () => refresh().then(done));
      cancel.onclick = done;
      area.onkeydown = (k) => {
        k.stopPropagation();
        if (k.key === "Enter" && k.ctrlKey) save.click();
        if (k.key === "Escape") cancel.click();
      };
      edit.hidden = true;
      body.replaceChildren(area, el("div", { className: "row end" }, cancel, save));
      area.focus();
    };
    return box;
  }));
}
function pollPlugins() {
  if (!st || !inRoom() || document.hidden) return;
  const gen = ++pluginsGen;
  Promise.all([invoke("room_plugins"), invoke("jobs")]).then(([list, info]) => {
    if (gen !== pluginsGen) return;
    plugins = list;
    if (changed("tags", info.tags)) for (const k of ["events", "board"]) delete sigs[k];
    jobInfo = info;
    renderPlugins();
    renderSessions();
  }).catch(warn);
}
const capital = (c) => c.charAt(0).toUpperCase() + c.slice(1);
function choiceText(key, c) {
  if (key === "language" && c !== "auto") return capital(new Intl.DisplayNames([language], { type: "language" }).of(c) || c);
  return key === "model" ? c : t(capital(c));
}
function renderPlugins() {
  if (page !== "room" || !changed("plugins", [plugins, downloading, lang])) return;
  $("pluginlist").replaceChildren(...plugins.map((p) => {
    const settings = { ...p.settings };
    const save = (on) => invoke("set_room_plugin", { id: p.id, on, settings }).then(pollPlugins).catch(fail);
    const on = el("input", { type: "checkbox", className: "switch", role: "switch", checked: p.on, ariaLabel: t(p.name), onchange: (e) => save(e.target.checked) });
    const options = p.options.map((o) => el("select", { ariaLabel: t(capital(o.key)), onchange: (e) => { settings[o.key] = e.target.value; save(p.on); } }, ...o.choices.map((c) => el("option", { value: c, textContent: `${t(capital(o.key))}: ${choiceText(o.key, c)}`, selected: settings[o.key] === c }))));
    const get = () => {
      downloading[p.id] = true;
      renderPlugins();
      invoke("prepare_plugin", { id: p.id }).catch(fail).finally(() => { delete downloading[p.id]; pollPlugins(); });
    };
    const here = downloading[p.id] ? [icon("spinner", "sm spin"), t("Downloading…")]
      : p.ready ? [icon("ok", "sm ok"), t("Ready on this PC")]
      : [icon("download", "sm"), t("Not on this PC"), el("span", { className: "sp" }), el("button", { className: "sm", textContent: t("Download {size}", { size: bytes(p.download_bytes) }), onclick: get })];
    return el("div", { className: "plug" }, el("b", { textContent: t(p.name) }), on, options.length ? el("div", { className: "opts" }, ...options) : null, el("div", { className: "here" }, ...here));
  }));
}
window.__TAURI__.event.listen("job", (e) => {
  if (jobInfo) jobInfo.running = e.payload;
  renderSessions();
});
async function openStorage() {
  $("keepdays").value = st.keep_days || "";
  $("storagedlg").showModal();
  const info = await invoke("data_info").catch((e) => { fail(e); return null; });
  $("storagehead").textContent = info ? t("{used} used · {free} free", { used: bytes(info.used_bytes), free: bytes(info.free_bytes) }) : "";
  $("storagelist").replaceChildren(...st.sets.map((s) => {
    const more = iconButton("more", t("More"), (e) => sessionMenu(s.id, e.currentTarget), "icon sm more");
    const row = el("div", { className: "strow" }, el("span", { textContent: sessionName(s.id) }), el("span", { className: "muted", textContent: setLength(s) }), el("span", { className: "size", textContent: bytes(s.progress.have) }), more);
    row.oncontextmenu = (e) => { e.preventDefault(); sessionMenu(s.id, e); };
    return row;
  }));
}
function openSettings() {
  if (st) {
    $("name").value = st.name;
    $("language").value = st.language;
    $("jobshere").checked = st.process_jobs;
    $("hotkey").textContent = st.hotkey;
    $("hotkeyerr").hidden = true;
  }
  $("settings").showModal();
}
function toggleDialog(d) { if (d.open) d.close(); else d.showModal(); }
for (const d of document.querySelectorAll("dialog")) {
  d.addEventListener("click", (e) => {
    const r = d.getBoundingClientRect();
    const outside = e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom;
    if ((e.target === d && outside) || e.target.closest(".close")) d.close();
  });
}
function setRate(r) {
  clock.m = nowM();
  clock.at = performance.now();
  rate = r;
  applyAudio();
}
const changeRate = (dir) => { setRate(SPEEDS[Math.max(0, Math.min(SPEEDS.length - 1, SPEEDS.indexOf(rate) + dir))]); osd(t("Speed {rate}×", { rate })); share(); };
const toggleMute = () => { muted = !muted; applyAudio(); };
function setVolume(v) {
  volume = Math.max(0, Math.min(1, v));
  muted = false;
  store("volume", volume);
  applyAudio();
}
function cycle(s, delta) {
  const n = s.keys.length;
  if (n) setView(s, s.view === 0 ? (delta > 0 ? 1 : n) : ((s.view - 1 + delta + n) % n) + 1);
}
const toggleRecording = () => run(st && st.recording ? "stop_recording" : "start_recording");
const current = () => (playSet() ? play : rec);

$("speed").replaceChildren(...SPEEDS.map((s) => el("option", { value: s, textContent: `${s}×` })));
$("rec").onclick = (e) => { e.stopPropagation(); toggleRecording(); };
$("now").onclick = openNow;
$("now").onkeydown = (e) => { if (e.key === "Enter" && e.target === $("now")) openNow(); };
$("navroom").onclick = () => go("room");
$("roomsw").onclick = roomMenu;
$("you").onclick = () => go("capture");
$("you").onkeydown = (e) => { if (e.key === "Enter" && e.target === $("you")) go("capture"); };
$("youmic").onclick = (e) => { e.stopPropagation(); run("set_mix", { kind: "mic", volume: st.capture.mix.mic.volume, muted: !st.capture.mix.mic.muted }); };
$("checkbtn").onclick = checkRoom;
$("invitebtn").onclick = copyInvite;
$("homemore").onclick = (e) => homeMenu(e.currentTarget);
$("play").onclick = togglePlay;
$("back").onclick = () => jump(-STEP.normal);
$("fwd").onclick = () => jump(STEP.normal);
$("addnote").onclick = openQuick;
$("wp").onclick = () => { watchParty = !watchParty; store("watchParty", watchParty); joinParty().then(refresh); renderHeader(playSet()); };
$("osdundo").onclick = runUndo;
$("live").onclick = () => { live = !live; if (live) setClock(true, Math.max(0, received() - LIVE_LAG)); renderPlayer(); };
$("speed").onchange = (e) => { setRate(Number(e.target.value)); share(); };
$("volume").oninput = () => setVolume(Number($("volume").value));
$("mute").onclick = toggleMute;
$("cc").onclick = () => { subtitles = !subtitles; store("subtitles", subtitles); renderPlayer(); };
$("panelbtn").onclick = togglePanel;
$("full").onclick = toggleFull;
$("layoutplay").onclick = (e) => pickLayout(play, e.currentTarget);
$("sideevents").onclick = () => { side = "events"; renderEvents(); };
$("sideconvo").onclick = () => { side = "convo"; renderEvents(); };
$("scope").onchange = () => { scope = $("scope").value; renderEvents(); };
for (const b of $("tools").querySelectorAll("[data-tool]")) b.onclick = () => { ink.tool = b.dataset.tool; renderTools(); };
$("clearink").onclick = () => clearInk(false);
$("toast").onmouseenter = () => clearTimeout(toastTimer);
$("toast").onmouseleave = () => { toastTimer = setTimeout(() => { $("toast").hidden = true; }, TOAST_MS); };
function seekFrac(x) {
  const r = $("seek").getBoundingClientRect();
  return Math.max(0, Math.min(1, (x - r.left) / r.width));
}
$("seek").onpointerdown = (e) => {
  const bar = $("seek");
  if (!playSet() || drag) return;
  drag = { playing: clock.playing, m: nowM() };
  bar.setPointerCapture(e.pointerId);
  const move = (x) => { live = false; setClock(clock.playing, seekFrac(x) * total()); };
  move(e.clientX);
  bar.onpointermove = (m) => move(m.clientX);
  bar.onpointerup = bar.onpointercancel = () => {
    bar.onpointermove = bar.onpointerup = bar.onpointercancel = null;
    const from = drag;
    drag = null;
    act("seek", nowM(), from);
  };
};
const peek = { video: el("video", { muted: true, preload: "auto" }), want: null };
peek.video.onseeked = () => {
  const c = $("peek").querySelector("canvas");
  c.getContext("2d").drawImage(peek.video, 0, 0, c.width, c.height);
  if (peek.want !== null && Math.abs(peek.want - peek.video.currentTime) > SEEK_TOLERANCE) peek.video.currentTime = peek.want;
};
function peekAt(x) {
  const r = $("seek").getBoundingClientRect();
  const tot = total();
  if (!tot) return;
  const m = seekFrac(x) * tot;
  const list = players();
  const p = list.find((q) => q.key === (play.view ? play.keys[play.view - 1] : play.keys[0]));
  $("peek").hidden = false;
  $("peek").style.left = Math.max(0, Math.min(r.width, x - r.left)) + "px";
  $("peektime").textContent = fmt(m);
  const chapter = chapterAt(m);
  $("peekchapter").textContent = chapter ? chapter.body : "";
  if (!p) return;
  const b = base(list);
  const part = partFor(p, "v", b + m * 1e6);
  if (!part) return;
  const src = convertFileSrc(part.path, "ezh");
  if (peek.src !== src) { peek.src = src; peek.video.src = src; }
  peek.want = m + shiftOf(p, part, b);
  if (!peek.video.seeking && peek.video.readyState > 0) peek.video.currentTime = peek.want;
  else if (peek.video.readyState === 0) peek.video.onloadedmetadata = () => { peek.video.currentTime = peek.want; };
}
$("seek").onmousemove = (e) => peekAt(e.clientX);
$("seek").onmouseleave = () => { $("peek").hidden = true; };
$("startname").oninput = () => renderStart();
$("startname").onchange = () => run("set_name", { name: $("startname").value });
$("newroom").onclick = () => run("set_name", { name: $("startname").value || st.name }, createRoom);
$("joincode").oninput = () => renderStart();
$("join").onclick = () => run("set_name", { name: $("startname").value || st.name }, () => joinWith($("joincode")));
$("joincode").onkeydown = (e) => { if (e.key === "Enter" && !composing(e) && !$("join").disabled) $("join").click(); };
$("join2").onclick = () => joinWith($("joincode2"));
$("joincode2").onkeydown = (e) => { if (e.key === "Enter" && !composing(e)) joinWith($("joincode2")); };
$("pendingcancel").onclick = () => st.room && run("leave_room", { id: st.room.id });
$("renamesave").onclick = () => run("rename_room", { name: $("renameinput").value }, () => { $("renamedlg").close(); refresh(); });
$("renameinput").onkeydown = (e) => { if (e.key === "Enter" && !composing(e)) $("renamesave").click(); };
$("keepdays").onchange = () => run("set_keep_days", { days: $("keepdays").value ? Number($("keepdays").value) : null });
$("name").onchange = () => run("set_name", { name: $("name").value });
$("language").onchange = () => run("set_language", { language: $("language").value });
$("jobshere").onchange = () => run("set_process_jobs", { on: $("jobshere").checked });
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
$("rreset").onclick = async () => { if (await ask(t("Reset settings and cache?"), t("Reset"), true)) run("reset", { everything: false, confirm: "" }, null); };
$("rcontinue").onclick = () => run("dismiss_recovery");

const menuActions = {
  "settings": openSettings,
  "storage": openStorage,
  "create-room": createRoom,
  "join-room": openJoin,
  "copy-invite": copyInvite,
  "cancel-invite": cancelInvite,
  "rename-room": openRename,
  "check-room": checkRoom,
  "leave-room": () => leaveRoom(st.room),
  "add-note": openQuick,
  "prev-session": () => stepSet(-1),
  "next-session": () => stepSet(1),
  "export-session": () => exportSet(playSet()),
  "delete-session": () => deleteSession(playSet()),
  "page-room": () => goPage("room"),
  "page-now": () => goPage("now"),
  "page-capture": () => goPage("capture"),
  "sidebar": toggleSidebar,
  "events-panel": togglePanel,
  "fullscreen": toggleFull,
  "all-players": () => setView(current(), 0),
  "next-player": () => cycle(current(), 1),
  "prev-player": () => cycle(current(), -1),
  "shortcuts": () => { $("hotkeyshown").textContent = st ? st.hotkey : ""; toggleDialog($("shortcuts")); },
  "update": () => checkUpdate(true),
};
window.__TAURI__.event.listen("menu", (e) => { if (menuActions[e.payload]) menuActions[e.payload](); });
window.__TAURI__.event.listen("joined", (e) => flash(t("{name} joined", { name: e.payload })));
window.__TAURI__.event.listen("flash", (e) => flash(e.payload));
$("filter").oninput = () => renderEvents();
addEventListener("resize", () => renderShell());

const once = (fn) => (e) => { if (!e.repeat) fn(); };
const PLAYER_KEYS = {
  " ": once(togglePlay),
  k: once(togglePlay),
  ArrowLeft: (e) => jump(-(e.shiftKey ? STEP.small : STEP.normal)),
  ArrowRight: (e) => jump(e.shiftKey ? STEP.small : STEP.normal),
  j: (e) => jump(-(e.shiftKey ? STEP.huge : STEP.big)),
  l: (e) => jump(e.shiftKey ? STEP.huge : STEP.big),
  "<": () => changeRate(-1),
  ",": () => changeRate(-1),
  ">": () => changeRate(1),
  ".": () => changeRate(1),
  m: toggleMute,
  ArrowUp: () => setVolume(volume + VOLUME_STEP),
  ArrowDown: () => setVolume(volume - VOLUME_STEP),
  c: openQuick,
  q: () => stepNote(-1),
  e: () => stepNote(1),
  a: () => stepMatch(-1),
  d: () => stepMatch(1),
  x: () => clearInk(false),
  "Shift+p": () => stepSet(-1),
  "Shift+n": () => stepSet(1),
};
function stageKey(e, key) {
  const s = current();
  if (/^[0-9]$/.test(key) && Number(key) <= Math.min(MAX_KEYED, s.keys.length)) return () => setView(s, Number(key));
  if (key === "Tab" && !e.shiftKey && document.activeElement === document.body) return () => setView(s, s.view ? 0 : s.last);
  if (key.toLowerCase() === "f") return toggleFull;
  return null;
}
function menuKey(e) {
  const list = [...$("menu").querySelectorAll("button:not(:disabled)")];
  const i = list.indexOf(document.activeElement);
  if (e.key === "Escape" || e.key === "Tab") { e.preventDefault(); closeMenu(); }
  else if ((e.key === "ArrowDown" || e.key === "ArrowUp") && list.length) { e.preventDefault(); list[(i + (e.key === "ArrowDown" ? 1 : -1) + list.length) % list.length].focus(); }
}
function escapeKey() {
  $("pop").hidden = true;
  if (typing()) document.activeElement.blur();
  else if (full) toggleFull();
  else if (panelOpen) showPanel(false);
}
document.addEventListener("keydown", (e) => {
  if (composing(e) || listening) return;
  if (!$("menu").hidden) { menuKey(e); return; }
  const ctrl = e.ctrlKey && !e.altKey && !e.metaKey;
  if (ctrl && e.shiftKey && e.key.toLowerCase() === "i") { e.preventDefault(); invoke("open_debug"); return; }
  if (document.querySelector("dialog[open]")) {
    if (e.key === "?" && $("shortcuts").open) { $("shortcuts").close(); e.preventDefault(); }
    return;
  }
  if (!inRoom()) return;
  if (ctrl && ["1", "2", "3", "b", "r"].includes(e.key.toLowerCase())) { e.preventDefault(); return; }
  if (ctrl && e.key.toLowerCase() === "z" && !typing()) { e.preventDefault(); runUndo(); return; }
  if (e.key === "Escape") { escapeKey(); return; }
  if (typing() || nativeKey(e) || e.metaKey || e.altKey) return;
  const player = page === "player" && !!playSet();
  const key = e.key.normalize("NFKC");
  const k = key.length === 1 ? key.toLowerCase() : key;
  let action = null;
  if (e.ctrlKey) action = player && (key === "ArrowLeft" || key === "ArrowRight") ? () => jump(key === "ArrowLeft" ? -STEP.huge : STEP.huge) : null;
  else if (key === "?") action = menuActions.shortcuts;
  else if (page === "player") action = stageKey(e, key) || (player && (PLAYER_KEYS[(e.shiftKey ? "Shift+" : "") + k] || PLAYER_KEYS[k])) || null;
  if (!action) return;
  e.preventDefault();
  action(e);
});

async function reportCodecs() {
  const types = { av1: 'video/mp4; codecs="av01.0.08M.08"', hevc: 'video/mp4; codecs="hvc1.1.6.L93.B0"', h264: 'video/mp4; codecs="avc1.640028"' };
  const list = [];
  for (const [name, contentType] of Object.entries(types)) {
    try {
      const info = await navigator.mediaCapabilities.decodingInfo({ type: "file", video: { contentType, ...CODEC_PROBE } });
      if (info.supported) list.push(name);
    } catch (e) { warn(e); }
  }
  quiet("set_codecs", { list: list.length ? list : ["h264"] });
}
fillIcons();
reportCodecs();
setInterval(() => { if (!document.hidden) tick(); }, TICK_MS);
setInterval(() => { if (!document.hidden) refresh().catch(warn); }, REFRESH_MS);
setInterval(pollPlugins, JOBS_MS);
refresh();
setTimeout(() => checkUpdate(false), UPDATE_CHECK_DELAY_MS);
setInterval(() => checkUpdate(false), UPDATE_EVERY_MS);
window.__TAURI__.event.listen("shown", () => { if (Date.now() - checkedAt > UPDATE_RECHECK_MS) checkUpdate(false); });

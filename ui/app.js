const { invoke, convertFileSrc } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

const STEP = { small: 1, normal: 5, big: 10, huge: 60 };
const VOLUME_STEP = 0.1;
const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2];
const TICK_MS = 150;
const IDLE_MS = 2000;
const LIVE_LAG = 1.5;
const RELOAD_GROWTH = 300000;
const RETRY_MS = 3000;
const NOTICE_MS = 5000;
const CONTROL_REPEAT_MS = 4000;

const tiles = {};
const clock = { playing: false, m: 0, at: 0 };
let st = null;
let ready = false;
let tab = "record";
let selected = null;
let view = 0;
let lastSingle = 1;
let rate = 1;
let live = false;
let watchParty = true;
let lastSeek = null;
let noteSig = "";
let layoutSig = "";
let idleTimer = null;
let undoAction = null;
let noteItems = [];
let lastCtl = null;

window.__TAURI__.app.getName().then((name) => {
  $("appname").textContent = name;
  document.title = name;
});

const mb = (bytes) => (bytes / 1e6).toFixed(1) + " MB";
const gb = (bytes) => (bytes / 1e9).toFixed(1) + " GB";
const label = (p) => (p.name ? `${p.name} (${p.short})` : p.short);
function fmt(sec) {
  sec = Math.max(0, Math.floor(sec));
  const h = Math.floor(sec / 3600);
  const m = String(Math.floor((sec % 3600) / 60)).padStart(h ? 2 : 1, "0");
  return `${h ? h + ":" : ""}${m}:${String(sec % 60).padStart(2, "0")}`;
}
const setTitle = (id) => { const [d, t] = id.split(" "); return t ? `${d.slice(5)} ${t.replaceAll("-", ":").slice(0, 5)}` : id; };
function el(tag, props = {}, ...children) {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...children);
  return e;
}
const nameOf = (key) => (key === "me" ? "You" : label(st.peers.find((p) => p.id === key) || { short: key.slice(0, 10) }));

function liveSet() {
  if (!st.room) return null;
  const own = st.capture.live ? [{ key: "me", ...st.capture.live, offset_us: 0 }] : [];
  const peers = st.peers.filter((p) => p.live).map((p) => ({ key: p.id, ...p.live, offset_us: p.offset_us || 0 }));
  return { id: "live", live: true, players: [...own, ...peers].map((p) => ({ ...p, recording: true })) };
}
function currentSet() {
  if (!st) return null;
  if (tab === "record") return st.sets.find((s) => s.live) || liveSet() || st.sets[0] || null;
  return st.sets.find((s) => s.id === selected) || st.sets[0] || null;
}
function players() {
  const set = currentSet();
  if (!set) return [];
  return set.players.filter((p) => p.size > 0).map((p) => ({ key: p.key, label: nameOf(p.key), start: p.start_us, off: p.offset_us || 0, rec: p, recording: p.recording }));
}
function base() {
  const list = players();
  return list.length ? Math.min(...list.map((p) => p.start - p.off)) : Date.now() * 1000;
}
const deltaOf = (p, b) => (b + p.off - p.start) / 1e6;
function total() {
  const b = base();
  const ends = players().map((p) => ({ p, v: tiles[p.key] && tiles[p.key].video })).filter((x) => x.v && isFinite(x.v.duration)).map((x) => x.v.duration - deltaOf(x.p, b));
  return ends.length ? Math.max(...ends) : 0;
}
const nowM = () => (clock.playing ? clock.m + ((performance.now() - clock.at) / 1000) * rate : clock.m);
function setClock(playing, m) {
  clock.playing = playing;
  clock.m = Math.max(0, m);
  clock.at = performance.now();
  tick();
}
const wallOf = (m) => Math.round(base() + m * 1e6);

function loadOrder() {
  try { return JSON.parse(localStorage.getItem("order")) || []; } catch { return []; }
}
function keys() {
  const present = players().map((p) => p.key);
  const saved = loadOrder().filter((k) => present.includes(k));
  return [...saved, ...present.filter((k) => !saved.includes(k))];
}
function swap(a, b) {
  const list = keys();
  const i = list.indexOf(a), j = list.indexOf(b);
  [list[i], list[j]] = [list[j], list[i]];
  try { localStorage.setItem("order", JSON.stringify(list)); } catch {}
  applyView();
}

function tile(key) {
  if (tiles[key]) return tiles[key];
  const video = el("video", { muted: true, preload: "auto" });
  const name = el("span");
  const dot = el("span", { className: "dot" });
  const badge = el("span", { className: "key" });
  const box = el("div", { className: "tile", draggable: true }, video, el("div", { className: "label" }, dot, name), badge);
  box.onclick = () => { const i = keys().indexOf(key) + 1; view = view === i ? 0 : i; if (view) lastSingle = view; applyView(); };
  box.ondblclick = toggleFull;
  box.ondragstart = (e) => e.dataTransfer.setData("text/plain", key);
  box.ondragover = (e) => { e.preventDefault(); box.classList.add("over"); };
  box.ondragleave = () => box.classList.remove("over");
  box.ondrop = (e) => { e.preventDefault(); box.classList.remove("over"); const from = e.dataTransfer.getData("text/plain"); if (from && from !== key) swap(from, key); };
  $("videos").append(box);
  return (tiles[key] = { box, video, name, dot, badge });
}
function applyView() {
  const list = keys();
  if (view > list.length) view = 0;
  $("videos").classList.toggle("focus", view > 0);
  $("videos").style.setProperty("--n", Math.max(1, list.length - 1));
  list.forEach((key, i) => {
    const t = tile(key);
    const active = view === i + 1;
    t.box.classList.toggle("active", active);
    t.box.style.order = i;
    t.badge.textContent = i + 1;
    t.video.muted = !active && !(view === 0 && list.length === 1);
    t.video.volume = Number($("volume").value);
    t.video.playbackRate = rate;
  });
  const views = [el("button", { textContent: "All", className: view === 0 ? "on" : "", onclick: () => { view = 0; applyView(); } }),
    ...list.map((key, i) => el("button", { textContent: `${i + 1} ${nameOf(key)}`, className: view === i + 1 ? "on" : "", onclick: () => { view = i + 1; lastSingle = view; applyView(); } }))];
  const sig = JSON.stringify([view, list, list.map(nameOf)]);
  if (sig !== layoutSig) {
    layoutSig = sig;
    $("views").replaceChildren(...views);
    const shown = list.map((key, i) => ({ id: key, index: i, w: Math.round(tile(key).box.clientWidth), h: Math.round(tile(key).box.clientHeight) }));
    invoke("log_layout", { layout: { set: currentSet() && currentSet().id, mode: view ? "focus" : "grid", tiles: shown } }).catch(() => {});
  }
}

function reload(v, rec) {
  const t = v.currentTime;
  v.dataset.size = rec.size;
  v.dataset.retry = Date.now();
  v.src = v.dataset.src + "?r=" + Date.now();
  v.onloadedmetadata = () => { v.currentTime = t; };
}
function source(v, rec) {
  if (!rec || !rec.size) return false;
  const src = convertFileSrc(rec.path);
  if (v.dataset.src !== src) { v.dataset.src = src; v.src = src; v.dataset.size = rec.size; return true; }
  const loaded = Number(v.dataset.size);
  const settled = v.dataset.seen === String(rec.size);
  v.dataset.seen = rec.size;
  const stuck = v.error && Date.now() - Number(v.dataset.retry || 0) > RETRY_MS;
  const grown = rec.size > loaded + RELOAD_GROWTH && (v.paused || live);
  const final = settled && rec.size !== loaded && v.paused;
  if (stuck || grown || final) reload(v, rec);
  return true;
}

function tick() {
  if (!st) return;
  const tot = total();
  let m = nowM();
  if (clock.playing && tot && m >= tot) { clock.playing = false; clock.m = tot; m = tot; }
  const b = base();
  for (const p of players()) {
    const t = tiles[p.key];
    if (!t) continue;
    const v = t.video;
    const target = m + deltaOf(p, b);
    const inRange = target >= 0 && (!isFinite(v.duration) || target <= v.duration);
    if (!inRange || !clock.playing) {
      if (!v.paused) v.pause();
      if (v.readyState > 0 && Math.abs(v.currentTime - Math.max(0, target)) > 0.05) v.currentTime = Math.max(0, Math.min(target, v.duration || target));
    } else if (v.paused) {
      v.currentTime = target;
      v.play().catch(() => {});
    } else if (Math.abs(v.currentTime - target) > 0.4) {
      v.currentTime = target;
    }
  }
  $("clock").textContent = `${fmt(m)} / ${fmt(tot)}`;
  const frac = tot ? Math.min(1, m / tot) : 0;
  document.querySelector("#timeline .fill").style.width = frac * 100 + "%";
  document.querySelector("#timeline .head").style.left = frac * 100 + "%";
  $("play").textContent = clock.playing ? "Pause" : "Play";
}

const say = (text) => { $("who").textContent = text; };
function apply(cmd, m, who) {
  live = false;
  if (cmd === "play") setClock(true, m);
  else if (cmd === "pause") setClock(false, m);
  else if (cmd === "seek") setClock(clock.playing, m);
  say(`${who} ${cmd === "play" ? "played" : cmd === "pause" ? "paused" : "jumped"} at ${fmt(m)}`);
}
const offsetOf = (id) => { const p = st.peers.find((x) => x.id === id); return (p && p.offset_us) || 0; };
function isNewer(c) {
  if (!lastCtl) return true;
  const a = c.sent_us - offsetOf(c.origin), b = lastCtl.sent_us - offsetOf(lastCtl.origin);
  return a > b || (a === b && c.origin > lastCtl.origin);
}
function sendCtl(c) {
  invoke("control", { cmd: c.cmd, wallUs: c.wall_us, set: c.set, sentUs: c.sent_us, origin: c.origin }).catch(() => {});
}
function act(cmd, m) {
  if (cmd === "seek") { lastSeek = nowM(); undoAction = null; $("undo").hidden = false; $("undo").textContent = "Undo seek"; }
  apply(cmd, m, "You");
  const set = currentSet();
  if (!set) return;
  lastCtl = { cmd, wall_us: wallOf(m), set: set.id, sent_us: Date.now() * 1000, origin: st.id };
  if (watchParty) sendCtl(lastCtl);
}
window.__TAURI__.event.listen("control", (e) => {
  const set = currentSet();
  const c = e.payload;
  if (!st || !watchParty || !set || c.set !== set.id || c.origin === st.id || !isNewer(c)) return;
  lastCtl = c;
  const peer = st.peers.find((p) => p.id === c.origin);
  const age = c.cmd === "play" ? (Date.now() * 1000 - (c.sent_us - offsetOf(c.origin))) / 1e6 : 0;
  apply(c.cmd === "seek" ? "seek" : c.cmd, (c.wall_us - offsetOf(c.origin) - base()) / 1e6 + Math.max(0, age), peer ? peer.name || peer.short : "Someone");
});
setInterval(() => {
  const set = currentSet();
  if (watchParty && lastCtl && set && lastCtl.set === set.id) sendCtl(lastCtl);
}, CONTROL_REPEAT_MS);
const toggle = () => act(clock.playing ? "pause" : "play", nowM());
const jump = (delta) => act("seek", Math.min(total(), Math.max(0, nowM() + delta)));

function offerUndo(text, action) {
  say(text);
  undoAction = action;
  $("undo").hidden = false;
  $("undo").textContent = "Undo";
}

function renderNotes() {
  const set = currentSet();
  const b = base();
  noteItems = st.notes
    .filter((n) => set && (set.notes || []).includes(n.key))
    .map((n) => ({ id: n.id, mine: n.mine, m: (n.local_us - b) / 1e6, who: n.who, text: n.text }))
    .sort((x, y) => x.m - y.m);
  const sig = JSON.stringify(noteItems) + Math.round(total());
  if (sig === noteSig || $("notelist").querySelector("input")) return;
  noteSig = sig;
  $("noteshead").textContent = `Notes ${noteItems.length || ""}`;
  $("notelist").replaceChildren(...noteItems.map((n) => {
    const acts = el("span", { className: "acts" });
    const row = el("div", { className: "note", role: "button", tabIndex: 0 });
    if (n.mine) {
      acts.append(
        el("button", { textContent: "Edit", onclick: (e) => { e.stopPropagation(); editNote(n, row); } }),
        el("button", { textContent: "Delete", onclick: (e) => {
          e.stopPropagation();
          invoke("note_action", { kind: "delete", id: n.id, text: "" });
          offerUndo("Note deleted", () => invoke("note_action", { kind: "restore", id: n.id, text: "" }));
        } }),
      );
    }
    row.append(
      el("span", { className: "when", textContent: fmt(n.m) }),
      el("span", { className: "body" }, el("span", { textContent: n.text }), el("div", { className: "who", textContent: n.who })),
      acts);
    row.onclick = () => act("seek", Math.max(0, n.m));
    return row;
  }));
  const tot = total();
  document.querySelectorAll("#timeline .mark").forEach((x) => x.remove());
  if (tot) for (const n of noteItems) {
    const mark = el("div", { className: "mark", title: n.text });
    mark.style.left = Math.min(100, (n.m / tot) * 100) + "%";
    $("timeline").append(mark);
  }
}
function editNote(n, row) {
  const input = el("input", { value: n.text });
  input.onkeydown = (e) => {
    if (e.key === "Enter") { invoke("note_action", { kind: "edit", id: n.id, text: input.value }); noteSig = ""; }
    if (e.key === "Escape") { noteSig = ""; renderNotes(); }
    e.stopPropagation();
  };
  row.replaceChildren(input);
  input.focus();
}
function stepNote(dir) {
  const m = nowM();
  const list = noteItems.map((n) => n.m);
  const next = dir > 0 ? list.find((x) => x > m + 0.5) : [...list].reverse().find((x) => x < m - 0.5);
  if (next !== undefined) act("seek", Math.max(0, next));
}

function renderSide() {
  $("siderecord").hidden = tab !== "record";
  $("sideplayer").hidden = tab !== "player";
  $("tabrecord").classList.toggle("on", tab === "record");
  $("tabplayer").classList.toggle("on", tab === "player");
  $("roomtitle").textContent = st.room ? st.room.name : "";
  const me = el("div", { className: "peer" }, el("span", { className: "dot " + (st.recording ? "rec" : "") }),
    el("div", { className: "info" }, el("div", { className: "name", textContent: `${st.name || "You"} (you)` }), el("div", { className: "sub", textContent: [st.recording ? "recording" : "idle", st.capture.sharing ? "sharing" : "not sharing"].join(", ") })));
  const rows = st.peers.map((p) => {
    const set = st.sets.find((s) => s.live);
    const pl = set && set.players.find((x) => x.key === p.id);
    const route = p.connected ? `${p.path === "direct" ? "direct" : "via relay"}, ${p.rtt_ms} ms` : p.failures > 2 ? "offline" : "connecting";
    const info = el("div", { className: "info" }, el("div", { className: "name", textContent: label(p) }),
      el("div", { className: "sub", textContent: [pl && pl.recording ? "recording" : "", route, p.connected ? (p.sharing ? "live" : "no source") : "", pl && pl.size ? mb(pl.size) : ""].filter(Boolean).join(", ") }));
    if (p.message) info.append(el("div", { className: "sub fail", textContent: p.message }));
    return el("div", { className: "peer" }, el("span", { className: "dot " + (pl && pl.recording ? "rec" : p.connected ? p.path : p.failures > 2 ? "bad" : "") }), info,
      el("button", { textContent: "Remove", onclick: () => invoke("remove_peer", { id: p.id }) }));
  });
  $("members").replaceChildren(me, ...rows);
  $("cancelinvite").hidden = !st.invites;
  $("setlist").replaceChildren(...st.sets.map((s) => {
    const names = s.players.map((p) => nameOf(p.key)).join(", ");
    const notes = s.notes.length;
    const cur = currentSet();
    return el("button", { className: "set" + (cur && cur.id === s.id ? " sel" : ""), onclick: () => { selected = s.id; resetPlayback(); } },
      el("div", { className: "t" }, setTitle(s.id), s.live ? el("span", { className: "live-tag", textContent: "LIVE" }) : ""),
      el("div", { className: "p", textContent: names + (notes ? `, ${notes} notes` : "") }),
      el("div", { className: "p", textContent: s.status }));
  }));
  const rooms = st.rooms.map((r) => el("div", { className: "roomrow" + (r.active ? " active" : "") },
    el("span", { textContent: `${r.name} (${r.members})` }),
    r.active ? "" : el("button", { textContent: "Switch", onclick: () => invoke("switch_room", { id: r.id }).catch((e) => { $("roomerr").textContent = e; }) }),
    el("button", { textContent: "Leave", onclick: () => invoke("leave_room", { id: r.id }) })));
  $("roomlist").replaceChildren(...rooms);
  $("roombtn").textContent = st.room ? `${st.room.name} (${st.peers.length + 1})` : "Rooms";
}
function resetPlayback() {
  clock.playing = false;
  clock.m = 0;
  live = false;
  noteSig = "";
  layoutSig = "";
  for (const key of Object.keys(tiles)) { tiles[key].box.remove(); delete tiles[key]; }
}

async function refresh() {
  st = await invoke("state");
  if (!ready) { ready = true; invoke("ui_ready"); }
  for (const id of ["name", "name2"]) if (document.activeElement !== $(id)) $(id).value = st.name;
  $("share").checked = st.share;
  if (document.activeElement !== $("limit")) $("limit").value = st.limit_mbps || "";
  if (document.activeElement !== $("keepdays")) $("keepdays").value = st.keep_days || "";
  if (document.activeElement !== $("proc")) $("proc").value = st.audio_process;
  const warnings = [st.notice, st.ffmpeg, st.disk.low ? `Low disk space: ${gb(st.disk.free)} free` : ""].filter(Boolean);
  if (warnings.length) { $("notice").textContent = warnings.join(" "); $("notice").hidden = false; }
  else if (!flashing) $("notice").hidden = true;
  $("rec").textContent = st.recording ? "Stop" : "Record";
  $("rec").className = st.recording ? "rec-on" : "primary";
  const live_ = st.sets.find((s) => s.live);
  $("recinfo").textContent = st.active_set ? `${fmt((Date.now() * 1000 - st.active_set.start_us) / 1e6)}${st.active_set.by ? ", started by " + st.active_set.by : ""}` : live_ ? "recording" : "";
  const set = currentSet();
  if (tab === "record" && set && (!selected || live_)) selected = set.id;
  const wanted = new Set(keys());
  for (const key of Object.keys(tiles)) if (!wanted.has(key)) { tiles[key].box.remove(); delete tiles[key]; }
  let any = false;
  for (const p of players()) {
    const t = tile(p.key);
    t.name.textContent = p.label;
    t.dot.className = "dot " + (p.recording ? "rec" : "");
    any = source(t.video, p.rec) || any;
  }
  const inRoom = !!st.room;
  const waiting = tab === "record" && set && set.id === "live" ? st.peers.filter((p) => !p.live) : [];
  for (const [id, box] of Object.entries(placeholders)) if (!waiting.some((p) => p.id === id)) { box.remove(); delete placeholders[id]; }
  for (const p of waiting) {
    const box = placeholders[p.id] || (placeholders[p.id] = $("videos").appendChild(el("div", { className: "tile waiting" })));
    box.textContent = `${label(p)}: ${p.connected ? "connected, no source" : "not connected"}`;
  }
  any = any || waiting.length > 0;
  $("onboard").hidden = inRoom;
  $("empty").hidden = any || !inRoom;
  $("videos").hidden = !any;
  $("toolbar").hidden = !any;
  $("reviewwrap").classList.toggle("bare", !inRoom);
  applyView();
  renderSide();
  renderNotes();
  if (tab === "record" && set && set.live && !live) { live = true; }
  renderCapture();
  if (live && total() - nowM() > LIVE_LAG * 2) setClock(true, total() - LIVE_LAG);
  $("recovery").hidden = !st.recovery;
  $("recoverymsg").textContent = st.recovery || "";
  $("rinfo").textContent = st.previous_version ? `Version ${st.version}, previous run ${st.previous_version}` : `Version ${st.version}`;
  if (!$("debugpage").hidden) renderInfo();
}

const placeholders = {};
function renderCapture() {
  const c = st.capture;
  const value = c.source ? JSON.stringify(c.source) : "";
  if (document.activeElement !== $("source")) {
    if (![...$("source").options].some((o) => o.value === value)) $("source").append(el("option", { value, textContent: c.source.kind === "window" ? `${c.source.title} (${c.source.exe})` : `Display ${c.source.index}` }));
    $("source").value = value;
  }
  if (document.activeElement !== $("mic")) $("mic").value = [...$("mic").options].some((o) => o.value === c.mic) ? c.mic : "";
  $("lost").hidden = !c.source_lost;
  for (const kind of ["game", "mic"]) {
    if (document.activeElement !== $(kind + "vol")) $(kind + "vol").value = c.mix[kind].volume;
    $(kind + "mute").textContent = c.mix[kind].muted ? "Unmute" : "Mute";
  }
  $("capinfo").textContent = [c.source ? `${c.source_kind === "display" ? "Showing the whole display" : "Window"}: ${c.source_name || "not found"}` : "No source: nothing is captured or sent",
    c.game ? `game sound from ${c.game}` : "no game sound", c.mic_name ? `mic ${c.mic_name}` : "no microphone", c.status].filter(Boolean).join(". ");
}
async function fillSources() {
  const list = await invoke("list_sources");
  $("source").replaceChildren(el("option", { value: "", textContent: "No source" }), ...list.map((s) => el("option", { value: JSON.stringify(s.id), textContent: s.name })));
  if (st) renderCapture();
}
async function fillMics() {
  const list = await invoke("list_mics");
  $("mic").replaceChildren(el("option", { value: "", textContent: list.length ? "Default microphone" : "No microphone found" }), ...list.map((m) => el("option", { value: m.id, textContent: m.name })));
  if (st) renderCapture();
}
const setMix = (kind, muted) => invoke("set_mix", { kind, volume: Number($(kind + "vol").value), muted }).then(refresh);
for (const kind of ["game", "mic"]) {
  $(kind + "vol").oninput = () => setMix(kind, st.capture.mix[kind].muted);
  $(kind + "mute").onclick = () => setMix(kind, !st.capture.mix[kind].muted);
}
$("source").onfocus = fillSources;
$("source").onchange = () => invoke("set_source", { source: $("source").value ? JSON.parse($("source").value) : null }).then(() => $("source").blur());
$("mic").onfocus = fillMics;
$("mic").onchange = () => invoke("set_mic", { id: $("mic").value }).then(() => $("mic").blur());
$("usedisplay").onclick = () => invoke("use_display").then(fillSources);
window.__TAURI__.event.listen("levels", (e) => { $("gamelevel").value = e.payload.game; $("miclevel").value = e.payload.mic; });

function renderInfo() {
  const lines = [`version ${st.version}${st.previous_version ? ` (previous run ${st.previous_version})` : ""}`, `id ${st.id}`, `recording codec ${st.codec}`];
  for (const p of st.peers) lines.push(`peer ${label(p)}: ${p.connected ? p.path + ", " + p.rtt_ms + " ms" : "not connected"}${p.version ? ", v" + p.version : ""}${p.offset_us != null ? ", clock offset " + (p.offset_us / 1000).toFixed(1) + " ms" : ""}`);
  $("info").textContent = lines.join("\n");
}

let pending = null;
async function checkUpdate(manual) {
  const text = $("update");
  try {
    pending = await window.__TAURI__.updater.check();
  } catch (e) {
    if (manual) text.textContent = "update check failed";
    return;
  }
  if (!pending) { if (manual) text.textContent = "up to date"; return; }
  const button = el("button", { textContent: "Update to " + pending.version, className: "primary" });
  button.onclick = async () => {
    button.disabled = true;
    text.textContent = "installing " + pending.version + "; the app will restart";
    await pending.downloadAndInstall().catch(() => { text.textContent = "update failed"; });
  };
  text.replaceChildren(button);
}

async function runCheck() {
  $("checks").textContent = "checking...";
  const lines = await invoke("check", { audioProcess: $("proc").value });
  const bad = lines.filter((l) => l.level === "fail").length;
  const warn = lines.filter((l) => l.level === "warn").length;
  $("checksummary").textContent = bad ? `Check: ${bad} problem(s)` : warn ? `Check: ${warn} warning(s)` : "Check: OK";
  $("checks").replaceChildren(...lines.map((l) => el("div", { className: l.level === "info" ? "muted" : l.level, textContent: l.text })));
}
async function fillProcesses() {
  const names = await invoke("list_processes");
  const current = $("proc").value;
  $("proc").replaceChildren(el("option", { value: "", textContent: "Game sound: automatic" }), ...names.map((n) => el("option", { textContent: n })));
  $("proc").value = current || (st ? st.audio_process : "");
}
async function refreshDebug() {
  if (!$("debugpage").hidden) {
    $("debug").textContent = await invoke("debug_text");
    const info = await invoke("data_info");
    $("datainfo").textContent = `${info.dir}\n${mb(info.used_bytes)} used, ${gb(info.free_bytes)} free`;
  }
}

function toggleFull() {
  if (document.fullscreenElement) document.exitFullscreen();
  else $("reviewwrap").requestFullscreen().catch(() => {});
}
function wake() {
  $("reviewwrap").classList.remove("idle");
  clearTimeout(idleTimer);
  idleTimer = setTimeout(() => $("reviewwrap").classList.add("idle"), IDLE_MS);
}
document.addEventListener("mousemove", wake);
document.addEventListener("fullscreenchange", wake);

function closePops() { for (const id of ["roompop", "settings", "helpbox"]) $(id).hidden = true; }
function pop(id) { const open = $(id).hidden; closePops(); $(id).hidden = !open; }
let flashing = false;
function flash(text) {
  flashing = true;
  $("notice").textContent = text;
  $("notice").hidden = false;
  setTimeout(() => { flashing = false; $("notice").hidden = true; }, NOTICE_MS);
}

const record = () => (st && st.recording ? invoke("stop_recording") : invoke("start_recording").catch((e) => flash(e)));
const showCode = (code) => { $("invitebox").hidden = false; $("code").value = code; };
const makeInvite = () => invoke("create_invite", { reuse: false }).then(showCode);
const createRoom = () => invoke("create_room", { name: "" }).then(() => { tab = "record"; return makeInvite(); }).catch((e) => flash(e));
const join = (field, err) => invoke("join_room", { code: $(field).value }).then(() => { $(field).value = ""; $(err).textContent = ""; closePops(); }).catch((e) => { $(err).textContent = e; });
function openDebug() { closePops(); $("debugpage").hidden = false; renderInfo(); runCheck(); refreshDebug(); }
function focusPlayer(delta) {
  const n = keys().length;
  view = view === 0 ? (delta > 0 ? 1 : n) : ((view - 1 + delta + n) % n) + 1;
  lastSingle = view;
  applyView();
}
function showTab(name) {
  tab = name;
  if (name === "record") selected = null;
  resetPlayback();
  refresh();
}

$("rec").onclick = record;
$("tabrecord").onclick = () => showTab("record");
$("tabplayer").onclick = () => { selected = (st.sets[0] || {}).id; showTab("player"); };
$("play").onclick = toggle;
$("back").onclick = () => jump(-STEP.normal);
$("fwd").onclick = () => jump(STEP.normal);
$("back60").onclick = () => jump(-STEP.huge);
$("fwd60").onclick = () => jump(STEP.huge);
$("wp").onclick = () => { watchParty = !watchParty; $("wp").classList.toggle("on", watchParty); };
$("undo").onclick = () => {
  if (undoAction) { undoAction(); undoAction = null; }
  else if (lastSeek !== null) act("seek", lastSeek);
  lastSeek = null;
  $("undo").hidden = true;
};
$("live").onclick = () => { live = true; setClock(true, Math.max(0, total() - LIVE_LAG)); };
$("speed").onchange = (e) => { rate = Number(e.target.value); clock.m = nowM(); clock.at = performance.now(); applyView(); };
$("volume").oninput = applyView;
$("full").onclick = toggleFull;
$("timeline").onclick = (e) => {
  const r = $("timeline").getBoundingClientRect();
  act("seek", Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)) * total());
};
$("noteform").onsubmit = (e) => {
  e.preventDefault();
  const set = currentSet();
  if (set) invoke("add_note", { text: $("note").value, atUs: wallOf(nowM()) });
  $("note").value = "";
  $("note").blur();
};
$("roombtn").onclick = () => pop("roompop");
$("newroom").onclick = () => { closePops(); createRoom(); };
$("invite").onclick = makeInvite;
$("invite2").onclick = createRoom;
$("cancelinvite").onclick = () => invoke("cancel_invites").then(() => { $("invitebox").hidden = true; });
$("copycode").onclick = () => navigator.clipboard.writeText($("code").value);
$("join").onclick = () => join("joincode", "roomerr");
$("join2").onclick = () => join("joincode2", "roomerr2");
$("closesettings").onclick = closePops;
$("proc").onchange = () => invoke("set_audio_process", { name: $("proc").value });
$("keepdays").onchange = () => invoke("set_keep_days", { days: $("keepdays").value ? Number($("keepdays").value) : null });
$("name").onchange = () => invoke("set_name", { name: $("name").value });
$("name2").onchange = () => invoke("set_name", { name: $("name2").value });
$("share").onchange = () => invoke("set_share", { on: $("share").checked });
$("limit").onchange = () => invoke("set_limit", { mbps: $("limit").value ? Number($("limit").value) : null });
$("closedebug").onclick = () => { $("debugpage").hidden = true; };
$("report").onclick = () => invoke("save_report", { includeAddresses: $("addr").checked, audioProcess: $("proc").value }).then((p) => { $("reportpath").textContent = p; }).catch((e) => { $("reportpath").textContent = String(e); });
$("runcheck").onclick = runCheck;
$("openfolder").onclick = () => invoke("open_data_folder");
$("openfolder2").onclick = () => invoke("open_data_folder");
$("resetopen").onclick = () => { $("resetpanel").hidden = false; };
$("resetcancel").onclick = () => { $("resetpanel").hidden = true; $("reseterr").textContent = ""; };
document.querySelectorAll("input[name=level]").forEach((r) => {
  r.onchange = () => { $("confirm").hidden = document.querySelector("input[name=level]:checked").value !== "all"; };
});
$("doreset").onclick = () => {
  const everything = document.querySelector("input[name=level]:checked").value === "all";
  invoke("reset", { everything, confirm: $("confirm").value }).catch((e) => { $("reseterr").textContent = String(e); });
};
$("rfolder").onclick = () => invoke("open_data_folder");
$("rreport").onclick = () => invoke("save_report", { includeAddresses: false, audioProcess: "" }).then((p) => { $("rinfo").textContent = p; });
$("rreset").onclick = () => invoke("reset", { everything: false, confirm: "" });
$("rcontinue").onclick = () => invoke("dismiss_recovery");
$("proc").onfocus = fillProcesses;

const menuActions = {
  "settings": () => { closePops(); $("settings").hidden = false; },
  "create-room": createRoom,
  "join-room": () => { closePops(); $("roompop").hidden = false; $("joincode").focus(); },
  "copy-invite": () => invoke("create_invite", { reuse: true }).then((code) => { showCode(code); navigator.clipboard.writeText(code); flash("Invite code copied"); }),
  "cancel-invite": () => invoke("cancel_invites").then(() => { $("invitebox").hidden = true; flash("Invite cancelled"); }),
  "leave-room": () => st && st.room && invoke("leave_room", { id: st.room.id }),
  "fullscreen": toggleFull,
  "all-players": () => { view = 0; applyView(); },
  "next-player": () => focusPlayer(1),
  "prev-player": () => focusPlayer(-1),
  "shortcuts": () => { closePops(); $("helpbox").hidden = false; },
  "debug": openDebug,
  "update": () => checkUpdate(true),
};
window.__TAURI__.event.listen("menu", (e) => { if (menuActions[e.payload]) menuActions[e.payload](); });
window.__TAURI__.event.listen("joined", (e) => flash(`${e.payload} joined`));

function changeRate(dir) {
  rate = SPEEDS[Math.max(0, Math.min(SPEEDS.length - 1, SPEEDS.indexOf(rate) + dir))];
  $("speed").value = rate;
  clock.m = nowM();
  clock.at = performance.now();
  applyView();
}
document.addEventListener("keydown", (e) => {
  const typing = /^(INPUT|SELECT|TEXTAREA)$/.test(document.activeElement.tagName);
  if (e.key === "Escape") {
    if (document.activeElement && typing) document.activeElement.blur();
    else if (document.fullscreenElement) document.exitFullscreen();
    else closePops();
    return;
  }
  if (typing || e.ctrlKey || e.metaKey || e.altKey || !st) return;
  const k = e.key.toLowerCase();
  const stepSize = (a, b) => (e.shiftKey ? a : b);
  if ((k === " " || k === "k") && !e.repeat) toggle();
  else if (e.key === "ArrowLeft") jump(-stepSize(STEP.small, STEP.normal));
  else if (e.key === "ArrowRight") jump(stepSize(STEP.small, STEP.normal));
  else if (k === "j") jump(-stepSize(STEP.huge, STEP.big));
  else if (k === "l") jump(stepSize(STEP.huge, STEP.big));
  else if (e.key === "<" || e.key === ",") changeRate(-1);
  else if (e.key === ">" || e.key === ".") changeRate(1);
  else if (k === "m") { $("volume").value = Number($("volume").value) > 0 ? 0 : 1; applyView(); }
  else if (e.key === "ArrowUp") { $("volume").value = Math.min(1, Number($("volume").value) + VOLUME_STEP); applyView(); }
  else if (e.key === "ArrowDown") { $("volume").value = Math.max(0, Number($("volume").value) - VOLUME_STEP); applyView(); }
  else if (k === "f") toggleFull();
  else if (/^[0-9]$/.test(e.key)) { view = Number(e.key); if (view) lastSingle = view; applyView(); }
  else if (e.key === "Tab") { view = view ? 0 : lastSingle; applyView(); }
  else if (k === "c") $("note").focus();
  else if (k === "q") stepNote(-1);
  else if (k === "e") stepNote(1);
  else if (k === "u") $("undo").click();
  else if (e.key === "?") pop("helpbox");
  else return;
  e.preventDefault();
});

async function reportCodecs() {
  const types = { av1: 'video/mp4; codecs="av01.0.08M.08"', hevc: 'video/mp4; codecs="hvc1.1.6.L93.B0"', h264: 'video/mp4; codecs="avc1.640028"' };
  const list = [];
  for (const [name, contentType] of Object.entries(types)) {
    try {
      const info = await navigator.mediaCapabilities.decodingInfo({ type: "file", video: { contentType, width: 1280, height: 720, bitrate: 6000000, framerate: 30 } });
      if (info.supported) list.push(name);
    } catch {}
  }
  invoke("set_codecs", { list: list.length ? list : ["h264"] });
}
reportCodecs();
setInterval(tick, TICK_MS);
setInterval(() => refresh().catch(() => {}), 1000);
setInterval(() => refreshDebug().catch(() => {}), 2000);
refresh();
fillProcesses();
fillSources();
fillMics();
setTimeout(() => checkUpdate(false), 4000);

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
const LOW_DISK = 5e9;

const tiles = {};
const clock = { playing: false, m: 0, at: 0 };
const chipState = {};
let st = null;
let ready = false;
let view = 0;
let lastSingle = 1;
let rate = 1;
let live = false;
let lastSeek = null;
let noteSig = "";
let layoutSig = "";
let rateSample = {};
let idleTimer = null;

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
function el(tag, props = {}, ...children) {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...children);
  return e;
}

function loadOrder() {
  try { return JSON.parse(localStorage.getItem("order")) || []; } catch { return []; }
}
function keys() {
  const present = ["me", ...st.peers.map((p) => p.id)];
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

function players() {
  const list = [];
  if (st.mine) list.push({ key: "me", label: "You", start: st.mine.start_us, off: 0, rec: st.mine });
  for (const p of st.peers) if (p.rec && p.rec.size) list.push({ key: p.id, label: label(p), start: p.rec.start_us, off: p.offset_us || 0, rec: p.rec });
  return list;
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
  const sig = JSON.stringify([view, list]);
  if (sig !== layoutSig) {
    layoutSig = sig;
    const shown = list.map((key, i) => ({ id: key === "me" ? "me" : key, index: i, w: Math.round(tile(key).box.clientWidth), h: Math.round(tile(key).box.clientHeight) }));
    invoke("log_layout", { layout: { mode: view ? "focus" : "grid", tiles: shown } }).catch(() => {});
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
function act(cmd, m) {
  if (cmd === "seek") { lastSeek = nowM(); $("undo").hidden = false; $("undo").textContent = "Undo seek"; }
  apply(cmd, m, "You");
  if (players().length) invoke("control", { cmd, wallUs: wallOf(m) }).catch(() => {});
}
window.__TAURI__.event.listen("control", (e) => {
  if (!st) return;
  const peer = st.peers.find((p) => p.id === e.payload.from);
  apply(e.payload.cmd, (e.payload.wall_us - ((peer && peer.offset_us) || 0) - base()) / 1e6, peer ? peer.name || peer.short : "Someone");
});
const toggle = () => act(clock.playing ? "pause" : "play", nowM());
const jump = (delta) => act("seek", Math.min(total(), Math.max(0, nowM() + delta)));

let undoAction = null;
function offerUndo(text, action) {
  say(text);
  undoAction = action;
  $("undo").hidden = false;
  $("undo").textContent = "Undo";
}

let noteItems = [];
function renderNotes() {
  const b = base();
  noteItems = st.notes
    .map((n) => {
      const peer = st.peers.find((p) => p.id === n.from);
      return { id: n.id, mine: n.from === "me", m: (n.at_us - ((peer && peer.offset_us) || 0) - b) / 1e6, who: n.from === "me" ? "You" : n.who, text: n.text };
    })
    .sort((x, y) => x.m - y.m);
  const sig = JSON.stringify(noteItems) + Math.round(total());
  if (sig === noteSig || $("notelist").querySelector("input")) return;
  noteSig = sig;
  $("notelist").replaceChildren(...noteItems.map((n) => {
    const acts = el("span", { className: "acts" });
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
    const row = el("div", { className: "note", role: "button", tabIndex: 0 },
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

function renderChips() {
  const entries = [{ key: "me", label: "You", dot: st.recording ? "rec" : "", text: st.recording && st.mine ? `rec ${fmt((Date.now() * 1000 - st.mine.start_us) / 1e6)}` : "", sig: "" }];
  for (const p of st.peers) {
    const route = p.connected ? "" : p.failures > 2 ? "offline" : "connecting";
    const recText = p.recording && p.rec ? `rec ${fmt((Date.now() * 1000 + (p.offset_us || 0) - p.rec.start_us) / 1e6)}` : "";
    const got = p.rec && p.rec.size ? mb(p.rec.size) : "";
    const prev = rateSample[p.id];
    const speed = prev && p.rec && p.rec.size > prev.size ? ` ${(((p.rec.size - prev.size) * 8) / 1e6 / ((Date.now() - prev.at) / 1000)).toFixed(1)} Mbit/s` : "";
    if (p.rec) rateSample[p.id] = { size: p.rec.size, at: Date.now() };
    entries.push({ key: p.id, label: label(p), dot: p.recording ? "rec" : p.connected ? p.path : p.failures > 2 ? "bad" : "", text: [route, recText, got && p.recording ? got + speed : ""].filter(Boolean).join(", "), sig: `${p.connected}${p.path}${p.recording}` });
  }
  const chips = $("chips");
  const keep = new Set(entries.map((e) => e.key));
  for (const key of Object.keys(chipState)) if (!keep.has(key)) { chipState[key].box.remove(); delete chipState[key]; }
  for (const e of entries) {
    let c = chipState[e.key];
    if (!c) {
      const dot = el("span", { className: "dot" });
      const text = el("span");
      c = chipState[e.key] = { box: el("span", { className: "chip" }, dot, text), dot, text, sig: e.sig };
      chips.append(c.box);
    }
    c.dot.className = "dot " + e.dot;
    c.text.textContent = e.text ? `${e.label}: ${e.text}` : e.label;
    if (c.sig !== e.sig) { c.sig = e.sig; c.box.classList.remove("flash"); void c.box.offsetWidth; c.box.classList.add("flash"); }
  }
}

function renderRoom() {
  const inRoom = st.peers.length > 0;
  $("members").replaceChildren(...st.peers.map((p) => el("div", { className: "member" },
    el("span", { className: "dot " + (p.connected ? p.path : p.failures > 2 ? "bad" : "") }),
    el("span", { textContent: label(p) + (p.message ? ` - ${p.message}` : "") }),
    el("button", { textContent: "Remove", onclick: () => invoke("remove_peer", { id: p.id }) }))));
  $("rejoin").hidden = inRoom || !st.can_rejoin;
  $("roombtn").textContent = inRoom ? `Room (${st.peers.length + 1})` : "Room";
  $("onboard").hidden = inRoom;
}

async function refresh() {
  st = await invoke("state");
  if (!ready) { ready = true; invoke("ui_ready"); }
  for (const id of ["name", "name2"]) if (document.activeElement !== $(id)) $(id).value = st.name;
  $("share").checked = st.share;
  if (document.activeElement !== $("limit")) $("limit").value = st.limit_mbps || "";
  if (document.activeElement !== $("keepdays")) $("keepdays").value = st.keep_days || "";
  if (document.activeElement !== $("proc")) $("proc").value = st.audio_process;
  const warnings = [st.notice, st.ffmpeg, st.free_bytes && st.free_bytes < LOW_DISK ? `Low disk space: ${gb(st.free_bytes)} free` : ""].filter(Boolean);
  $("notice").textContent = warnings.join(" ");
  $("notice").hidden = !warnings.length;
  $("rec").textContent = st.recording ? "Stop" : "Record";
  $("rec").className = st.recording ? "rec-on" : "primary";
  $("recinfo").textContent = st.recording && st.mine ? fmt((Date.now() * 1000 - st.mine.start_us) / 1e6) : "";
  const ids = new Set(keys());
  for (const key of Object.keys(tiles)) if (!ids.has(key)) { tiles[key].box.remove(); delete tiles[key]; }
  const shown = [];
  tile("me").name.textContent = "You";
  tile("me").dot.className = "dot " + (st.recording ? "rec" : "");
  shown.push(source(tile("me").video, st.mine));
  for (const p of st.peers) {
    const t = tile(p.id);
    t.name.textContent = label(p);
    t.dot.className = "dot " + (p.recording ? "rec" : p.connected ? p.path : "");
    shown.push(source(t.video, p.rec));
  }
  tile("me").box.hidden = !shown[0];
  st.peers.forEach((p, i) => { tiles[p.id].box.hidden = !shown[i + 1]; });
  const any = shown.some(Boolean);
  $("empty").hidden = any || st.peers.length === 0;
  $("videos").hidden = !any;
  $("reviewwrap").classList.toggle("bare", !any && st.notes.length === 0);
  applyView();
  renderChips();
  renderRoom();
  renderNotes();
  if (live && total() - nowM() > LIVE_LAG * 2) setClock(true, total() - LIVE_LAG);
  $("recovery").hidden = !st.recovery;
  $("recoverymsg").textContent = st.recovery || "";
  $("rinfo").textContent = st.previous_version ? `Version ${st.version}, previous run ${st.previous_version}` : `Version ${st.version}`;
  if (!$("debugpage").hidden) renderInfo();
}

function renderInfo() {
  const lines = [`version ${st.version}${st.previous_version ? ` (previous run ${st.previous_version})` : ""}`, `id ${st.id}`];
  for (const p of st.peers) lines.push(`peer ${label(p)}: ${p.connected ? p.path + ", " + p.rtt_ms + " ms" : "not connected"}${p.version ? ", v" + p.version : ""}${p.offset_us != null ? ", clock offset " + (p.offset_us / 1000).toFixed(1) + " ms" : ""}${p.rec ? ", " + mb(p.rec.size) : ""}`);
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

const record = () => (st && st.recording ? invoke("stop_recording") : invoke("start_recording").catch((e) => { $("notice").textContent = e; $("notice").hidden = false; }));
const showCode = (code) => { for (const [box, input] of [["invitebox", "code"], ["invitebox2", "code2"]]) { $(box).hidden = false; $(input).value = code; } };
const createInvite = () => invoke("create_invite", { reuse: false }).then(showCode);
const join = (field, err) => invoke("join_room", { code: $(field).value }).then(() => { $(field).value = ""; $(err).textContent = ""; closePops(); }).catch((e) => { $(err).textContent = e; });

$("rec").onclick = record;
$("play").onclick = toggle;
$("back").onclick = () => jump(-STEP.normal);
$("fwd").onclick = () => jump(STEP.normal);
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
  invoke("add_note", { text: $("note").value, atUs: wallOf(nowM()) });
  $("note").value = "";
  $("note").blur();
};
$("roombtn").onclick = () => pop("roompop");
$("invite").onclick = createInvite;
$("invite2").onclick = createInvite;
$("copycode").onclick = () => navigator.clipboard.writeText($("code").value);
$("copycode2").onclick = () => navigator.clipboard.writeText($("code2").value);
$("join").onclick = () => join("joincode", "roomerr");
$("join2").onclick = () => join("joincode2", "roomerr2");
$("leave2").onclick = () => invoke("leave_room");
$("rejoin").onclick = () => invoke("rejoin");
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

function openNoteInput() { $("note").focus(); }
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
  else if (e.key === "<" || e.key === ",") { const i = Math.max(0, SPEEDS.indexOf(rate) - 1); rate = SPEEDS[i]; $("speed").value = rate; clock.m = nowM(); clock.at = performance.now(); applyView(); }
  else if (e.key === ">" || e.key === ".") { const i = Math.min(SPEEDS.length - 1, SPEEDS.indexOf(rate) + 1); rate = SPEEDS[i]; $("speed").value = rate; clock.m = nowM(); clock.at = performance.now(); applyView(); }
  else if (k === "m") { $("volume").value = Number($("volume").value) > 0 ? 0 : 1; applyView(); }
  else if (e.key === "ArrowUp") { $("volume").value = Math.min(1, Number($("volume").value) + VOLUME_STEP); applyView(); }
  else if (e.key === "ArrowDown") { $("volume").value = Math.max(0, Number($("volume").value) - VOLUME_STEP); applyView(); }
  else if (k === "f") toggleFull();
  else if (/^[0-9]$/.test(e.key)) { view = Number(e.key); if (view) lastSingle = view; applyView(); }
  else if (e.key === "Tab") { view = view ? 0 : lastSingle; applyView(); }
  else if (k === "c") openNoteInput();
  else if (k === "q") stepNote(-1);
  else if (k === "e") stepNote(1);
  else if (k === "u") $("undo").click();
  else if (e.key === "?") pop("helpbox");
  else return;
  e.preventDefault();
});

function openDebug() { closePops(); $("debugpage").hidden = false; renderInfo(); runCheck(); refreshDebug(); }
function focusPlayer(delta) {
  const n = keys().length;
  view = view === 0 ? (delta > 0 ? 1 : n) : ((view - 1 + delta + n) % n) + 1;
  lastSingle = view;
  applyView();
}
function flash(text) {
  $("notice").textContent = text;
  $("notice").hidden = false;
  setTimeout(() => { $("notice").hidden = true; }, 5000);
}
const menuActions = {
  "settings": () => { closePops(); $("settings").hidden = false; },
  "create-room": () => { closePops(); $("roompop").hidden = false; createInvite(); },
  "join-room": () => { closePops(); $("roompop").hidden = false; $("joincode").focus(); },
  "copy-invite": () => invoke("create_invite", { reuse: true }).then((code) => { showCode(code); navigator.clipboard.writeText(code); flash("Invite code copied"); }),
  "cancel-invite": () => invoke("cancel_invites").then(() => { for (const b of ["invitebox", "invitebox2"]) $(b).hidden = true; flash("Invite cancelled"); }),
  "leave-room": () => invoke("leave_room"),
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
setInterval(tick, TICK_MS);
setInterval(() => refresh().catch(() => {}), 1000);
setInterval(() => refreshDebug().catch(() => {}), 2000);
refresh();
fillProcesses();
setTimeout(() => checkUpdate(false), 4000);

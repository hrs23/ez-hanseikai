const { invoke, convertFileSrc } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const tiles = {};
const clock = { playing: false, m: 0, at: 0 };
let st = null;
let ready = false;
let view = 0;
let rate = 1;
let live = false;
let collapsed = false;
let lastSeek = null;
let noteSig = "";

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
  const s = String(sec % 60).padStart(2, "0");
  return h ? `${h}:${m}:${s}` : `${m}:${s}`;
}
function el(tag, props = {}, ...children) {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...children);
  return e;
}

function players() {
  const list = [];
  if (st.mine) list.push({ key: "me", label: "You", start: st.mine.start_us, off: 0, rec: st.mine, recording: st.recording });
  for (const p of st.peers) if (p.rec) list.push({ key: p.id, label: label(p), start: p.rec.start_us, off: p.offset_us || 0, rec: p.rec, recording: p.recording });
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
  const box = el("div", { className: "tile" }, video, el("div", { className: "label" }, dot, name), badge);
  box.onclick = () => { view = view === order().indexOf(key) + 1 ? 0 : order().indexOf(key) + 1; applyView(); };
  $("videos").append(box);
  return (tiles[key] = { box, video, name, dot, badge });
}
const order = () => ["me", ...st.peers.map((p) => p.id)];
function applyView() {
  const keys = order();
  if (view > keys.length) view = 0;
  $("videos").classList.toggle("focus", view > 0);
  keys.forEach((key, i) => {
    const t = tile(key);
    const active = view === i + 1;
    t.box.classList.toggle("active", active);
    t.badge.textContent = i + 1;
    t.video.muted = !active;
    t.video.volume = Number($("volume").value);
    t.video.playbackRate = rate;
  });
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
  const stuck = v.error && Date.now() - Number(v.dataset.retry || 0) > 3000;
  const grown = rec.size > loaded + 300000 && (v.paused || live);
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

function say(text) { $("who").textContent = text; }
function apply(cmd, m, who) {
  live = false;
  if (cmd === "play") setClock(true, m);
  else if (cmd === "pause") setClock(false, m);
  else if (cmd === "seek") setClock(clock.playing, m);
  say(`${who} ${cmd === "play" ? "played" : cmd === "pause" ? "paused" : "jumped"} at ${fmt(m)}`);
}
function act(cmd, m) {
  if (cmd === "seek") { lastSeek = nowM(); $("undo").hidden = false; }
  apply(cmd, m, "You");
  if (players().length) invoke("control", { cmd, wallUs: wallOf(m) }).catch(() => {});
}
window.__TAURI__.event.listen("control", (e) => {
  if (!st) return;
  const peer = st.peers.find((p) => p.id === e.payload.from);
  const m = (e.payload.wall_us - ((peer && peer.offset_us) || 0) - base()) / 1e6;
  apply(e.payload.cmd, m, peer ? peer.name || peer.short : "Someone");
});
const toggle = () => act(clock.playing ? "pause" : "play", nowM());
const jump = (delta) => act("seek", Math.min(total(), Math.max(0, nowM() + delta)));

function renderNotes() {
  const b = base();
  const items = st.notes
    .map((n) => {
      const peer = st.peers.find((p) => p.id === n.from);
      return { m: (n.at_us - ((peer && peer.offset_us) || 0) - b) / 1e6, who: n.from === "me" ? "You" : n.who, text: n.text };
    })
    .sort((x, y) => x.m - y.m);
  const sig = JSON.stringify(items) + Math.round(total());
  if (sig === noteSig) return;
  noteSig = sig;
  $("notelist").replaceChildren(...items.map((n) => {
    const row = el("button", { className: "note" }, el("span", { className: "when", textContent: fmt(n.m) }), el("span", { textContent: n.text }), el("div", { className: "who", textContent: n.who }));
    row.onclick = () => act("seek", Math.max(0, n.m));
    return row;
  }));
  const tot = total();
  document.querySelectorAll("#timeline .mark").forEach((x) => x.remove());
  if (tot) for (const n of items) {
    const mark = el("div", { className: "mark", title: n.text });
    mark.style.left = Math.min(100, (n.m / tot) * 100) + "%";
    $("timeline").append(mark);
  }
}

const chipState = {};
function renderChips() {
  const entries = [{ key: "me", label: "You", dot: st.recording ? "rec" : "", text: st.recording && st.mine ? `rec ${fmt((Date.now() * 1000 - st.mine.start_us) / 1e6)}` : "idle" }];
  for (const p of st.peers) {
    const route = p.connected ? `${p.path === "direct" ? "direct" : "relay"} ${p.rtt_ms} ms` : p.failures > 2 ? "offline" : "connecting";
    const recText = p.recording && p.rec ? `rec ${fmt((Date.now() * 1000 + (p.offset_us || 0) - p.rec.start_us) / 1e6)}, ` : "";
    entries.push({ key: p.id, label: label(p), dot: p.recording ? "rec" : p.connected ? p.path : p.failures > 2 ? "bad" : "", text: recText + route, sig: `${p.connected}${p.path}${p.recording}` });
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
    c.text.textContent = `${e.label}: ${e.text}`;
    if (c.sig !== e.sig) { c.sig = e.sig; c.box.classList.remove("flash"); void c.box.offsetWidth; c.box.classList.add("flash"); }
  }
}

function peerRow(p) {
  const route = p.connected ? `${p.path === "direct" ? "direct" : "via relay"}, ${p.rtt_ms} ms` : p.failures > 2 ? "not reachable" : "connecting...";
  const hint = p.message || (!p.connected && p.failures > 2 ? "Is their app open? If Windows asked about the firewall, it must be allowed." : "");
  const info = el("div", { className: "info" }, el("div", { className: "name", textContent: label(p) }), el("div", { className: "sub", textContent: route }));
  if (hint) info.append(el("div", { className: "sub fail", textContent: hint }));
  return el("div", { className: "peer" }, el("span", { className: "dot " + (p.connected ? p.path : p.failures > 2 ? "bad" : "") }), info, el("button", { textContent: "Remove", onclick: () => invoke("remove_peer", { id: p.id }) }));
}

async function refresh() {
  st = await invoke("state");
  if (!ready) { ready = true; invoke("ui_ready"); }
  $("id").value = st.id;
  if (document.activeElement !== $("name")) $("name").value = st.name;
  $("version").textContent = "v" + st.version;
  $("ffmpeg").textContent = st.ffmpeg;
  $("notice").textContent = st.notice;
  for (const id of ["rec", "rec2"]) {
    $(id).textContent = st.recording ? "Stop" : "Record";
    $(id).className = st.recording ? "rec-on" : "primary";
  }
  $("peers").replaceChildren(...st.peers.map(peerRow));
  const parts = [];
  if (st.mine) parts.push(`you ${mb(st.mine.size)}`);
  if (st.free_bytes && st.free_bytes < 5e9) parts.push(`low disk space: ${gb(st.free_bytes)} free`);
  $("recstate").textContent = parts.join(", ");
  const ids = new Set(order());
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
  $("empty").hidden = shown.some(Boolean);
  $("videos").hidden = !shown.some(Boolean);
  applyView();
  renderChips();
  renderNotes();
  if (live && total() - nowM() > 3) setClock(true, total() - 1.5);
  if (!collapsed && st.peers.length && st.peers.every((p) => p.connected)) { collapsed = true; $("setup").open = false; }
  $("recovery").hidden = !st.recovery;
  $("recoverymsg").textContent = st.recovery || "";
  $("rinfo").textContent = st.previous_version ? `Version ${st.version}, previous run ${st.previous_version}` : `Version ${st.version}`;
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
  $("proc").replaceChildren(el("option", { value: "", textContent: "No game sound" }), ...names.map((n) => el("option", { textContent: n })));
  $("proc").value = current || names.find((n) => /apex/i.test(n)) || "";
}
async function refreshDebug() {
  if ($("debug").parentElement.open) $("debug").textContent = await invoke("debug_text");
}
async function refreshFiles() {
  if (!$("datainfo").parentElement.open) return;
  const info = await invoke("data_info");
  $("datainfo").textContent = `${info.dir}\n${mb(info.used_bytes)} used, ${gb(info.free_bytes)} free`;
}

const record = () => (st && st.recording ? invoke("stop_recording") : invoke("start_recording", { audioProcess: $("proc").value }).catch((e) => { $("notice").textContent = e; $("setup").open = true; }));
$("play").onclick = toggle;
$("back").onclick = () => jump(-5);
$("fwd").onclick = () => jump(5);
$("undo").onclick = () => { if (lastSeek !== null) act("seek", lastSeek); lastSeek = null; $("undo").hidden = true; };
$("live").onclick = () => { live = true; setClock(true, Math.max(0, total() - 1.5)); };
$("speed").onchange = (e) => { rate = Number(e.target.value); clock.m = nowM(); clock.at = performance.now(); applyView(); };
$("volume").oninput = applyView;
$("timeline").onclick = (e) => {
  const r = $("timeline").getBoundingClientRect();
  act("seek", Math.max(0, Math.min(1, (e.clientX - r.left) / r.width)) * total());
};
$("noteform").onsubmit = (e) => {
  e.preventDefault();
  invoke("add_note", { text: $("note").value, atUs: wallOf(nowM()) });
  $("note").value = "";
};
$("helpbtn").onclick = () => { $("helpbox").hidden = !$("helpbox").hidden; };
$("copy").onclick = () => navigator.clipboard.writeText($("id").value);
$("set").onclick = () => invoke("add_peer", { id: $("peer").value }).then(() => { $("peer").value = ""; $("notice").textContent = ""; }).catch((e) => { $("notice").textContent = e; });
$("rec").onclick = record;
$("rec2").onclick = record;
$("name").onchange = () => invoke("set_name", { name: $("name").value });
$("report").onclick = () => invoke("save_report", { includeAddresses: $("addr").checked, audioProcess: $("proc").value }).then((p) => { $("reportpath").textContent = p; }).catch((e) => { $("reportpath").textContent = String(e); });
$("checkupdate").onclick = () => checkUpdate(true);
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

document.addEventListener("keydown", (e) => {
  const typing = /^(INPUT|SELECT|TEXTAREA)$/.test(document.activeElement.tagName);
  if (e.key === "Escape") { document.activeElement.blur(); $("helpbox").hidden = true; return; }
  if (typing || e.ctrlKey || e.metaKey || e.altKey || !st) return;
  if (e.key === " ") toggle();
  else if (e.key === "ArrowLeft") jump(-5);
  else if (e.key === "ArrowRight") jump(5);
  else if (/^[0-9]$/.test(e.key)) { view = Number(e.key); applyView(); }
  else if (e.key === "n" || e.key === "N") $("note").focus();
  else if (e.key === "u" || e.key === "U") $("undo").click();
  else if (e.key === "?") $("helpbtn").click();
  else return;
  e.preventDefault();
});

setInterval(tick, 150);
setInterval(() => refresh().catch(() => {}), 1000);
setInterval(() => refreshDebug().catch(() => {}), 2000);
setInterval(() => refreshFiles().catch(() => {}), 5000);
refresh();
fillProcesses().then(runCheck);
setTimeout(() => checkUpdate(false), 4000);

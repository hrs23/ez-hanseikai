const { invoke, convertFileSrc } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const live = new Set();
const tiles = {};
const sec = (us) => us / 1e6;
let st = null;
let ready = false;

window.__TAURI__.app.getName().then((name) => {
  $("appname").textContent = name;
  document.title = name;
});

function mb(bytes) { return (bytes / 1e6).toFixed(1) + " MB"; }
function gb(bytes) { return (bytes / 1e9).toFixed(1) + " GB"; }
function label(p) { return p.name ? `${p.name} (${p.short})` : p.short; }

function tile(key) {
  if (tiles[key]) return tiles[key];
  const box = document.createElement("div");
  box.className = "tile";
  const head = document.createElement("div");
  head.className = "head";
  const title = document.createElement("span");
  head.append(title);
  const video = document.createElement("video");
  video.muted = key === "me";
  if (key !== "me") {
    const button = document.createElement("button");
    button.textContent = "Live";
    button.onclick = () => {
      live.add(key);
      if (video.duration) { video.currentTime = Math.max(0, video.duration - 1.5); video.play(); }
    };
    head.append(button);
  }
  box.append(head, video);
  $("videos").append(box);
  return (tiles[key] = { box, video, title });
}
const a = tile("me").video;

function delta(peer) {
  if (!st || !st.mine || !peer.rec) return 0;
  return sec(st.mine.start_us + (peer.offset_us || 0) - peer.rec.start_us);
}
function reload(v, rec, mode) {
  const t = v.currentTime;
  v.dataset.size = rec.size;
  v.dataset.retry = Date.now();
  v.load();
  v.onloadedmetadata = () => {
    if (mode === "live") { v.currentTime = Math.max(0, v.duration - 1.5); v.play(); }
    else v.currentTime = t;
  };
}
function source(v, rec, isLive) {
  if (!rec || !rec.size) return false;
  const src = convertFileSrc(rec.path);
  if (v.dataset.src !== src) { v.dataset.src = src; v.src = src; v.dataset.size = rec.size; return true; }
  const loaded = Number(v.dataset.size);
  const settled = v.dataset.seen === String(rec.size);
  v.dataset.seen = rec.size;
  const stuck = v.error && Date.now() - Number(v.dataset.retry || 0) > 3000;
  const grown = rec.size > loaded + 300000 && (v.paused || isLive);
  const final = settled && rec.size !== loaded && v.paused;
  if (stuck || grown || final) reload(v, rec, isLive ? "live" : "");
  return true;
}
function peerVideos() {
  return st.peers.filter((p) => tiles[p.id]).map((p) => ({ peer: p, video: tiles[p.id].video }));
}
function seekTo(pos) {
  a.currentTime = Math.max(0, pos);
  for (const { peer, video } of peerVideos()) video.currentTime = Math.max(0, pos + delta(peer));
}
function wallOf(pos) { return Math.round(st.mine.start_us + pos * 1e6); }
function posOf(wall, from) {
  const peer = st.peers.find((p) => p.id === from);
  return sec(wall - ((peer && peer.offset_us) || 0) - st.mine.start_us);
}
function apply(cmd, pos) {
  live.clear();
  const all = [a, ...peerVideos().map((x) => x.video)];
  if (cmd === "play") { seekTo(pos); all.forEach((v) => v.play()); }
  else if (cmd === "pause") { all.forEach((v) => v.pause()); seekTo(pos); }
  else if (cmd === "seek") seekTo(pos);
}
function send(cmd, pos) {
  apply(cmd, pos);
  if (st && st.mine) invoke("control", { cmd, wallUs: wallOf(pos) }).catch(() => {});
}
window.__TAURI__.event.listen("control", (e) => {
  if (st && st.mine) apply(e.payload.cmd, posOf(e.payload.wall_us, e.payload.from));
});
a.ontimeupdate = () => {
  $("seek").value = a.currentTime;
  $("time").textContent = a.currentTime.toFixed(1) + " s";
  if (a.paused) return;
  for (const { peer, video } of peerVideos()) {
    if (!video.paused && Math.abs(video.currentTime - (a.currentTime + delta(peer))) > 0.4) video.currentTime = Math.max(0, a.currentTime + delta(peer));
  }
};

function peerRow(p) {
  const row = document.createElement("div");
  row.className = "peer";
  const dot = document.createElement("span");
  dot.className = "dot " + (p.connected ? (p.path === "direct" ? "direct" : "relay") : p.failures > 2 ? "bad" : "");
  const info = document.createElement("div");
  info.className = "info";
  const name = document.createElement("div");
  name.className = "name";
  name.textContent = label(p);
  const sub = document.createElement("div");
  sub.className = "sub";
  const route = p.connected ? `${p.path === "direct" ? "direct" : "via relay"}, ${p.rtt_ms} ms` : p.failures > 2 ? "not reachable" : "connecting...";
  sub.textContent = route + (p.rec ? `, ${mb(p.rec.size)}` : "");
  info.append(name, sub);
  const hint = p.message || (!p.connected && p.failures > 2 ? "Is their app open? If Windows asked about the firewall, it must be allowed." : "");
  if (hint) {
    const h = document.createElement("div");
    h.className = "sub fail";
    h.textContent = hint;
    info.append(h);
  }
  const remove = document.createElement("button");
  remove.textContent = "Remove";
  remove.onclick = () => invoke("remove_peer", { id: p.id });
  row.append(dot, info, remove);
  return row;
}

async function refresh() {
  st = await invoke("state");
  if (!ready) { ready = true; invoke("ui_ready"); }
  $("id").value = st.id;
  if (document.activeElement !== $("name")) $("name").value = st.name;
  $("version").textContent = "v" + st.version;
  $("ffmpeg").textContent = st.ffmpeg;
  $("notice").textContent = st.notice;
  $("rec").textContent = st.recording ? "Stop" : "Record";
  $("rec").className = st.recording ? "danger" : "primary";
  $("peers").replaceChildren(...st.peers.map(peerRow));
  const free = st.free_bytes;
  const parts = [];
  if (st.mine) parts.push(`you ${mb(st.mine.size)}`);
  if (free && free < 5e9) parts.push(`low disk space: ${gb(free)} free`);
  $("recstate").textContent = parts.join(", ");
  const shown = [source(a, st.mine, false)];
  const ids = new Set(st.peers.map((p) => p.id));
  for (const key of Object.keys(tiles)) if (key !== "me" && !ids.has(key)) { tiles[key].box.remove(); delete tiles[key]; }
  tiles.me.title.textContent = "You";
  for (const p of st.peers) {
    const t = tile(p.id);
    t.title.textContent = label(p);
    shown.push(source(t.video, p.rec, live.has(p.id)));
  }
  tiles.me.box.hidden = !shown[0];
  for (const p of st.peers) tiles[p.id].box.hidden = !p.rec;
  $("empty").hidden = shown.some(Boolean);
  if (a.duration) $("seek").max = a.duration;
  $("notes").replaceChildren(...st.notes.map((n) => {
    const d = document.createElement("div");
    d.textContent = `${n.who}: ${n.text}`;
    return d;
  }));
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
  const button = document.createElement("button");
  button.textContent = "Update to " + pending.version;
  button.className = "primary";
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
  $("checks").replaceChildren(...lines.map((l) => {
    const d = document.createElement("div");
    d.className = l.level === "info" ? "muted" : l.level;
    d.textContent = l.text;
    return d;
  }));
}

async function fillProcesses() {
  const names = await invoke("list_processes");
  const current = $("proc").value;
  const first = document.createElement("option");
  first.value = "";
  first.textContent = "No game sound";
  $("proc").replaceChildren(first, ...names.map((n) => { const o = document.createElement("option"); o.textContent = n; return o; }));
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

$("play").onclick = () => send("play", a.currentTime);
$("pause").onclick = () => send("pause", a.currentTime);
$("seek").onchange = (e) => send("seek", parseFloat(e.target.value));
$("copy").onclick = () => navigator.clipboard.writeText($("id").value);
$("set").onclick = () => invoke("add_peer", { id: $("peer").value }).then(() => { $("peer").value = ""; $("notice").textContent = ""; }).catch((e) => { $("notice").textContent = e; });
$("rec").onclick = () => st && st.recording
  ? invoke("stop_recording")
  : invoke("start_recording", { audioProcess: $("proc").value }).catch((e) => { $("notice").textContent = e; });
$("addnote").onclick = () => { invoke("add_note", { text: $("note").value }); $("note").value = ""; };
$("name").onchange = () => invoke("set_name", { name: $("name").value });
$("report").onclick = () => invoke("save_report", { includeAddresses: $("addr").checked, audioProcess: $("proc").value }).then((p) => { $("reportpath").textContent = p; }).catch((e) => { $("reportpath").textContent = String(e); });
$("checkupdate").onclick = () => checkUpdate(true);
$("runcheck").onclick = () => runCheck();
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

setInterval(() => refresh().catch(() => {}), 1000);
setInterval(() => refreshDebug().catch(() => {}), 2000);
setInterval(() => refreshFiles().catch(() => {}), 5000);
refresh();
fillProcesses().then(runCheck);
setTimeout(() => checkUpdate(false), 4000);

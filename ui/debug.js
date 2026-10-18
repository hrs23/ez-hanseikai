const { invoke } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);
const FAST_MS = 1000;
const SLOW_MS = 3000;
const PERIOD = { connections: FAST_MS, network: FAST_MS, recording: FAST_MS, data: SLOW_MS, jobs: SLOW_MS, log: FAST_MS };
const ROWS_KEPT = 1500;
const ROWS_SHOWN = 500;
const EVENTS_KEPT = 500;
const LOG_LINES_KEPT = 5000;
const SPARK_POINTS = 60;
const SPARK_SIZE = [120, 18];
const LEVELS = { TRACE: 0, DEBUG: 0, INFO: 1, WARN: 2, ERROR: 3 };

let tab = "connections";
let names = {};
const prev = {};
const sparks = {};
let events = [];
let eventsSince = 0;
let rows = [];
let rowsSince = 0;
let paused = false;
let cutsSince = 0;
let cuts = [];
let frames = null;
let logLines = [];
let logFrom = 0;
let chosenSet = null;
let st = null;

function el(tag, props, ...kids) {
  const e = document.createElement(tag);
  if (props) Object.assign(e, props);
  for (const k of kids.flat()) if (k != null) e.append(k instanceof Node ? k : String(k));
  return e;
}
const td = (v, className) => el("td", { className: className || "" }, v);
function table(heads, list, row) {
  if (!list.length) return el("div", { className: "empty", textContent: "None" });
  const head = el("tr", null, heads.map((h) => el("th", { className: h.endsWith(" ") ? "num" : "", textContent: h.trim() })));
  return el("table", null, el("thead", null, head), el("tbody", null, list.map(row)));
}
function kv(target, pairs) {
  $(target).replaceChildren(...pairs.filter((p) => p).flatMap(([k, v]) => [el("div", { textContent: k }), v instanceof Node ? v : el("div", { textContent: v == null ? "" : String(v) })]));
}
const bytes = (n) => n == null ? "" : n < 1e3 ? n + " B" : n < 1e6 ? (n / 1e3).toFixed(1) + " kB" : n < 1e9 ? (n / 1e6).toFixed(1) + " MB" : (n / 1e9).toFixed(2) + " GB";
const rate = (n) => n == null ? "" : (n * 8 / 1e6).toFixed(2) + " Mbps";
const ms = (n) => n == null ? "" : n < 1000 ? Math.round(n) + " ms" : (n / 1000).toFixed(1) + " s";
const pad = (n, w = 2) => String(n).padStart(w, "0");
function clock(us) { const d = new Date(us / 1000); return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}.${pad(d.getMilliseconds(), 3)}`; }
const yes = (b) => el("span", { className: b ? "ok" : "bad", textContent: b ? "yes" : "no" });
const who = (short) => names[short] ? `${names[short]} (${short})` : short || "";
const showAddr = () => $("showaddr").checked;

function spark(points) {
  const [w, h] = SPARK_SIZE;
  const max = Math.max(1, ...points.flatMap((p) => [p[0], p[1]]));
  const line = (i) => points.map((p, x) => `${(x / (SPARK_POINTS - 1) * w).toFixed(1)},${(h - p[i] / max * (h - 2) - 1).toFixed(1)}`).join(" ");
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
  svg.setAttribute("class", "spark");
  svg.setAttribute("width", w);
  svg.setAttribute("height", h);
  for (const [i, color] of [[0, "#3dd6c6"], [1, "#fbbf24"]]) {
    const poly = document.createElementNS("http://www.w3.org/2000/svg", "polyline");
    for (const [k, v] of [["fill", "none"], ["stroke", color], ["stroke-width", "1"], ["points", line(i)]]) poly.setAttribute(k, v);
    svg.append(poly);
  }
  return svg;
}

function speeds(id, conns, now) {
  const tx = conns.reduce((a, c) => a + c.tx, 0), rx = conns.reduce((a, c) => a + c.rx, 0);
  const p = prev[id];
  prev[id] = { tx, rx, at: now };
  if (!p || tx < p.tx || rx < p.rx) return [null, null];
  const dt = (now - p.at) / 1000;
  const up = (tx - p.tx) / dt, down = (rx - p.rx) / dt;
  const list = sparks[id] || (sparks[id] = []);
  list.push([up, down]);
  if (list.length > SPARK_POINTS) list.shift();
  return [up, down];
}

function connections(v) {
  const net = v.me.net || {};
  const a = showAddr();
  kv("me", [
    ["ID", v.me.id.slice(0, 10)],
    ["Protocol", v.proto],
    ["Relay", v.me.relay_connected ? "connected" : "not connected"],
    ["UDP", `IPv4 ${net.udp_v4 ? "works" : "no"}, IPv6 ${net.udp_v6 ? "works" : "no"}`],
    ["Address per destination", net.mapping_varies_v4 == null ? "unknown" : net.mapping_varies_v4 ? "varies (direct may fail)" : "same"],
    ["Captive portal", net.captive_portal == null ? "unknown" : net.captive_portal ? "yes" : "no"],
    v.me.relay_only && ["Mode", "relay only"],
    a && ["Home relay", v.me.relay], a && ["Preferred relay", net.preferred_relay], a && ["Public IPv4", net.global_v4], a && ["Public IPv6", net.global_v6],
    a && ["Sockets", v.me.sockets], a && ["Address", v.me.addr],
  ]);
  const now = performance.now();
  for (const p of v.peers) names[p.short] = p.name;
  $("peers").replaceChildren(table(["Name", "ID", "State", "Path", "RTT ", "Up ", "Down ", "Traffic", "Lost ", "Clock offset ", "Version", "Codecs", "Live"], v.peers, (p) => {
    const live = p.conns.filter((c) => !c.closed);
    const sel = live.flatMap((c) => c.paths).find((x) => x.selected);
    const [up, down] = speeds(p.id, p.conns, now);
    const state = live.length ? `connected (${live.map((c) => c.label).join(", ")})` : p.failures ? `not connected, ${p.failures} failed` : "not connected";
    const version = p.version ? `${p.version} / ${p.proto}${p.proto !== v.proto ? " (differs)" : ""}` : "";
    return el("tr", null, td(p.name), td(p.short, "mono"), td(state, live.length ? "ok" : "bad"), td(sel ? sel.kind : "", sel && sel.kind === "relay" ? "warn" : ""),
      td(sel ? ms(sel.rtt_ms) : "", "num"), td(rate(up), "num"), td(rate(down), "num"), el("td", null, spark(sparks[p.id] || [])), td(live.reduce((n, c) => n + c.lost, 0), "num"),
      td(p.offset_us == null ? "" : (p.offset_us / 1000).toFixed(1) + " ms", "num"), td(version, p.proto !== v.proto && p.version ? "bad" : ""), td(p.codecs.join(", ")), td(p.sharing ? "sharing" : ""));
  }));
  const paths = v.peers.flatMap((p) => p.conns.flatMap((c) => c.paths.map((x) => ({ ...x, name: p.name, label: c.label, closed: c.closed }))));
  if (paths.length) $("peers").append(el("h4", { textContent: "Paths" }), table(["Peer", "Connection", "Kind", "Selected", "Remote", "RTT ", "Sent ", "Received ", "MTU "], paths, (x) =>
    el("tr", null, td(x.name), td(x.label + (x.closed ? " (closed)" : "")), td(x.kind, x.kind === "relay" ? "warn" : "ok"), td(x.selected ? "yes" : ""), td(a ? x.addr : "hidden", "mono"),
      td(ms(x.rtt_ms), "num"), td(bytes(x.tx), "num"), td(bytes(x.rx), "num"), td(x.mtu, "num"))));
  for (const e of v.events) { events.push(e); eventsSince = Math.max(eventsSince, e.t_us); }
  events = events.slice(-EVENTS_KEPT);
  $("events").replaceChildren(table(["Time", "Event"], events.slice().reverse(), (e) => el("tr", null, td(clock(e.t_us), "mono"), td(e.text, "wrap mono"))));
}

function rowMatches(r) {
  const f = $("netfilter").value.trim().toLowerCase();
  if ($("netdir").value && r.dir !== $("netdir").value) return false;
  if ($("netkind").value && r.kind !== $("netkind").value) return false;
  if ($("netresult").value && r.result !== $("netresult").value) return false;
  return !f || [who(r.peer), r.owner, r.set, r.capture, r.kind, r.result, `${r.series}/${r.n}`].join(" ").toLowerCase().includes(f);
}
const piece = (r) => r.kind === "log" ? "" : r.n === 0 ? `${r.series}/init` : `${r.series}/${r.n}`;
const resultClass = (r) => r.result === "ok" ? "ok" : r.result === "retry" || r.result === "partial" ? "warn" : "bad";
function renderRows() {
  const shown = rows.filter(rowMatches).reverse();
  $("netcount").textContent = `${shown.length} of ${rows.length}${shown.length > ROWS_SHOWN ? `, newest ${ROWS_SHOWN} shown` : ""}${paused ? ", paused" : ""}`;
  $("transfers").replaceChildren(table(["Time", "Peer", "Dir", "Kind", "Owner", "Set", "Capture", "Piece", "Bytes ", "Duration ", "Speed ", "Result"], shown.slice(0, ROWS_SHOWN), (r) =>
    el("tr", null, td(clock(r.t_us), "mono"), td(who(r.peer)), td(r.dir), td(r.kind), td(who(r.owner)), td(r.set || ""), td(r.capture || "", "mono"), td(piece(r), "mono"),
      td(bytes(r.bytes), "num"), td(ms(r.ms), "num"), td(r.ms > 0 ? rate(r.bytes / r.ms * 1000) : "", "num"), td(r.result, resultClass(r)))));
}

function network(v) {
  if (!paused) {
    rows.push(...v.rows);
    rows = rows.slice(-ROWS_KEPT);
    rowsSince = v.seq;
    renderRows();
  }
  const active = v.active.slice().sort((a, b) => b.priority - a.priority || a.ms - b.ms);
  $("active").replaceChildren(table(["Priority ", "Kind", "Dir", "Peer", "Owner", "Piece", "Bytes ", "Age "], active, (r) =>
    el("tr", null, td(r.priority, "num"), td(r.kind), td(r.dir), td(who(r.peer)), td(who(r.owner)), td(piece(r), "mono"), td(bytes(r.bytes), "num"), td(ms(r.ms), "num"))));
  const p = v.pacer;
  kv("pacer", [
    ["Speed limit", p.limit_mbps ? p.limit_mbps + " Mbps" : "none"],
    ["Booked ahead", ms(p.ahead_ms)],
    ["History booked ahead", ms(p.history_ahead_ms)],
    ["History keeps", Math.round(p.history_share * 100) + "% of the limit"],
    ["Chunk", bytes(p.chunk)],
    ["Priorities", p.priorities.map((x) => `${x.kind} ${x.priority}`).join(", ")],
  ]);
  $("messages").replaceChildren(table(["Dir", "Type", "Count ", "Bytes "], v.messages, (m) => el("tr", null, td(m.dir), td(m.kind), td(m.count, "num"), td(bytes(m.bytes), "num"))));
}

function recording(v) {
  const e = v.engine;
  kv("capture", [
    ["Source", e.source ? `${e.names.kind || e.source.kind || ""} ${e.names.source || ""}`.trim() || JSON.stringify(e.source) : "none"],
    ["Source lost", e.source_lost ? "yes" : "no"],
    ["Capturing", e.capturing ? "yes" : "no"],
    ["Wanted for room", e.want || "none"],
    ["Recording set", v.recording && v.recording.id || "none"],
    ["Frame size", e.frame_size ? e.frame_size.join("x") : ""],
    ["Last frame", e.frame_age_ms == null ? "none" : ms(e.frame_age_ms) + " ago"],
    ["App sound", `${e.audio[0] ? "capturing" : "off"}${e.names.game ? " (" + e.names.game + ")" : ""}`],
    ["Microphone", `${e.audio[1] ? "capturing" : "off"}${e.names.mic ? " (" + e.names.mic + ")" : ""}`],
    ["Status", e.status || "OK"],
    e.failures && ["Failures", `${e.failures}, retry in ${ms(e.retry_in_ms)}`],
    ["ffmpeg", `${v.ffmpeg.present ? "installed" : "missing"}${v.ffmpeg.status ? ", " + v.ffmpeg.status : ""}`],
  ]);
  const now = performance.now(), f = v.frames;
  const per = (k) => frames && now > frames.at ? ((f[k] - frames.f[k]) / ((now - frames.at) / 1000)).toFixed(1) + "/s" : "";
  kv("frames", [
    ["Arrived", `${per("arrived")}  (${f.arrived})`],
    ["Taken (cap " + v.fps + " fps)", `${per("taken")}  (${f.taken})`],
    ["Sent to encoder", `${per("encoded")}  (${f.encoded})`],
    ["Repeated (no new frame)", `${per("repeated")}  (${f.repeated})`],
    ["Taken but not encoded", Math.max(0, f.taken - (f.encoded - f.repeated))],
  ]);
  frames = { f, at: now };
  $("encoder").replaceChildren(el("div", { className: "kv" }, el("div", { textContent: "Codecs here" }), el("div", { textContent: v.mine.join(", ") }), el("div", { textContent: "Playable by all" }), el("div", { textContent: v.usable.join(", ") || "none" })),
    table(["Encoder", "Codec", "Probe", "Playable by all", "Chosen"], v.candidates, (c) =>
      el("tr", { className: c.chosen ? "sel" : "" }, td(c.name, "mono"), td(c.codec), td(c.probe, c.probe === "works" ? "ok" : "muted"), td(c.playable ? "yes" : "no", c.playable ? "" : "muted"), td(c.chosen ? "chosen: first that works and all can play" : ""))));
  const s = e.sink;
  kv("sink", s ? [
    ["Capture", s.capture], ["Size", s.size.join("x")], ["Encoder", `${s.meta.encoder} (${s.codec})`], ["Running for", ms(s.age_ms)],
    ["ffmpeg process", s.pid], ["State", s.dead ? "stopped" : s.closed ? "closing" : "running"], ["Segment", `${v.fragments_per_segment} fragments of 1 s`],
  ] : [["State", "not running"]]);
  $("series").replaceChildren(s ? table(["Series", "Last segment ", "Open segment", "Set", "Fragments ", "Bytes ", "Fragment step ", "Audio fed "], s.series, (x) =>
    el("tr", null, td(x.series === "v" ? "v (picture and app sound)" : "m (voice)"), td(x.n, "num"), td(x.open ? x.open.n : "", "mono"), td(x.open && x.open.set || (x.open ? "live only" : "")),
      td(x.open ? x.open.fragments : "", "num"), td(x.open ? bytes(x.open.size) : "", "num"), td(ms(x.step_us / 1000), "num"), td((x.pcm_frames / 48000).toFixed(1) + " s", "num"))) : el("div", { className: "empty", textContent: "No encoder running" }));
  cuts.push(...v.cuts);
  cuts = cuts.slice(-ROWS_KEPT);
  if (v.cuts.length) cutsSince = v.cuts[v.cuts.length - 1].seq;
  $("cuts").replaceChildren(table(["Time", "Capture", "Series", "Segment ", "Set", "Fragments ", "Duration ", "Bytes "], cuts.slice().reverse(), (c) =>
    el("tr", null, td(clock(c.t_us), "mono"), td(c.capture, "mono"), td(c.series), td(c.n, "num"), td(c.set || "live only"), td(c.fragments, "num"), td(ms(c.d_us / 1000), "num"), td(bytes(c.bytes), "num"))));
  $("stderr").textContent = v.stderr.map((l) => `${clock(l.t_us)} ${l.text}`).join("\n") || "No output";
}

function data(v) {
  const rooms = v.rooms.map((r) => [
    el("h4", { textContent: `Room ${r.name}${r.active ? " (open)" : ""}${r.pending ? `, ${bytes(r.pending)} waiting for its own log` : ""}` }),
    table(["Member", "ID", "Head ", "Log ", "Records by kind"], r.members, (m) =>
      el("tr", null, td(m.name), td(m.short, "mono"), td(m.head, "num"), td(bytes(m.bytes), "num"), td(Object.entries(m.kinds).map(([k, n]) => `${k} ${n}`).join(" · "), "wrap"))),
  ]);
  $("rooms").replaceChildren(...rooms.flat());
  $("sets").replaceChildren(table(["Set", "State", "Players ", "Segments "], v.sets, (s) => {
    const tr = el("tr", { className: "pick" + (s.id === chosenSet ? " sel" : "") }, td(s.id), td(s.live ? "recording" : "ended"), td(s.files, "num"), td(s.segs, "num"));
    tr.onclick = () => { chosenSet = s.id === chosenSet ? null : s.id; refreshNow(); };
    return tr;
  }));
  const d = v.detail;
  $("detail").replaceChildren(...(d ? [el("h4", { textContent: `Set ${d.id}` }), table(["Player", "Series", "Segments ", "Verified ", "Missing", "Not checked", "Bad", "Held in order by"], d.files.flatMap((f) => ["v", "m"].map((s) => [f, s, f.series[s]])), ([f, s, x]) =>
    el("tr", null, td(f.name), td(s), td(x.total, "num"), td(x.verified, "num", ), td(x.missing, "wrap warn"), td(x.unchecked, "wrap muted"), td(x.bad, "wrap bad"),
      td(Object.entries(x.have).map(([k, h]) => typeof h === "number" ? `you ${h}` : `${who(k)} ${h.n}${h.online ? "" : " (offline)"}`).join(" · "), "wrap")))] : []));
  $("bad").replaceChildren(table(["By", "Set", "File", "Size ", "Time"], v.bad, (b) =>
    el("tr", null, td(who(b.by)), td(b.record.set), td(b.record.file, "mono wrap"), td(bytes(b.record.size), "num"), td(b.record.t_us ? clock(b.record.t_us) : "", "mono"))));
  $("party").textContent = JSON.stringify(v.party, null, 2);
}

function jobs(v) {
  $("jobs").replaceChildren(table(["Job", "Plugin", "Target", "Claimed by", "State", "Progress ", "Last by", "Updated", "Records "], v.jobs, (j) =>
    el("tr", null, td(j.job, "mono"), td([j.plugin, j.version].filter((x) => x).join(" ")), td(j.file || j.set || ""), td(who(j.claimed_by)), td(j.state), td(j.progress == null ? "" : Math.round(j.progress * 100) + "%", "num"),
      td(who(j.by)), td(j.t_us > 0 ? clock(j.t_us) : "", "mono"), td(j.records, "num"))));
  $("jobrecords").replaceChildren(table(["By", "Record"], v.records, (r) => el("tr", null, td(who(r.by)), td(JSON.stringify(r.record), "mono wrap"))));
}

function levelOf(line, last) {
  const m = line.slice(0, 40).match(/\b(TRACE|DEBUG|INFO|WARN|ERROR)\b/);
  return m ? m[1] : last;
}
function renderLog() {
  const min = Number($("loglevel").value), q = $("logsearch").value.trim().toLowerCase();
  const shown = logLines.filter((l) => LEVELS[l.level] >= min && (!q || l.text.toLowerCase().includes(q)));
  $("logcount").textContent = `${shown.length} of ${logLines.length} lines`;
  $("loglines").replaceChildren(...shown.map((l) => el("div", { className: l.level.toLowerCase(), textContent: l.text })));
  if ($("follow").checked) $("loglines").lastElementChild?.scrollIntoView({ block: "end" });
}
function log(v) {
  if (v.reset) logLines = [];
  logFrom = v.len;
  if (!v.text) return;
  let last = logLines.length ? logLines[logLines.length - 1].level : "INFO";
  for (const text of v.text.replace(/\n$/, "").split("\n")) { last = levelOf(text, last); logLines.push({ level: last, text }); }
  logLines = logLines.slice(-LOG_LINES_KEPT);
  renderLog();
}

const RENDER = { connections, network, recording, data, jobs, log };
const since = () => ({ connections: eventsSince, network: rowsSince, recording: cutsSince, log: logFrom })[tab] || 0;

let gen = 0;
async function refresh() {
  if (!RENDER[tab]) return;
  const [shown, mine, addresses] = [tab, ++gen, showAddr()];
  const v = await invoke("debug_view", { tab: shown, since: since(), set: chosenSet, addresses });
  if (mine === gen && shown === tab && addresses === showAddr()) RENDER[shown](v);
}
let timer = null;
function schedule() {
  clearTimeout(timer);
  if (PERIOD[tab]) timer = setTimeout(() => (document.hidden ? Promise.resolve() : refresh()).catch(() => {}).finally(schedule), PERIOD[tab]);
}
function refreshNow() { refresh().catch(() => {}).finally(schedule); }

async function refreshNames() {
  st = await invoke("state");
  for (const [id, name] of Object.entries(st.names || {})) names[id.slice(0, 10)] = name;
}

function show(next) {
  tab = next;
  try { localStorage.setItem("debug-tab", tab); } catch {}
  document.querySelectorAll("nav button").forEach((b) => b.classList.toggle("on", b.dataset.tab === tab));
  document.querySelectorAll("section[data-tab]").forEach((s) => { s.hidden = s.dataset.tab !== tab; });
  document.querySelectorAll(".bar[data-bar]").forEach((b) => { b.hidden = b.dataset.bar !== tab; });
  if (tab === "app") app().catch(() => {});
  refreshNow();
}
document.querySelectorAll("nav button").forEach((b) => { b.onclick = () => show(b.dataset.tab); });
document.addEventListener("keydown", (e) => {
  const n = Number(e.key);
  if (e.ctrlKey && n >= 1 && n <= 8) { e.preventDefault(); show(document.querySelectorAll("nav button")[n - 1].dataset.tab); }
});

$("showaddr").onchange = () => { events = []; eventsSince = 0; logLines = []; logFrom = 0; refreshNow(); };
$("pause").onclick = () => { paused = !paused; $("pause").textContent = paused ? "Resume" : "Pause"; renderRows(); };
$("clearnet").onclick = () => { rows = []; renderRows(); };
for (const id of ["netfilter", "netdir", "netkind", "netresult"]) $(id).oninput = renderRows;
for (const id of ["loglevel", "logsearch"]) $(id).oninput = renderLog;
$("follow").onchange = renderLog;
$("clearlog").onclick = () => { logLines = []; renderLog(); };

async function report(save) {
  $("reportstatus").textContent = $("everyone").checked ? "Collecting reports from everyone..." : "Collecting...";
  try {
    const out = await invoke("debug_report", { includeAddresses: $("addr").checked, everyone: $("everyone").checked, audioProcess: st ? st.audio_process : "", save });
    if (save) { $("reportstatus").textContent = `Saved to ${out}`; return; }
    $("reporttext").textContent = out;
    await copy(out);
    $("reportstatus").textContent = "Copied";
  } catch (e) {
    $("reportstatus").textContent = String(e);
  }
}
async function copy(text) {
  try { await navigator.clipboard.writeText(text); return; } catch {}
  const area = el("textarea", { value: text });
  document.body.append(area);
  area.select();
  document.execCommand("copy");
  area.remove();
}
$("copy").onclick = () => report(false);
$("save").onclick = () => report(true);
$("addr").onchange = () => invoke("set_report_sharing", { on: $("addr").checked });

async function app() {
  await refreshNames();
  kv("info", [["Version", st.version], ["Previous run", st.previous_version || "none"], ["ID", st.id.slice(0, 10)], ["Recording codec", st.codec], ["ffmpeg", st.ffmpeg || "ready"]]);
  const info = await invoke("data_info");
  kv("datainfo", [["Folder", info.dir], ["Used", bytes(info.used_bytes)], ["Free", bytes(info.free_bytes)]]);
}
async function runCheck() {
  $("checks").replaceChildren(el("div", { className: "empty", textContent: "Checking..." }));
  const lines = await invoke("check", { audioProcess: st ? st.audio_process : "" });
  const bad = lines.filter((l) => l.level === "fail").length, warn = lines.filter((l) => l.level === "warn").length;
  $("checksummary").textContent = bad ? `Check: ${bad} problem(s)` : warn ? `Check: ${warn} warning(s)` : "Check: OK";
  $("checks").replaceChildren(table(["Level", "Result"], lines, (l) => el("tr", null, td(l.level, l.level), td(l.text, "wrap"))));
}
$("runcheck").onclick = runCheck;
$("openfolder").onclick = () => invoke("open_data_folder");

invoke("set_report_sharing", { on: false });
setInterval(() => { if (!document.hidden) refreshNames().catch(() => {}); }, SLOW_MS);
refreshNames().catch(() => {});
let saved = null;
try { saved = localStorage.getItem("debug-tab"); } catch {}
show(saved && document.querySelector(`nav button[data-tab="${saved}"]`) ? saved : "connections");

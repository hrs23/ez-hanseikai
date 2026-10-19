const METER_MIN_DB = -60;
const METER_TICKS = [-60, -40, -20, -10, 0];
const METER_YELLOW_DB = -20;
const METER_RED_DB = -9;
const PEAK_HOLD_MS = 1500;
const CLIP_HOLD_MS = 3000;

const meters = { app: { v: 0, peak: 0, at: 0, clip: 0 }, mic: { v: 0, peak: 0, at: 0, clip: 0 } };
const rates = {};
let showGone = stored("showGone", true);

function renderSide() {
  if (!inRoom()) return;
  $("roomname").classList.toggle("on", page === "room");
  renderFaces();
  renderStartButton();
  renderSessions();
  renderBanner();
  renderDock();
}
function tickSide() {
  const a = st.active_set;
  const timer = $("livetimer");
  if (a && timer) timer.textContent = fmt((nowUs() - a.start_us) / 1e6);
  if (page === "player") {
    if (a && a.voice) $("reviewrec").replaceChildren(el("i", { className: "recdot" }), t("Debrief {time}", { time: fmt((nowUs() - a.start_us) / 1e6) }));
    const chip = $("pchips").querySelector(".pill.rec");
    if (chip && a && !review && playSet() && playSet().live) chip.lastChild.textContent = t("Recording {time}", { time: fmt((nowUs() - a.start_us) / 1e6) });
  }
}

function memberDot(id) {
  if (recOf(id)) return "rec";
  if (id === st.id) return "on";
  const p = peerOf(id);
  return p && p.connected ? "on" : null;
}
function renderFaces() {
  const ids = people();
  const sig = [ids, ids.map(memberDot), st.asks.length, st.names, st.colors];
  if (!changed("faces", sig)) return;
  const faces = ids.slice(0, AVATARS_SHOWN).map((id) => {
    const dot = memberDot(id);
    const a = avatar(id, "", dot);
    if (!dot) a.classList.add("dim");
    return a;
  });
  $("faces").replaceChildren(el("span", { className: "avs" }, ...faces), st.asks.length ? el("span", { className: "badge", textContent: String(st.asks.length) }) : "");
  $("faces").title = t("Members");
}

function startState() {
  if (st.active_set) return "end";
  return reviewing() ? "review" : "start";
}
function renderStartButton() {
  const state = startState();
  if (!changed("startbtn", [state, lang])) return;
  const b = $("startbtn");
  b.className = "startbtn " + (state === "end" ? "end" : "go") + (state === "review" ? " review" : "");
  if (state === "end") b.replaceChildren(icon("stop", "sm"), el("span", { textContent: t("Stop recording") }));
  else if (state === "review") b.replaceChildren(icon("mic"), el("span", { textContent: t("Record review") }), el("span", { className: "voice", textContent: t("Voice only") }));
  else b.replaceChildren(el("i", { className: "rd" }), el("span", { textContent: t("Start recording") }));
  b.title = state === "start" ? t("Starts recording for everyone. Everything until Stop recording is one recording; matches are chapters in it.") : state === "review" ? t("Records everyone's voice and what the Watch Party watches, with no picture.") : "";
}

function noteCount(s) {
  return s.events.filter((k) => { const e = idx.byKey.get(k); return e && e.kind === "note"; }).length;
}
function matchCount(s) {
  return s.events.filter((k) => { const e = idx.byKey.get(k); return e && e.plugin && (e.tags || []).includes(MATCH_TAG); }).length;
}
function receiving(s) {
  const p = s.progress;
  if (!p.total || p.have >= p.total) { delete rates[s.id]; return null; }
  const now = Date.now();
  const r = rates[s.id] || (rates[s.id] = []);
  if (!r.length || r[r.length - 1].have !== p.have || now - r[r.length - 1].t > REFRESH_MS) r.push({ t: now, have: p.have });
  while (r.length && now - r[0].t > RATE_WINDOW_MS) r.shift();
  const speed = rateOf(r, now);
  return { n: Math.floor((p.have / p.total) * 100), stuck: p.stuck, speed, left: speed ? (p.total - p.have) / speed : null };
}
function sessionState(s) {
  const watching = watchParty && st.party.on && st.party.shared.set === s.id ? st.party.watching.filter((id) => id !== st.id).length : 0;
  const running = jobInfo && jobInfo.running;
  const runningJob = running && jobInfo.jobs.find((j) => j.job === running.job);
  if (exporting && exporting.set === s.id) return { kind: "bar", icon: "film", text: t("Exporting {n}%", { n: Math.floor(exporting.percent) }), frac: exporting.percent / 100 };
  const recv = receiving(s);
  if (recv && recv.stuck) return { kind: "line", icon: "alert", cls: "warn", text: t("Stopped at {n}%", { n: recv.n }), title: lackingText(s) };
  if (recv) return { kind: "bar", icon: "download", text: [`${recv.n}%`, recv.speed ? `${bytes(recv.speed)}/s` : null].filter(Boolean).join(" · "), right: recv.left ? leftText(recv.left) : "", frac: recv.n / 100, title: lackingText(s) };
  if (watching) return { kind: "tail", icon: "users", cls: "acc", text: String(watching + 1), title: t("{n} watching", { n: watching + 1 }) };
  if (runningJob && runningJob.set === s.id) return { kind: "tail", icon: "cpu", text: `${Math.floor((running.progress || 0) * 100)}%`, title: pluginName(running.plugin) };
  if (jobInfo && jobInfo.jobs.some((j) => j.set === s.id && j.state === "failed")) return { kind: "tail", icon: "alert", cls: "warn", text: "", title: t("Analysis failed") };
  return null;
}
function lackingText(s) {
  return (s.lacking || []).map((l) => t("{name}: {n} files left", { name: nameOf(l.id), n: l.n })).join(" · ");
}
function liveRow() {
  const a = st.active_set;
  const who = a ? (liveSet() ? liveSet().players.map((p) => p.owner) : []) : [st.id, ...st.peers.filter((p) => p.connected).map((p) => p.id)];
  const online = 1 + st.peers.filter((p) => p.connected).length;
  const on = page === "player" && showingNow() && !review;
  const row = el("div", { className: "srow liverow" + (a ? " recording" : "") + (on ? " on" : ""), role: "option", tabIndex: 0, ariaSelected: on },
    el("div", { className: "l1" }, a ? el("i", { className: "recdot" }) : icon("radio", "sm"), el("b", { textContent: a ? t("Recording") : t("Standby") }), a ? el("span", { className: "r rec", id: "livetimer", textContent: fmt((nowUs() - a.start_us) / 1e6) }) : null),
    el("div", { className: "l2" }, avatars(who), a && a.voice ? pill("accent", "mic", t("Debrief")) : null, el("span", { textContent: a ? (a.voice ? t("Voice only") : matchNow(nowM()) || t("by {name}", { name: a.by || "" })) : t("{n} online", { n: online }) })));
  row.onclick = openNow;
  row.onkeydown = (e) => { if (e.key === "Enter") openNow(); };
  return row;
}
function importedPill(s) {
  const p = pill("gray", "download", t("Imported"));
  p.title = t("Imported by {name}", { name: s.imported_by });
  return p;
}
function sessionRow(s) {
  const on = page === "player" && !showingNow() && (review ? review.id === s.id : selected === s.id);
  const state = sessionState(s);
  const matches = matchCount(s);
  const notes = noteCount(s);
  const what = isReview(s) ? pill("accent", "mic", t("Debrief")) : s.imported_by ? importedPill(s) : null;
  const detail = matches ? t(matches === 1 ? "{n} match" : "{n} matches", { n: matches }) : notes ? t(notes === 1 ? "{n} note" : "{n} notes", { n: notes }) : "";
  let l2;
  if (state && state.kind === "bar") l2 = el("div", { className: "prog", title: state.title || "" }, el("div", { className: "l2" }, icon(state.icon, "sm"), el("span", { textContent: state.text }), el("span", { className: "r", textContent: state.right || "" })), el("div", { className: "bar" }, el("i", { style: `width:${Math.round(state.frac * 100)}%` })));
  else if (state && state.kind === "line") l2 = el("div", { className: "l2 " + (state.cls || ""), title: state.title || "" }, icon(state.icon, "sm"), el("span", { textContent: state.text }));
  else l2 = el("div", { className: "l2" }, avatars(s.players.map((p) => p.owner)), what, el("span", { textContent: detail }), state ? el("span", { className: "r " + (state.cls || ""), title: state.title || "" }, icon(state.icon, "sm"), state.text) : null);
  const row = el("div", { className: "srow" + (on ? " on" : "") + (state && state.icon === "users" ? " wp" : ""), role: "option", tabIndex: 0, ariaSelected: on },
    el("div", { className: "l1" }, el("b", { textContent: sessionName(s.id) }), el("span", { className: "r", textContent: setLength(s) })), l2);
  row.onclick = () => openSession(s.id);
  row.onkeydown = (e) => { if (e.key === "Enter") openSession(s.id); };
  row.oncontextmenu = (e) => { e.preventDefault(); sessionMenu(s.id, e); };
  return row;
}
function goneRow(g) {
  const len = g.end_us ? fmt((g.end_us - g.start_us) / 1e6) : "";
  const get = el("button", { className: "sm", onclick: (e) => { e.stopPropagation(); run("restore_set", { room: g.room, set: g.id }); } }, icon("download", "sm"), t("Download"));
  return el("div", { className: "srow gone" }, el("div", { className: "l1" }, el("b", { textContent: sessionName(g.id) }), el("span", { className: "r", textContent: len })), el("div", { className: "l2" }, el("span", { textContent: t("Not on this PC") }), el("span", { className: "sp" }), get));
}
function renderSessions() {
  if (!inRoom()) return;
  const list = st.sets.filter((s) => !s.live);
  const gone = showGone ? st.gone : [];
  const rows = [...list.map((s) => ({ id: s.id, s })), ...gone.map((g) => ({ id: g.id, g }))].sort((a, b) => (a.id < b.id ? 1 : -1));
  const sig = [rows.map((r) => [r.id, r.s ? [setLength(r.s), sessionState(r.s), isReview(r.s), r.s.imported_by, r.s.title, matchCount(r.s), noteCount(r.s), r.s.players.map((p) => p.owner)] : r.g.end_us]), page, nowView, selected, review && review.id, st.active_set, st.peers.map((p) => p.connected), lang, st.colors, st.names, dayOf(new Date())];
  if (!changed("sessions", sig)) return;
  $("sessions").replaceChildren(liveRow(), ...rows.map((r) => (r.s ? sessionRow(r.s) : goneRow(r.g))), rows.length ? "" : el("div", { className: "none", textContent: t("No recordings yet") }));
}
function sessionMenu(id, anchor) {
  const set = setById(id);
  if (!set) return;
  const failed = jobInfo ? jobInfo.jobs.filter((j) => j.set === id && j.state === "failed") : [];
  openMenu([
    { heading: `${sessionName(id)} · ${t("{size} on this PC", { size: bytes(set.progress.have) })}` },
    { label: t("Export video…"), icon: "film", disabled: set.live || voiceOnly(set), run: () => exportSet(set) },
    { label: t("Export to folder…"), icon: "upload", disabled: set.live, run: () => exportFolder(set) },
    { label: t("Open folder"), icon: "folder", run: () => run("open_set_folder", { room: set.room, set: set.id }, null) },
    { label: t("Copy link"), icon: "link", run: () => navigator.clipboard.writeText(momentLink(set.id, 0, null)).then(() => flash(t("Link copied"))).catch(fail) },
    ...(failed.length ? [{ label: t("Retry analysis"), icon: "rotate-cw", run: () => Promise.all(failed.map((j) => invoke("retry_job", { job: j.job }))).then(pollPlugins).catch(fail) }] : []),
    { separator: true },
    { label: t("Delete from this PC…"), icon: "trash", danger: true, disabled: set.live, run: () => confirmDelete([set.id]) },
  ], anchor);
}
function goneMenu(anchor) {
  openMenu([{ label: t("Show recordings not on this PC"), check: showGone, run: () => { showGone = !showGone; store("showGone", showGone); delete sigs.sessions; renderSessions(); } }], anchor);
}
function renderBanner() {
  const d = st.disk;
  const box = $("diskbanner");
  box.hidden = !d.low;
  if (!d.low || !changed("banner", [d.full, d.hours_left && Math.floor(d.hours_left), lang])) return;
  const text = d.full ? t("Disk almost full · recording stops") : d.hours_left !== null ? t("Disk space low · about {h} h of recording left", { h: Math.floor(d.hours_left) }) : t("Disk space low");
  box.replaceChildren(icon("drive", "sm"), el("span", { textContent: text }));
}

function sourceLabel(c) {
  if (!c.source) return t("Screen off");
  return c.source.kind === "window" ? c.source.title : t("Display {n}", { n: c.source.index });
}
function renderDock() {
  const c = st.capture;
  const problem = captureProblem(c);
  const state = !c.source ? "off" : problem ? "warn" : st.recording && c.working ? "recording" : "on";
  const sub = { off: t("Click to choose"), warn: problem, recording: t("Recording · live"), on: c.sharing && c.live ? t("Sending live") : t("Starting…") }[state];
  if (changed("screencap", [state, sourceLabel(c), sub, lang])) {
    $("screencap").className = "cap " + state;
    $("screenicon").replaceChildren(icon(!c.source ? "monitor-off" : c.source.kind === "display" ? "monitor" : "window"));
    $("screentitle").textContent = sourceLabel(c);
    $("screensub").textContent = sub;
    $("screenbtn").title = c.source ? t("Stop sending your screen") : t("Choose what to send");
  }
  const mute = c.mix.mic.muted;
  if (changed("miccap", [mute, lang])) {
    $("miccap").className = "cap mic " + (mute ? "muted" : "on");
    $("micbtn").replaceChildren(icon(mute ? "mic-off" : "mic"));
    $("micbtn").title = mute ? t("Unmute microphone") : t("Mute microphone");
    $("micbtn").setAttribute("aria-pressed", String(!mute));
  }
  if (changed("me", [st.name, st.id, st.colors, lang])) {
    $("meav").replaceChildren(avatar(st.id, "lg", "on"));
    $("mename").textContent = st.name;
    $("meid").textContent = shortId(st.id);
  }
  const net = netState(st.peers);
  if (changed("net", [net, lang])) {
    $("netbadge").hidden = !net;
    $("netbadge").className = "net " + (net || "");
    $("netbadge").replaceChildren(icon("relay", "sm"), net === "relay" ? t("Via relay") : t("Direct"));
    $("netbadge").title = t("Where data goes");
  }
}

async function stopScreen() {
  if (st.recording && !(await ask(t("Stop sending your screen?"), t("Stop"), false, t("Your part of the recording goes on with your voice only.")))) return;
  run("set_source", { source: null });
}
function popHead(text) {
  return el("div", { className: "ph", textContent: text });
}
async function openSource(anchor) {
  const list = await invoke("list_sources").catch((e) => { fail(e); return []; });
  const names = await invoke("list_processes").catch(() => []);
  const c = st.capture;
  const current = JSON.stringify(c.source);
  const item = (s) => {
    const on = JSON.stringify(s.id) === current;
    const b = el("button", { className: "pi" + (on ? " on" : "") }, icon(s.id.kind === "display" ? "monitor" : "window"), el("span", { className: "nm", textContent: s.id.kind === "display" ? t("Display {n}", { n: s.id.index }) : s.id.title }), s.width ? el("small", { textContent: `${s.width}×${s.height}` }) : null, on ? icon("check", "sm") : null);
    b.onclick = () => { closePop(); run("set_source", { source: s.id }); };
    return b;
  };
  const wins = list.filter((s) => s.id.kind === "window");
  const disps = list.filter((s) => s.id.kind === "display");
  const proc = el("select", { ariaLabel: t("App sound") }, el("option", { value: "", textContent: t("Chosen window") }), ...names.map((n) => el("option", { value: n, textContent: n })));
  if (st.audio_process && !names.includes(st.audio_process)) proc.append(el("option", { value: st.audio_process, textContent: st.audio_process }));
  proc.value = st.audio_process;
  proc.onchange = () => run("set_audio_process", { name: proc.value });
  showPop([
    popHead(t("Windows")), ...(wins.length ? wins.map(item) : [el("div", { className: "none", textContent: t("No windows") })]),
    el("hr"), popHead(t("Displays")), ...disps.map(item),
    ...(c.source ? [el("button", { className: "pi off", onclick: () => { closePop(); stopScreen(); } }, icon("monitor-off"), el("span", { className: "nm", textContent: t("Turn screen off") }))] : []),
    el("hr"),
    el("div", { className: "mixer" }, el("label", { className: "fl" }, el("span", { textContent: t("App sound") }), proc), meterBox("app")),
  ], anchor, true, "capture");
  drawMeter("app", meters.app.v);
}
async function openMic(anchor) {
  const list = await invoke("list_mics").catch(() => []);
  const mic = el("select", { ariaLabel: t("Microphone") }, el("option", { value: "", textContent: t("Default microphone") }), ...list.map((m) => el("option", { value: m.id, textContent: m.name })));
  mic.value = list.some((m) => m.id === st.capture.mic) ? st.capture.mic : "";
  mic.onchange = () => run("set_mic", { id: mic.value });
  showPop([el("div", { className: "mixer" }, el("label", { className: "fl" }, el("span", { textContent: t("Microphone") }), mic), meterBox("mic"))], anchor, true, "capture");
  drawMeter("mic", meters.mic.v);
}
function meterBox(kind) {
  const mix = st.capture.mix[kind];
  const range = el("input", { type: "range", min: 0, max: 1, step: VOLUME_STEP, value: mix.volume, ariaLabel: kind === "app" ? t("App sound") : t("Microphone") });
  const mute = el("button", { className: "icon sm" + (mix.muted ? " warn" : ""), ariaLabel: t("Mute"), title: t("Mute") }, icon(kind === "mic" ? (mix.muted ? "mic-off" : "mic") : mix.muted ? "volume-x" : "volume-2", "sm"));
  const save = (muted) => invoke("set_mix", { kind, volume: Number(range.value), muted }).then(refresh).then(() => {
    const off = st.capture.mix[kind].muted;
    mute.replaceChildren(icon(kind === "mic" ? (off ? "mic-off" : "mic") : off ? "volume-x" : "volume-2", "sm"));
    mute.classList.toggle("warn", off);
    $(kind + "meter").classList.toggle("muted", off);
  }).catch(fail);
  range.oninput = () => { rangeFill(range); save(st.capture.mix[kind].muted); };
  mute.onclick = () => save(!st.capture.mix[kind].muted);
  rangeFill(range);
  const bar = el("div", { className: "mbar" + (mix.muted ? " muted" : ""), id: kind + "meter" }, el("i", { className: "fill" }), el("i", { className: "peak" }));
  bar.style.setProperty("--yellow", meterPos(METER_YELLOW_DB) * 100 + "%");
  bar.style.setProperty("--red", meterPos(METER_RED_DB) * 100 + "%");
  requestAnimationFrame(() => bar.style.setProperty("--full", bar.clientWidth + "px"));
  const scale = el("div", { className: "mscale" }, ...METER_TICKS.map((db) => el("span", { textContent: String(db).replace("-", "−"), style: `left:${meterPos(db) * 100}%` })));
  return el("div", { className: "meter" }, el("div", { className: "mrow" }, bar, el("span", { className: "db", id: kind + "db" })), scale, el("div", { className: "mvol" }, mute, range));
}
function meterPos(db) {
  return Math.max(0, Math.min(1, (db - METER_MIN_DB) / -METER_MIN_DB));
}
const toDb = (v) => (v > 0 ? 20 * Math.log10(v) : -Infinity);
function drawMeter(kind, v) {
  const now = performance.now();
  const m = meters[kind];
  m.v = v;
  if (v >= m.peak || now - m.at > PEAK_HOLD_MS) { m.peak = v; m.at = now; }
  if (v >= 1) m.clip = now;
  const box = $(kind + "meter");
  if (!box) return;
  box.style.setProperty("--v", meterPos(toDb(v)) * 100 + "%");
  box.style.setProperty("--p", meterPos(toDb(m.peak)) * 100 + "%");
  box.classList.toggle("clip", now - m.clip < CLIP_HOLD_MS);
  box.classList.toggle("quiet", !m.peak);
  const db = toDb(m.peak);
  $(kind + "db").textContent = db > METER_MIN_DB ? `${String(Math.round(db)).replace("-", "−")} dB` : "";
}
window.__TAURI__.event.listen("levels", (e) => { for (const kind of ["app", "mic"]) drawMeter(kind, Math.max(0, e.payload[kind] || 0)); });

$("roomname").onclick = () => go("room");
$("faces").onclick = () => { roomTab = "members"; go("room"); };
$("startbtn").onclick = toggleRecording;
$("sessmore").onclick = (e) => goneMenu(e.currentTarget);
$("diskbanner").onclick = () => openSettings("storage");
$("diskbanner").onkeydown = (e) => { if (e.key === "Enter") openSettings("storage"); };
$("screenbtn").onclick = (e) => { if (st.capture.source) stopScreen(); else openSource($("screenmore")); };
$("screenmore").onclick = (e) => openSource(e.currentTarget);
$("micbtn").onclick = () => run("set_mix", { kind: "mic", volume: st.capture.mix.mic.volume, muted: !st.capture.mix.mic.muted });
$("micmore").onclick = (e) => openMic(e.currentTarget);
$("netbadge").onclick = () => openDiagnostics("flow");
$("gear").onclick = () => openSettings();

const PEAK_HOLD_MS = 1500;
const CLIP_HOLD_MS = 3000;

const meters = { app: { v: 0, peak: 0, at: 0, clip: 0 }, mic: { v: 0, peak: 0, at: 0, clip: 0 } };
const rates = {};

let sideWidth = clampSide(stored("sideWidth", SIDE_DEFAULT));

function renderSide() {
  if (!inRoom()) return;
  renderRoomHead();
  renderFaces();
  renderNav();
  renderStartButton();
  renderSessions();
  renderBanner();
  renderDock();
}
function renderRoomHead() {
  const name = st.room.name;
  if (!changed("roomhead", [name, stamp(st.room.icon)])) return;
  $("roomicon").replaceChildren(roomIcon(st.room, "lg"));
  $("roomname").querySelector(".nm").textContent = name;
  $("roomname").title = name;
  $("roomname").setAttribute("aria-label", name);
}
function navActive() {
  if (page === "player") return !nowView && !playSet() ? "live" : "";
  return roomTab;
}
function renderNav() {
  const active = navActive();
  for (const b of document.querySelectorAll("[data-nav]")) {
    const on = b.dataset.nav === active;
    b.classList.toggle("on", on);
    if (on) b.setAttribute("aria-current", "page");
    else b.removeAttribute("aria-current");
  }
  $("askcount").hidden = !st.asks.length;
  $("askcount").textContent = String(st.asks.length);
}
function setSideWidth(px, keep = true) {
  sideWidth = clampSide(px);
  document.documentElement.style.setProperty("--side", sideWidth + "px");
  $("splitter").setAttribute("aria-valuenow", String(sideWidth));
  if (keep) store("sideWidth", sideWidth);
}
function dragSide(down) {
  down.preventDefault();
  const start = { x: down.clientX, width: sideWidth };
  const bar = $("splitter");
  bar.setPointerCapture(down.pointerId);
  bar.classList.add("drag");
  document.body.classList.add("dragging-side");
  const move = (e) => setSideWidth(start.width + e.clientX - start.x, false);
  const end = () => {
    bar.removeEventListener("pointermove", move);
    bar.removeEventListener("pointerup", end);
    bar.removeEventListener("pointercancel", end);
    bar.classList.remove("drag");
    document.body.classList.remove("dragging-side");
    setSideWidth(sideWidth);
  };
  bar.addEventListener("pointermove", move);
  bar.addEventListener("pointerup", end);
  bar.addEventListener("pointercancel", end);
}
function roomMenu(anchor) {
  openMenu([
    { label: t("Room settings…"), icon: "gear", run: openRoomSettings },
    { label: t("Copy invite code"), icon: "user-plus", run: copyInvite },
    ...(st.invites ? [{ label: t("Cancel invite"), icon: "x", run: cancelInvite }] : []),
    { separator: true },
    { label: t("Close room"), icon: "logout", run: closeRoom },
  ], anchor);
}
function navigate(name) {
  if (!inRoom()) return;
  if (name === "live") openLive();
  else showRoomTab(name);
}
function tickSide() {
  const a = st.active_set;
  const timer = $("livetimer");
  if (a && timer) timer.textContent = fmt((nowUs() - a.start_us) / 1e6);
}

function memberDot(id) {
  const online = id === st.id || !!(peerOf(id) || {}).connected;
  if (!online) return null;
  return st.recording && !recOf(id) ? "warn" : "on";
}
function renderFaces() {
  const ids = people();
  const sig = [ids, ids.map(memberDot), st.names, looks()];
  if (!changed("faces", sig)) return;
  const faces = ids.slice(0, SIDE_FACES).map((id) => {
    const dot = memberDot(id);
    const a = avatar(id, "sm", dot);
    if (!dot) a.classList.add("dim");
    return a;
  });
  $("roomfaces").replaceChildren(el("span", { className: "avs" }, ...faces), ids.length > SIDE_FACES ? el("span", { className: "more", textContent: `+${ids.length - SIDE_FACES}` }) : "");
}

function renderStartButton() {
  const state = st.active_set ? "end" : reviewing() ? "debrief" : "start";
  const voice = !!(st.active_set && st.active_set.voice);
  if (!changed("startbtn", [state, voice, lang])) return;
  const b = $("startbtn");
  b.className = "startbtn";
  if (state === "end") b.replaceChildren(el("i", { className: "recdot" }), el("span", { textContent: t(voice ? "Stop debrief" : "Stop recording") }), el("span", { className: "timer", id: "livetimer", textContent: fmt((nowUs() - st.active_set.start_us) / 1e6) }));
  else b.replaceChildren(el("i", { className: "rd" }), el("span", { textContent: state === "debrief" ? t("Record debrief") : t("Start recording") }));
}

function receiving(s) {
  const p = s.progress;
  if (!onThisPc(s) || !p.total || p.have >= p.total) { delete rates[s.id]; return null; }
  const now = Date.now();
  const r = rates[s.id] || (rates[s.id] = []);
  if (!r.length || r[r.length - 1].have !== p.have || now - r[r.length - 1].t > REFRESH_MS) r.push({ t: now, have: p.have });
  while (r.length && now - r[0].t > RATE_WINDOW_MS) r.shift();
  const speed = rateOf(r, now);
  return { n: Math.floor((p.have / p.total) * 100), stuck: p.stuck, speed, left: speed ? (p.total - p.have) / speed : null };
}
function sessionState(s) {
  if (exporting && exporting.set === s.id) return { kind: "bar", icon: "film", text: t("Exporting {n}%", { n: Math.floor(exporting.percent) }), frac: exporting.percent / 100, title: exporting.folder };
  const recv = receiving(s);
  if (recv && !recv.stuck) return { kind: "bar", icon: "download", text: progressText(recv), frac: recv.n / 100 };
  return null;
}
function liveRow() {
  const a = st.active_set;
  const watching = st.party.watching[a.id];
  const on = page === "player" && nowView;
  const detail = a.voice ? null : matchNow(nowM()) || t("by {name}", { name: a.by || "" });
  const row = el("div", { className: "srow liverow" + (on ? " on" : ""), role: "option", tabIndex: 0, ariaSelected: on },
    thumbBox(null, true),
    el("div", { className: "txt" }, el("div", { className: "l1" }, icon("rec", "sm"), el("b", { textContent: t(a.voice ? "Recording audio" : "Recording") }), watching && watching.length ? el("span", { className: "wpn", title: watching.map(nameOf).join(", ") }, avatars(watching.slice(0, 2), "xs")) : null), el("div", { className: "l2" }, a.voice ? pill("gray", "mic", t("Debrief")) : null, detail ? el("span", { textContent: detail }) : null)));
  row.onclick = openNow;
  row.onkeydown = (e) => { if (e.key === "Enter") openNow(); };
  return row;
}
function wpPill(s, here, mine, watching) {
  const faces = watching && watching.length ? avatars(watching.slice(0, 2), "xs") : null;
  const title = watching ? watching.map(nameOf).join(", ") : "";
  if (mine) return faces ? el("span", { className: "wpn", title }, faces) : null;
  const join = (e) => { e.stopPropagation(); openSession(s.id); setParty(true); };
  if (!here) return faces ? el("span", { className: "wpn", title }, faces) : null;
  if (faces) return el("button", { className: "sm wpn live", title, onclick: join }, faces, icon("users", "sm"), el("span", { textContent: t("Join") }));
  return null;
}
function wtButton(s, mine) {
  if (mine) return el("button", { className: "sm wt on", title: t("Leave the Watch Party"), ariaLabel: t("Leave the Watch Party"), ariaPressed: true, onclick: (e) => { e.stopPropagation(); setParty(false); } }, icon("users", "sm"));
  return el("button", { className: "sm wt", onclick: (e) => { e.stopPropagation(); openSession(s.id); setParty(true); } }, icon("users", "sm"), el("span", { textContent: t("Watch Party") }));
}
function debriefPill(s) {
  const list = debriefsOf(s.id);
  if (!list.length) return null;
  const p = pill("gray", "mic", String(list.length));
  p.title = t("Debriefs of this recording: {names}", { names: list.map((d) => sessionName(d.id)).join(", ") });
  return p;
}
function sessionRow(s) {
  const on = page === "player" && !showingNow() && selected === s.id;
  const here = onThisPc(s);
  const state = sessionState(s);
  const watching = st.party.watching[s.id];
  const running = runningMark(s);
  const mine = !!watching && watching.includes(st.id);
  let l2;
  if (state && state.kind === "bar") l2 = el("div", { className: "prog", title: state.title || "" }, el("div", { className: "l2" }, icon(state.icon, "sm"), el("span", { textContent: state.text })), el("div", { className: "bar" }, el("i", { style: `width:${Math.round(state.frac * 100)}%` })));
  else l2 = el("div", { className: "l2" }, el("span", { textContent: setLength(s) }), titles[s.id] ? el("span", { className: "muted", textContent: `· ${sessionWhen(s.id)}`, title: fullWhen(sessionStart(s.id)) }) : null, s.imported_by ? importedPill(s) : null, debriefPill(s), here && (mine || !(watching && watching.length)) ? wtButton(s, mine) : null);
  const row = el("div", { className: "srow" + (on ? " on" : "") + (running ? " run" : "") + (mine ? " wp" : "") + (here ? "" : " gone"), role: "option", tabIndex: 0, ariaSelected: on },
    thumbBox(s, false),
    el("div", { className: "txt" }, el("div", { className: "l1" }, el("b", { textContent: sessionName(s.id), title: fullWhen(sessionStart(s.id)) }), wpPill(s, here, mine, watching), partyFrom === s.id && watching && watching.length ? el("button", { className: "sm", textContent: t("Back"), onclick: (e) => { e.stopPropagation(); openSession(s.id); setParty(true); } }) : null), l2, whereLine(s)), running ? el("span", { className: "arun", title: running }, icon("cpu", "sm")) : null);
  row.onclick = () => { if (onThisPc(s)) openSession(s.id); };
  row.onkeydown = (e) => { if (e.key === "Enter" && onThisPc(s)) openSession(s.id); };
  row.oncontextmenu = (e) => { e.preventDefault(); sessionMenu(s.id, e); };
  return row;
}
function renderSessions() {
  if (!inRoom()) return;
  const rows = recordingRows();
  const sig = [rows.map(rowSignature), page, nowView, selected, partyFrom, st.active_set, st.active_set && st.party.watching[st.active_set.id], st.peers.map((p) => p.connected), lang, looks(), st.names];
  if (!changed("sessions", sig)) return;
  $("sessions").replaceChildren(...(st.active_set ? [liveRow()] : []), ...rows.map((r) => sessionRow(r.s)), rows.length || st.active_set ? "" : el("div", { className: "none", textContent: t("No recordings yet") }));
}
function sessionMenu(id, anchor) {
  const set = setById(id);
  if (!set) return;
  const keys = new Set(set.events);
  const hasAi = st.events.some((e) => e.source === "ai" && keys.has(e.key));
  openMenu([
    { heading: `${sessionName(id)} · ${bytes(sizeOf(set))}` },
    { label: t("Export to folder…"), icon: "upload", disabled: set.live || !onThisPc(set), run: () => exportFolder(set) },
    { label: t("Open folder"), icon: "folder", disabled: set.live || set.progress.have === 0, run: () => dialog("open_set_folder", { room: set.room, set: set.id }).catch(fail) },
    { label: t("Copy link"), icon: "link", run: () => navigator.clipboard.writeText(momentLink(set.id, 0, null)).then(() => flash(t("Link copied"))).catch(fail) },
    { label: t("Delete AI notes"), icon: "trash", disabled: !hasAi, run: () => ask(t("Delete AI notes?"), t("Delete"), true).then((yes) => yes && run("delete_ai_notes", { room: set.room, set: set.id })) },
    { separator: true },
    { label: t("Delete…"), icon: "trash", danger: true, disabled: !canDelete(set), run: () => confirmDelete([set.id]) },
  ], anchor);
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
  const { screen: state, mic: micState } = captureStates(c, problem);
  const sub = { off: t("Click to choose"), warn: problem, on: st.recording ? "" : c.sharing && c.live ? t("Standby") : t("Starting…") }[state];
  if (changed("screencap", [state, sourceLabel(c), sub, lang])) {
    $("screencap").className = "cap " + state;
    $("screenicon").replaceChildren(icon(!c.source ? "monitor-off" : c.source.kind === "display" ? "monitor" : "window"));
    $("screentitle").textContent = sourceLabel(c);
    $("screensub").textContent = $("screensub").title = sub;
    $("screenbtn").title = `${c.source && !c.source_lost ? t("Stop sending your screen") : t("Choose what to send")} (V)`;
  }
  const mute = c.mix.mic.muted;
  if (changed("miccap", [mute, micState, lang])) {
    $("miccap").className = "cap mic " + micState;
    $("micbtn").firstElementChild.replaceChildren(icon(mute ? "mic-off" : "mic"));
    $("micbtn").title = `${mute ? t("Unmute microphone") : t("Mute microphone")} (M)`;
    $("micbtn").setAttribute("aria-pressed", String(!mute));
  }
  if (changed("me", [st.name, st.id, look(st.id), lang])) {
    $("meav").replaceChildren(avatar(st.id, "lg", "on"));
    $("mename").textContent = st.name;
    $("meid").textContent = longId(st.id);
    $("meid").title = st.id;
  }
  const relay = netState(st.peers) === "relay";
  if (changed("net", [relay, lang])) {
    $("netbadge").hidden = !relay;
    $("netbadge").replaceChildren(icon("relay", "sm"), t("Via relay"));
  }
  $("apibadge").hidden = !st.api.on;
}

async function stopScreen() {
  if (st.recording && !(await ask(t("Stop sending your screen?"), t("Stop"), false))) return;
  run("set_source", { source: null });
}
function popHead(text) {
  return el("div", { className: "ph", textContent: text });
}
async function openSource(anchor) {
  const list = await invoke("list_sources").catch((e) => { fail(e); return []; });
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
  showPop([
    popHead(t("Windows")), ...(wins.length ? wins.map(item) : [el("div", { className: "none", textContent: t("No windows") })]),
    el("hr"), popHead(t("Displays")), ...disps.map(item),
    ...(c.source ? [el("button", { className: "pi off", onclick: () => { closePop(); stopScreen(); } }, icon("monitor-off"), el("span", { className: "nm", textContent: t("Turn screen off") }))] : []),
    el("hr"),
    el("div", { className: "mixer" }, el("div", { className: "fl" }, el("span", { textContent: t("App sound") })), meterBox("app")),
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
listenMain("levels", (e) => {
  for (const kind of ["app", "mic"]) drawMeter(kind, Math.max(0, e.payload[kind] || 0));
  if (st) $("miclvl").style.setProperty("--lvl", String(micLevel(Math.max(0, e.payload.mic || 0), st.capture.mix.mic.muted)));
});

$("roomname").onclick = (e) => roomMenu(e.currentTarget);
for (const b of document.querySelectorAll("[data-nav]")) b.onclick = () => navigate(b.dataset.nav);
$("startbtn").onclick = () => toggleRecording();
$("splitter").onpointerdown = dragSide;
$("splitter").ondblclick = () => setSideWidth(SIDE_DEFAULT);
$("splitter").onkeydown = (e) => {
  const step = { ArrowLeft: -SIDE_KEY_STEP, ArrowRight: SIDE_KEY_STEP }[e.key];
  if (step) { e.preventDefault(); setSideWidth(sideWidth + step); }
};
$("splitter").setAttribute("aria-valuemin", String(SIDE_MIN));
$("splitter").setAttribute("aria-valuemax", String(SIDE_MAX));
setSideWidth(sideWidth, false);
$("diskbanner").onclick = () => openSettings("storage");
$("diskbanner").onkeydown = (e) => { if (e.key === "Enter") openSettings("storage"); };
$("screenbtn").onclick = () => { if (st.capture.source && !st.capture.source_lost) stopScreen(); else openSource($("screenmore")); };
$("screenmore").onclick = (e) => openSource(e.currentTarget);
$("micbtn").onclick = () => run("set_mix", { kind: "mic", volume: st.capture.mix.mic.volume, muted: !st.capture.mix.mic.muted });
$("micmore").onclick = (e) => openMic(e.currentTarget);
$("netbadge").onclick = () => openDiagnostics();
$("apibadge").onclick = () => openSettings("api");
$("gear").onclick = () => openSettings();
$("aibtn").onclick = (e) => openAiMenu(e.currentTarget, page === "player" ? selected : null);

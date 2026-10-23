const STEP = { small: 1, normal: 5, big: 10, huge: 60 };
const VOLUME_STEP = 0.05;
const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3, 4];
const MAX_KEYED = 6;
const TICK_MS = 150;
const REFRESH_MS = 1000;
const IDLE_MS = 2500;
const OSD_MS = 1200;
const SEEKWHO_MS = 4000;
const TOAST_MS = 6000;
const UNDO_MS = 8000;
const UPDATE_CHECK_DELAY_MS = 4000;
const UPDATE_EVERY_MS = 4 * 3600 * 1000;
const UPDATE_RECHECK_MS = 10 * 60 * 1000;
const DOUBLE_CLICK_MS = 250;
const NOTE_STEP_GAP = 0.5;
const PLUGINS_MS = 5000;
const CHIME = { start: [660, 880], end: [880, 660], join: [784, 988] };
const CHIME_NOTE_S = 0.14;
const CHIME_GAIN = 0.15;
const TAG_HINTS = 5;
const DRAG_START_PX = 4;
const CODEC_PROBE = { width: 1280, height: 720, bitrate: 6000000, framerate: 30 };
const GAP = 6;
const STRIP = 26;
document.documentElement.style.setProperty("--strip", STRIP + "px");
const MINI_MS = 100;
const MINI_MIN_PX = 96;
const WIDE_PANEL = 1240;

const tileGrid = (box, strip, kind) => ({ el: box, strip, kind, tiles: {}, chips: {}, keys: [], voice: [], mode: "grid", slots: 0, view: 0, last: 1, free: [] });
const rec = tileGrid($("livetiles"), $("livevoices"), "record");
const play = tileGrid($("playtiles"), $("playvoices"), "player");
const clock = { playing: false, m: 0, at: 0 };
const positions = new Map();
const liveClock = { pos: null, rate: 1, held: false, late: [], at: 0 };
let st = null;
let idx = null;
let ready = false;
let lang = "";
let page = "player";
let nowView = false;
let nowSet = null;
let selected = null;
let held = false;
let actedAt = 0;
let rate = 1;
let volume = stored("volume", 1);
let sessionSound = stored("sessionSound", true);
let gameSound = stored("gameSound", null);
let voiceSide = known(stored("voiceSide", null), VOICE_SIDES, VOICE_SIDE_DEFAULT);
let voiceToneChoice = known(stored("voiceTone", null), VOICE_TONES, VOICE_TONE_DEFAULT);
let seekRun = null;
let muted = false;
let live = false;
let watchParty = false;
let partyFrom = null;
let partyLast = null;
let partyLayout = null;
let drag = null;
let items = [];
let idleTimer = null;
let undo = null;
let available = null;
let checkedAt = 0;
let updateText = "";
let menuReturn = null;
let popReturn = null;
let menuAnchor = null;
let exporting = null;
let importedIds = new Set();
let receive = stored("receive", {});
let sidebar = stored("sidebar", true);
let panel = stored("panel", true);
let panelOpen = false;
let quick = null;
let partyRoom = null;
let played = false;
let debriefOpen = null;
const speaking = new Set();
let plugins = [];
let pluginsGen = 0;
let dismissed = new Set();
let full = false;
let listening = false;
let restored = false;

const peerOf = (id) => st.peers.find((p) => p.id === id);
const nameOf = (id) => (id === st.id ? st.name || t("You") : st.names[id] || longId(id));
const people = () => [st.id, ...st.peers.map((p) => p.id)];
const liveSet = () => st.sets.find((s) => s.live) || null;
const inRoom = () => !!st && !!st.room && !st.room.pending;
const fail = (e) => flash(e, "error", { label: t("Report…"), run: () => openFeedback("bug", errorReport(e)) });
const copyText = (text, message) => navigator.clipboard.writeText(text).then(() => flash(message)).catch(fail);
const run = (cmd, args, then = refresh) => invoke(cmd, args).then(then).catch(fail);
const quiet = (cmd, args) => invoke(cmd, args).catch(warn);
const showingNow = () => nowView || !playSet();
const followingLive = () => live && !!playSet() && playSet().live;
const livePinned = () => followingLive() && liveClock.pos !== null;
const setById = (id) => (st && st.sets.find((s) => s.id === id)) || null;

function markVoice(e, on, off) {
  e.classList.toggle("speaking", on && !off);
  e.classList.toggle("micoff", off);
}
function markLive(id) {
  if (!st) return;
  markSpeak(document.querySelectorAll(id ? `[data-speak="${CSS.escape(id)}"]` : "[data-speak]"), speaking, st.id, st.capture.mix.mic.muted);
}

function ownVoice(box, a) {
  if (!box.dataset.speak) delete a.dataset.speak;
  return a;
}

function avatars(ids, size = "sm", max = ids.length) {
  return el("span", { className: "avs" }, ...ids.slice(0, max).map((id) => avatar(id, size, null, false, true)), ids.length > max ? el("span", { className: "more", textContent: `+${ids.length - max}` }) : null);
}

function iconButton(name, label, onclick, cls = "icon sm") {
  return el("button", { className: cls, ariaLabel: label, title: label, onclick }, icon(name, "sm"));
}

function pill(cls, iconName, text) {
  return el("span", { className: "pill " + cls }, iconName ? icon(iconName, "sm") : null, text);
}

function tile(s, key) {
  if (s.tiles[key]) return s.tiles[key];
  const tl = { key, sigs: {}, media: slot(el("video", { muted: true, preload: "auto" }), key.slice(0, 4)), mic: null, loud: -Infinity, name: el("span", { className: "nm" }), av: el("span"), imp: el("span", { className: "imp", hidden: true }), who: el("div", { className: "who" }), what: el("div", { className: "what" }), problem: el("div", { className: "problem", hidden: true }) };
  tl.snd = el("span", { className: "snd", hidden: true }, icon("volume-2"));
  tl.lab = el("div", { className: "nl" }, tl.av, tl.name, tl.imp, tl.snd, el("span", { className: "moff" }, icon("mic-off", "sm")));
  tl.pic = el("div", { className: "pic" }, tl.media.video, el("div", { className: "waitbox" }, tl.who, tl.what), tl.problem);
  tl.box = el("div", { className: "tile", tabIndex: 0 }, tl.lab, tl.pic);
  const focus = () => { const i = s.keys.indexOf(key) + 1; setView(s, s.view === i ? 0 : i); };
  if (s === rec) tl.box.dataset.speak = key;
  else tl.media.video.addEventListener("resize", () => place(play));
  tl.box.onkeydown = (e) => { if (e.key === "Enter") focus(); };
  if (s === play) {
    tl.box.onpointerdown = (e) => inkDown(tl, e);
    tl.box.onpointermove = inkMove;
    tl.box.onpointerup = (e) => { if (inkUp(e)) tl.drewAt = Date.now(); };
    tl.box.onpointercancel = tl.box.onlostpointercapture = inkUp;
    tl.box.ondblclick = (e) => { if (e.target.closest(".nl") || Date.now() - (tl.drewAt || 0) < 2 * DOUBLE_CLICK_MS) return; if (full) toggleFull(); else focus(); };
    tl.box.oncontextmenu = (e) => { e.preventDefault(); stageMenu(e, key); };
  } else {
    tl.box.onclick = (e) => { if (e.target.closest(".choose")) return; focus(); };
    tl.box.ondblclick = toggleFull;
  }
  tl.lab.onpointerdown = (e) => dragTile(s, key, e);
  s.el.append(tl.box);
  return (s.tiles[key] = tl);
}
function voiceChip(s, key) {
  if (s.chips[key]) return s.chips[key];
  const c = { key, sigs: {}, media: null, mic: null, loud: -Infinity, av: el("span"), name: el("span", { className: "nm" }) };
  c.box = el("div", { className: "vchip" }, c.av, c.name, el("span", { className: "moff" }, icon("mic-off", "sm")));
  if (s === rec) c.box.dataset.speak = key;
  s.strip.append(c.box);
  return (s.chips[key] = c);
}
function nameVoice(c, key, online = true) {
  c.name.textContent = nameOf(key);
  c.box.classList.toggle("off", !online);
  if (changed("av", [nameOf(key), look(key), online], c.sigs)) c.av.replaceChildren(ownVoice(c.box, avatar(key, "sm", null, !online)));
}
function showVoices(s, shown) {
  const boxes = s.voice.map((k) => s.chips[k].box);
  if (boxes.some((b, i) => s.strip.children[i] !== b)) s.strip.append(...boxes);
  s.strip.hidden = !shown || !s.voice.length;
  if (shown) $("stage").classList.toggle("onlyvoice", !s.keys.length && s.voice.length > 0);
}
function nameTile(tl, key) {
  tl.name.textContent = nameOf(key);
  tl.who.textContent = nameOf(key);
  tl.imp.hidden = !importedIds.has(key);
  tl.imp.textContent = t("Imported");
  if (changed("av", [nameOf(key), look(key)], tl.sigs)) tl.av.replaceChildren(ownVoice(tl.box, avatar(key, "sm")));
}
function prune(s, keys, voice = []) {
  for (const [key, tl] of Object.entries(s.tiles)) if (!keys.includes(key)) { drop(tl.media); drop(tl.mic); tl.box.remove(); delete s.tiles[key]; }
  for (const [key, c] of Object.entries(s.chips)) if (!voice.includes(key)) { drop(c.mic); c.box.remove(); delete s.chips[key]; }
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
  let mini = s === play ? miniKey() : null;
  const fit = (extra) => rects(n, mode, s.el.clientWidth, s.el.clientHeight, GAP, slots + extra, STRIP);
  let r = fit(0);
  if (mini && !r.free.length) r = fit(1);
  if (mini && r.free[0].h - STRIP < MINI_MIN_PX) [mini, r] = [null, fit(0)];
  const blanks = mini ? r.free.slice(1) : r.free;
  const at = (e, b) => Object.assign(e.style, { left: b.x + "px", top: b.y + "px", width: b.w + "px", height: b.h + "px" });
  order.forEach((key, i) => {
    const tl = s.tiles[key];
    if (!tl) return;
    at(tl.box, r.tiles[i]);
    tl.box.classList.toggle("active", key === focus);
  });
  while (s.free.length > blanks.length) s.free.pop().remove();
  while (s.free.length < blanks.length) s.free.push(s.el.appendChild(el("div", { className: "tile blank" })));
  blanks.forEach((b, i) => at(s.free[i], b));
  if (s === play) {
    showMini(mini, r.free[0]);
    if (mini) at(play.mini.box, r.free[0]);
    const all = [...r.tiles, ...r.free];
    if (all.length) {
      s.el.parentElement.style.setProperty("--pt", Math.min(...all.map((b) => b.y)) + "px");
      s.el.parentElement.style.setProperty("--pr", s.el.clientWidth - Math.max(...all.map((b) => b.x + b.w)) + "px");
    }
  }
  if (s === play && st) renderInk();
}
function miniKey() {
  const on = plugins.find((p) => p.view === "minimap" && p.on);
  const chosen = on && on.settings && on.settings.source;
  const key = st && !play.view && on && (play.keys.includes(chosen) ? chosen : soundKey(0, play.keys, st.id, gameSound));
  return key && play.tiles[key] && play.tiles[key].media.video.videoWidth ? key : null;
}
function showMini(key, b) {
  const m = play.mini || (play.mini = { gen: 0, video: null, look: { shown: null, run: 0 }, sigs: {}, canvas: el("canvas"), av: el("span"), name: el("span", { className: "nm" }) });
  if (!m.box) play.el.append((m.box = el("div", { className: "tile mini" }, el("div", { className: "nl" }, m.av, m.name), el("div", { className: "pic" }, m.canvas))));
  m.box.hidden = !key || m.look.shown === false;
  m.name.textContent = key ? `${t("Minimap")} · ${nameOf(key)}` : t("Minimap");
  if (key && changed("av", [key, nameOf(key), look(key)], m.sigs)) m.av.replaceChildren(avatar(key, "sm"));
  m.side = key ? b.h : 0;
  const v = key ? play.tiles[key].media.video : null;
  if (m.video === v) return drawMini(m);
  m.video = v;
  const gen = ++m.gen;
  m.drawn = null;
  m.look = { shown: null, run: 0 };
  m.checked = 0;
  m.box.hidden = !key;
  if (!v) return;
  const frame = () => {
    if (m.gen !== gen || !v.isConnected) return;
    drawMini(m);
    if (v.requestVideoFrameCallback) v.requestVideoFrameCallback(frame);
    else setTimeout(frame, MINI_MS);
  };
  frame();
}
function drawMini(m) {
  const v = m.video;
  const r = v && minimapRect(v.videoWidth, v.videoHeight);
  const px = Math.round(m.side * devicePixelRatio);
  const sig = r && [v.currentTime, v.videoWidth, v.videoHeight, px].join();
  if (!r || (v.paused && m.drawn === sig)) return;
  if (m.canvas.width !== px) Object.assign(m.canvas, { width: px, height: px });
  const c = m.canvas.getContext("2d");
  c.imageSmoothingQuality = "high";
  c.drawImage(v, r.x * v.videoWidth, r.y * v.videoHeight, r.w * v.videoWidth, r.h * v.videoHeight, 0, 0, px, px);
  if (v.readyState < v.HAVE_CURRENT_DATA) return;
  m.drawn = sig;
  if (v.paused || performance.now() - m.checked >= MINIMAP_LOOK.everyMs) checkMini(m, v.paused);
}
// Hides the minimap tile while its crop is clearly not a minimap (a menu, a loading or black screen, another window), with hysteresis.
function checkMini(m, now) {
  m.checked = performance.now();
  const n = MINIMAP_LOOK.size;
  const small = m.small || (m.small = Object.assign(el("canvas"), { width: n, height: n }));
  let look = null;
  try {
    const c = small.getContext("2d", { willReadFrequently: true });
    c.drawImage(m.canvas, 0, 0, n, n);
    look = minimapLook(c.getImageData(0, 0, n, n).data, n);
  } catch {}
  if (!look) return;
  const was = m.look.shown;
  m.look = minimapShown(m.look, !look.not, now);
  if (m.look.shown === was) return;
  m.box.hidden = !m.look.shown;
  if (was !== null || !m.look.shown) playerLog(`minimap ${m.look.shown ? "shown" : `hidden (${look.not})`} flat=${look.flat.toFixed(2)} dark=${look.dark.toFixed(2)} bright=${look.bright.toFixed(2)}`);
}
for (const s of [rec, play]) new ResizeObserver(() => place(s)).observe(s.el);

function setView(s, view) {
  s.view = Math.max(0, Math.min(view, s.keys.length));
  if (s.view) s.last = s.view;
  place(s);
  if (s === play) { applyAudio(); renderViews(); share(); osd("monitor", s.view ? nameOf(s.keys[s.view - 1]) : t("All"), { kind: "view" }); }
}
function tileAt(s, x, y) {
  return Object.entries(s.tiles).find(([, tl]) => { const r = tl.box.getBoundingClientRect(); return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom; });
}
function dragTile(s, key, down) {
  if (down.button !== 0) return;
  const label = down.currentTarget;
  const from = { x: down.clientX, y: down.clientY };
  let moving = false;
  let over = null;
  const mark = (target) => {
    if (over && over !== target) over.box.classList.remove("over");
    over = target;
    if (over) over.box.classList.add("over");
  };
  const move = (e) => {
    if (!moving && Math.hypot(e.clientX - from.x, e.clientY - from.y) < DRAG_START_PX) return;
    moving = true;
    const hit = tileAt(s, e.clientX, e.clientY);
    mark(hit && hit[0] !== key ? hit[1] : null);
  };
  const end = (e) => {
    label.removeEventListener("pointermove", move);
    label.removeEventListener("pointerup", end);
    label.removeEventListener("pointercancel", end);
    label.releasePointerCapture(down.pointerId);
    const target = over;
    mark(null);
    if (!moving) return;
    label.addEventListener("click", (c) => c.stopImmediatePropagation(), { once: true, capture: true });
    const hit = target && e.type === "pointerup" ? Object.entries(s.tiles).find(([, tl]) => tl === target) : null;
    if (hit) swap(s, key, hit[0]);
  };
  label.setPointerCapture(down.pointerId);
  label.addEventListener("pointermove", move);
  label.addEventListener("pointerup", end);
  label.addEventListener("pointercancel", end);
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
    if (set && watchParty && st.party.on) { partyLayout = { set: set.id, mode, slots, order: s.keys }; share(); }
    else if (set) store("layout/" + set.id, { mode, slots, order: s.keys });
  }
  place(s);
}
const fromSaved = (l) => ({ mode: (l && l.mode) || "grid", slots: (l && l.slots) || 0, order: (l && l.order) || [] });
const playLayout = (set) => fromSaved(stored("layout/" + set.id, null) || set.layout);

function recOf(id) {
  const set = liveSet();
  return !!(set && set.players.some((p) => p.owner === id && p.recording));
}
const offOf = (key) => (key === st.id ? 0 : (peerOf(key) || {}).offset_us || 0);
function liveMembers() {
  if (playSet()) {
    return players().filter((p) => play.tiles[p.key]).map((p) => {
      const tl = play.tiles[p.key];
      const last = p.file.parts[p.file.parts.length - 1] || {};
      const on = (s, part) => !!(s && part && pathOf(s.part) === pathOf(part));
      const slots = [[tl.media, last.v], [tl.mic, last.m]].filter(([s, part]) => on(s, part)).map(([s]) => [s.base, s.video]);
      return { key: p.key, avail: on(tl.media, last.v) ? liveAvail(slots) : null, ended: !p.recording };
    });
  }
  return livePlayers(st).filter((p) => rec.tiles[p.key]).map(({ key, file }) => ({ key, avail: file ? liveAvail([[rec.tiles[key].media.base, rec.tiles[key].media.video]]) : null, ended: false }));
}
function goLive() {
  live = true;
  setClock(true, Math.max(0, total() - LIVE_HEADROOM_S - LIVE_SHRINK_GAP_S));
}
function tickLive() {
  const at = performance.now();
  const on = playSet() ? !holding() && followingLive() : Object.keys(rec.tiles).length > 0;
  if (!on) {
    Object.assign(liveClock, { pos: null, rate: 1, held: false, late: [], at });
    if (clock.playing && holding()) setClock(false, nowM());
    return;
  }
  Object.assign(liveClock, liveStep(liveClock, liveMembers(), nowUs() / 1e6, (at - liveClock.at) / 1000), { at });
  if (changed("liverate", liveClock.rate)) applyAudio();
  if (livePinned()) Object.assign(clock, { playing: true, m: nowM(), at });
  if (playSet()) return;
  for (const p of livePlayers(st)) {
    const tl = rec.tiles[p.key];
    if (!tl) continue;
    tl.box.classList.toggle("late", liveClock.late.includes(p.key));
    if (!p.file) continue;
    const s = tl.media;
    feed(s, p.file, offOf(p.key), liveClock.pos === null ? Infinity : liveClock.pos * 1e6);
    const edge = s.base === null || liveClock.pos === null;
    const step = edge ? liveFollow(bufferedEnd(s.video), s.video.currentTime) : { target: liveClock.pos - s.base, playing: !liveClock.held };
    aim(s, step.target);
    pace(s, step.target, step.playing);
    s.video.playbackRate = edge ? 1 : liveClock.rate;
  }
}
function liveWhat(p) {
  const problem = p.me && captureProblem(st.capture);
  if (problem) return [el("button", { className: "pick", onclick: (e) => { e.stopPropagation(); openSource(e.currentTarget); } }, icon("monitor"), problem)];
  return [el("span", { textContent: p.relayed ? t("No picture via relay") : t("Starting…") })];
}
function renderLive() {
  const idle = page === "player" && showingNow() && !playSet();
  $("livetiles").hidden = !idle;
  if (!idle) { rec.voice = []; prune(rec, []); showVoices(rec, false); return; }
  const list = livePlayers(st);
  const lay = fromSaved(st.layout);
  rec.keys = list.filter((p) => p.picture).map((p) => p.key);
  rec.voice = list.filter((p) => !p.picture).map((p) => p.key);
  rec.mode = lay.mode;
  rec.slots = lay.slots;
  prune(rec, rec.keys, rec.voice);
  const mine = st.recording && !(st.active_set && st.active_set.voice);
  for (const p of list) {
    if (!p.picture) { nameVoice(voiceChip(rec, p.key), p.key, p.online); continue; }
    const tl = tile(rec, p.key);
    nameTile(tl, p.key);
    const problem = (p.file && p.me && captureProblem(st.capture)) || tileProblem({ recording: mine, now: nowUs(), start: p.start, voiceEnd: p.last && p.last.m, pictureEnd: p.last && p.last.v });
    if (changed("livewhat", [!!p.file, p.relayed, captureProblem(st.capture), nameOf(p.key), problem, lang], tl.sigs)) {
      tl.what.replaceChildren(...liveWhat(p));
      tl.problem.textContent = problem || "";
      tl.problem.hidden = !problem;
    }
    tl.box.classList.toggle("wait", !p.file);
    tl.box.classList.toggle("hatch", !p.file && p.relayed);
  }
  showVoices(rec, true);
  place(rec);
}

function renderShell() {
  const room = inRoom();
  $("start").hidden = room;
  $("app").hidden = !room;
  $("roompage").hidden = page !== "room";
  $("playerpage").hidden = page !== "player";
  document.body.classList.toggle("noside", !sidebar);
  document.body.classList.toggle("nopanel", !panel || (page === "player" && !playSet()));
  document.body.classList.toggle("panelopen", panelOpen && !!playSet());
  document.body.classList.toggle("full", full);
}

function captureProblem(c) {
  if (!c.source) return "";
  if (c.source_lost) return t("Capture source closed");
  if (c.status) return t("Capture stopped. Retrying…");
  if (c.stalled) return t("The captured window is not updating");
  return "";
}

function playSet() { return (st && st.sets.find((s) => s.id === selected)) || null; }
function playersOf(set) {
  return set ? set.players.map((p) => ({ key: p.owner, start: p.start_us, off: p.offset_us || 0, file: p, recording: p.recording })) : [];
}
function players() { return playersOf(playSet()); }
function base(list = players()) {
  return list.length ? Math.min(...list.map((p) => p.start - p.off)) : nowUs();
}
function received(list = players()) {
  const b = base(list);
  const ends = [];
  for (const p of list) {
    const tl = play.tiles[p.key];
    const end = p.file.end_us ? (p.file.end_us - p.off) / 1e6 : tl && wallEnd(tl.media);
    if (end) ends.push(end - b / 1e6);
  }
  return ends.length ? Math.max(...ends) : 0;
}
function total(list = players()) {
  return Math.max(received(list), list.some((p) => p.recording) ? (nowUs() - base(list)) / 1e6 : 0);
}
const nowM = () => (livePinned() ? liveClock.pos - base() / 1e6 : clock.playing && !held ? clock.m + ((performance.now() - clock.at) / 1000) * rate : clock.m);
const wallOf = (m) => Math.round(base() + m * 1e6);
const holding = () => st.recording && !played && !(st.active_set && st.active_set.voice);
function setClock(playing, m) {
  clock.playing = playing && !holding();
  clock.m = Math.max(0, m);
  clock.at = performance.now();
  tick();
}

const recv = (key) => ({ app: { volume: 1, muted: false }, voice: { volume: 1, muted: false }, ...(receive[key] || {}) });
function chooseSound(key) {
  gameSound = key;
  store("gameSound", key);
  applyAudio();
  place(play);
}
function applyAudio() {
  const away = page !== "player";
  const heard = soundKey(play.view, play.keys, st.id, gameSound);
  const level = (on, mix) => (away || muted || !on || mix.muted ? 0 : volume * mix.volume);
  const tone = voiceTone(voiceSide, voiceToneChoice, watchParty && st.party.on && !!playSet());
  [...play.keys, ...play.voice].forEach((key) => {
    const tl = play.tiles[key] || play.chips[key];
    if (!tl) return;
    const mine = recv(key);
    if (tl.snd) tl.snd.hidden = key !== heard || play.keys.length < 2;
    for (const [s, on, mix] of [[tl.media, key === heard, mine.app], [tl.mic, true, mine.voice]]) if (s) {
      setLevel(s, level(on, mix));
      s.video.playbackRate = livePinned() ? liveClock.rate : rate;
    }
    if (tl.mic && tl.mic.tap) setTone(tl.mic.tap, tone);
  });
  const pick = play.keys.length > 0;
  $("gamesound").hidden = !pick;
  if (pick && changed("gamesound", [heard, nameOf(heard), look(heard), lang])) {
    $("gamesound").replaceChildren(avatar(heard, "sm"));
    $("gamesound").title = $("gamesound").ariaLabel = t("{name} app sound is playing", { name: nameOf(heard) });
  }
  if (changed("mute", [muted || volume === 0])) $("mute").replaceChildren(icon(muted || volume === 0 ? "volume-x" : "volume-2"));
  $("volume").value = muted ? 0 : volume;
  rangeFill($("volume"));
}
function rangeFill(r) {
  r.style.setProperty("--v", ((Number(r.value) - Number(r.min)) / (Number(r.max) - Number(r.min))) * 100 + "%");
}
function mixRow(key, kind, label) {
  const mix = recv(key)[kind];
  const range = el("input", { type: "range", min: 0, max: 1, step: VOLUME_STEP, value: mix.volume, ariaLabel: `${nameOf(key)} ${label}` });
  let off = mix.muted;
  const mute = el("button", { className: "icon sm", ariaLabel: t("Mute") });
  const save = () => { receive[key] = { ...recv(key), [kind]: { volume: Number(range.value), muted: off } }; store("receive", receive); mute.replaceChildren(icon(off ? "volume-x" : "volume-2", "sm")); mute.classList.toggle("warn", off); rangeFill(range); applyAudio(); };
  range.oninput = () => { off = false; save(); };
  mute.onclick = () => { off = !off; save(); };
  save();
  return el("div", { className: "poprow" }, el("span", { textContent: label }), range, mute);
}
function openMix(anchor) {
  const keys = [...play.keys, ...play.voice];
  const heard = soundKey(play.view, play.keys, st.id, gameSound);
  const pick = (key) => el("button", { className: "icon sm" + (key === heard ? " on" : ""), title: t("Switch app sound"), ariaLabel: t("Switch app sound"), onclick: () => { chooseSound(key); openMix(anchor); } }, icon("volume-2", "sm"));
  showPop([el("div", { className: "head" }, el("b", { textContent: t("Volume per member") })), el("div", { className: "mixgrid" }, ...keys.map((key) => el("div", { className: "mixm" }, el("div", { className: "who" }, avatar(key, "sm", null, false, true), el("span", { className: "nm", textContent: nameOf(key) }), play.keys.length > 1 && !play.view && play.tiles[key] ? pick(key) : null), play.tiles[key] ? mixRow(key, "app", t("App sound")) : null, mixRow(key, "voice", t("Voice"))))), el("button", { className: "poplink", onclick: openVoiceSettings }, icon("sliders", "sm"), t("Recorded voices direction and tone…"))], anchor, true, keys.length > 2 ? "wide" : "");
}
function openVoiceSettings() {
  closePop();
  openSettings("general");
  $("voiceside").querySelector(".on").focus();
}
function showPop(children, anchor, above, cls = "") {
  const pop = $("pop");
  if (pop.hidden) popReturn = document.activeElement;
  pop.className = "pop " + cls;
  pop.replaceChildren(...children);
  openPopup(pop, anchor, above);
}

function renderPlayer() {
  renderLive();
  const set = playSet();
  const idle = !set;
  const list = players();
  const shared = watchParty && st.party.on && set && partyLayout && partyLayout.set === set.id;
  const lay = set ? (shared ? { mode: partyLayout.mode || "grid", slots: partyLayout.slots || 0, order: partyLayout.order || [] } : playLayout(set)) : fromSaved(null);
  play.voice = list.filter((p) => noScreen(p.file)).map((p) => p.key);
  play.keys = ordered(list.map((p) => p.key).filter((k) => !play.voice.includes(k)), lay.order);
  play.mode = lay.mode;
  play.slots = lay.slots;
  if (shared && partyLayout.focus !== undefined) { play.view = partyLayout.focus ? play.keys.indexOf(partyLayout.focus) + 1 : 0; delete partyLayout.focus; }
  prune(play, play.keys, play.voice);
  for (const p of list) {
    if (play.voice.includes(p.key)) nameVoice(voiceChip(play, p.key), p.key);
    else nameTile(tile(play, p.key), p.key);
  }
  place(play);
  applyAudio();
  if (page !== "player") return;
  renderHeader(set);
  renderViews();
  $("playtiles").hidden = !set;
  if (set) showVoices(play, true); else play.strip.hidden = true;
  $("toolbar").hidden = idle;
  $("views").hidden = $("tools").hidden = !set;
  renderTransport(set, idle);
}
function renderTransport(set, idle) {
  const liveSetShown = !!(set && set.live);
  const edge = liveEdge(total(), nowM());
  if (set && !set.live) live = false;
  for (const id of ["play", "back", "fwd", "addnote", "seek", "lane", "chapters", "matchchip"]) $(id).classList.toggle("nodisp", idle);
  $("rate").hidden = idle || followingLive();
  $("transport").classList.toggle("idle", idle);
  $("clocknow").parentElement.hidden = idle;
  $("live").hidden = !liveSetShown;
  const atLive = live;
  $("live").className = "livebtn " + (atLive ? "at" : "behind");
  if (changed("livebtn", [atLive, Math.round(edge.behind), lang])) $("live").replaceChildren(el("i"), atLive ? t("Latest") : t("Jump to latest"), atLive ? "" : el("span", { className: "lagt", textContent: "−" + fmt(edge.behind) }));
  $("seek").classList.toggle("onlive", liveSetShown && atLive);
  $("seek").querySelector(".grow").hidden = !liveSetShown;
  const frac = set && set.progress.total ? Math.min(1, set.progress.have / set.progress.total) : 1;
  $("seek").querySelector(".got").style.width = frac * 100 + "%";
  $("seek").querySelector(".missing").hidden = !(set && set.progress.stuck);
  $("seek").querySelector(".missing").style.left = frac * 100 + "%";
}
function viewChip(set, line) {
  return el("span", { className: "pill " + (line.kind === "away" ? "warn" : "gray") }, ...lineNodes(set, line, line.kind === "receiving" ? "spinner" : line.kind === "away" ? "alert" : "download", " · "));
}
function headerChips(set) {
  if (!set || set.live) return [];
  const line = viewLine(set, viewContext());
  return [set.imported_by ? pill("gray", "download", t("Imported by {name}", { name: set.imported_by })) : null, line ? viewChip(set, line) : null];
}
function renderHeader(set) {
  const title = !set ? t("Live") : set.live ? t("Recording") : sessionName(set.id);
  $("ptitle").textContent = title;
  $("ptitle").title = set && !set.live ? fullWhen(new Date(set.start_us / 1000)) : "";
  const chips = headerChips(set);
  const csig = chips.map((c) => c && c.outerHTML);
  if (changed("pchips", csig)) $("pchips").replaceChildren(...chips.filter(Boolean));
  renderParty(set);
  $("panelbtn").hidden = !set;
  $("panelbtn").classList.toggle("on", panelShown());
  if (changed("fullicon", [full, lang])) {
    $("full").replaceChildren(icon(full ? "minimize" : "maximize"));
    $("full").title = t(full ? "Exit full screen (Esc)" : "Player full screen (F)");
    $("full").setAttribute("aria-pressed", full);
  }
}
const watchers = (id) => (st.party.watching[id] || []).filter((w) => w !== st.id);
function renderParty(set) {
  const on = watchParty && st.party.on;
  const box = $("wpbox");
  box.hidden = !set;
  $("shared").classList.toggle("wp", !!(on && set));
  const others = set ? watchers(set.id) : [];
  const status = on ? st.party.status || [] : [];
  if (!changed("wpbox", [on, others, status, st.names, lang])) return;
  if (on) {
    box.replaceChildren(el("div", { className: "wpbar" }, icon("users", "sm"), el("span", { textContent: t("Watch Party") }), ...status.map((s) => pill("gray", "spinner", t("Catching up with {name}", { name: nameOf(s.who) }))), avatars([st.id, ...others]), el("button", { className: "sm", onclick: () => setParty(false) }, icon("logout", "sm"), t("Leave"))));
  } else {
    box.replaceChildren(el("button", { className: "wpoff", onclick: () => setParty(true) }, icon("users", "sm"), el("span", { textContent: t("Watch Party") }), others.length ? avatars(others) : null, el("span", { className: "join", textContent: t("Join") })));
  }
}
function setParty(on) {
  if (!on && playSet()) store("layout/" + selected, { mode: play.mode, slots: play.slots, order: play.keys });
  watchParty = on;
  partyFrom = null;
  delete sigs.wpbox;
  joinParty().then(refresh);
}
const panelShown = () => (innerWidth > WIDE_PANEL ? panel : panelOpen);
function renderViews() {
  const names = play.keys.map(nameOf);
  if (!changed("views", [play.view, names, lang])) return;
  const button = (i, text) => {
    const b = el("button", { className: play.view === i ? "on" : "", title: i <= MAX_KEYED ? `${text} (${i})` : text, ariaPressed: play.view === i }, i <= MAX_KEYED ? el("span", { className: "key", textContent: i }) : null, el("span", { className: "ellip", textContent: text }));
    b.onclick = () => setView(play, i);
    return b;
  };
  const layout = el("button", { className: "ic", title: t("Layout"), ariaLabel: t("Layout"), ariaHasPopup: "menu" }, icon("grid", "sm"));
  layout.onclick = () => pickLayout(play, layout);
  $("views").replaceChildren(button(0, t("All")), ...names.map((n, i) => button(i + 1, n)), layout);
}
function cycleTool() {
  ink.tool = nextInkTool(ink.tool);
  osd(ink.tool || "x", t(INK_TOOL_NAME[ink.tool || "off"]));
  renderTools();
}
function renderTools() {
  const b = $("tool");
  const text = t(INK_TOOL_TITLE[ink.tool || "off"]);
  const name = ink.tool === "laser" ? "laser" : "pen";
  press(b, !!ink.tool);
  b.title = text;
  b.setAttribute("aria-label", text);
  if (b.dataset.icon !== name) { b.dataset.icon = name; b.replaceChildren(icon(name, "sm")); }
}

function openSession(id) {
  nowView = false;
  if (id !== selected && selected && watchParty && st.party.on) {
    const from = watchers(selected).length ? selected : null;
    setParty(false);
    partyFrom = from;
  }
  if (id !== selected) openSet(id);
  go("player");
}
function followNow() {
  const l = liveSet();
  if (!l || selected === l.id) return;
  selectSet(l.id);
  nowSet = l.id;
  goLive();
}
function openNow() {
  nowView = true;
  if (!liveSet() && selected) selectSet(null);
  followNow();
  go("player");
}
function openSet(id, m) {
  selectSet(id);
  if (typeof m === "number") { setClock(false, m); share(); }
  refresh();
}
function openFromStart(id) {
  selectSet(null);
  positions.delete(id);
  openSession(id);
}
function selectSet(id) {
  if (id === selected) return;
  closeQuick(false);
  if (watchParty && st.party.on && selected && watchers(selected).length) partyFrom = selected;
  if (id === partyFrom) partyFrom = null;
  selected = id;
  clock.playing = false;
  clock.m = positions.has(id) ? Math.max(0, (positions.get(id) - base()) / 1e6) : 0;
  live = false;
  delete sigs.views;
  peek.at = null;
  $("peek").hidden = true;
  ink.live.clear();
  ink.press = null;
  play.view = 0;
  prune(play, []);
  probed.clear();
  if (id) quiet("focus_set", { set: id });
  if (watchParty && st.party.on) joinParty();
}
function stepSet(dir) {
  const list = recordingRows().map((r) => r.s);
  const i = list.findIndex((s) => s.id === selected);
  const next = i < 0 ? list[0] : list[i - dir];
  if (next) { openSession(next.id); osd("film", sessionName(next.id)); }
}

function lineNodes(set, line, iconName, sep = "") {
  const action = line.action ? [sep, el("button", { className: "sm", textContent: line.action, onclick: (e) => { e.stopPropagation(); run("download_set", { room: set.room, set: set.id }); } })] : [];
  return [icon(iconName, "sm"), line.text, ...action];
}
function whatNodes(what, line, set) {
  if (line) return lineNodes(set, line, line.kind === "away" ? "alert" : "download");
  return what === "voice" ? [icon("mic", "sm"), t("Voice only")] : [t("Waiting for {name}…", { name: what })];
}
function tick() {
  if (!st) return;
  tickLive();
  const list = players();
  const tot = total(list);
  const set = playSet();
  let m = nowM();
  if (clock.playing && tot && m >= tot) { clock.playing = false; clock.m = tot; m = tot; }
  const b = base(list);
  const away = page !== "player";
  const wall = b + m * 1e6;
  if (page === "player") inkTick(m);
  if (set) positions.set(set.id, wall);
  const pinned = livePinned();
  const todo = [];
  const now = Date.now();
  for (const p of list) {
    const tl = play.tiles[p.key] || play.chips[p.key];
    if (!tl) continue;
    feedParts(tl, p, m, b);
    if (tl.mic) {
      tl.loud = clock.playing && !away ? talking(tl.loud, peakOf(tl.mic.tap), now) : -Infinity;
      markVoice(tl.box, stillTalking(tl.loud, now), recv(p.key).voice.muted);
    }
    if (tl.media) {
      const end = wallEnd(tl.media);
      const pictured = !!(partAt(p, wall) || {}).v;
      const behind = !pinned && !!p.file.total && p.file.have < p.file.total && end !== null && wall / 1e6 > end;
      tl.box.classList.toggle("wait", !tl.media.part || !pictured || behind);
      tl.box.classList.toggle("late", !!tl.media.part && pinned && liveClock.late.includes(p.key));
      const line = !pictured || behind ? viewLine(set, viewContext(), p.key) : null;
      const what = line ? line.text : !pictured ? "voice" : nameOf(p.key);
      if (changed("what", [what, line && line.action, nameOf(p.key), lang], tl.sigs)) tl.what.replaceChildren(...whatNodes(what, line, set));
    }
    for (const s of [tl.media, tl.mic]) if (s && s.base !== null) todo.push([s, wall / 1e6 - s.base, clock.playing && !away && !(pinned && liveClock.held)]);
  }
  const ready = todo.map(([s, at]) => aim(s, at));
  hold(!pinned && !away && todo.map(([s, at], i) => waiting(s, ready[i], at, now)).includes(true));
  for (const [s, at, playing] of todo) pace(s, at, playing && !held);
  if (peek.at) {
    feedSeries(peek.slot, peek.at.p, "v", peek.at.wall);
    if (peek.slot.base !== null) aim(peek.slot, peek.at.wall / 1e6 - peek.slot.base);
  }
  if (st.active_set) tickSide();
  if (away) return;
  $("clocknow").textContent = fmt(m);
  $("clocktotal").textContent = ` / ${fmt(tot)}`;
  const frac = tot ? Math.min(1, m / tot) : 0;
  $("seek").querySelector(".fill").style.width = frac * 100 + "%";
  $("seek").querySelector(".thumb").style.left = frac * 100 + "%";
  $("seek").querySelector(".grow").style.left = (tot ? Math.min(1, received(list) / tot) : 1) * 100 + "%";
  $("seek").classList.toggle("paused", !clock.playing);
  $("seek").setAttribute("aria-valuemax", Math.round(tot));
  $("seek").setAttribute("aria-valuenow", Math.round(m));
  $("seek").setAttribute("aria-valuetext", fmt(m));
  if (changed("playicon", [clock.playing, lang])) {
    $("play").replaceChildren(icon(clock.playing ? "pause" : "play"));
    $("play").setAttribute("aria-label", clock.playing ? t("Pause") : t("Play"));
    $("play").title = clock.playing ? t("Pause (Space)") : t("Play (Space)");
  }
  renderMoment(m);
}

function hideAfter(box, ms, then) {
  clearTimeout(box.hideTimer);
  box.hideTimer = setTimeout(() => { box.hidden = true; if (then) then(); }, ms);
}
let osdTimer = null;
function osd(name, text, o = {}) {
  const box = $("osd");
  if (o.kind !== "seek") seekRun = null;
  $("osdicon").replaceChildren(o.who ? avatar(o.who, "lg") : name ? icon(name) : "");
  $("osdtext").textContent = text;
  $("osdbar").hidden = o.bar === undefined;
  if (o.bar !== undefined) $("osdbar").firstElementChild.style.width = `${Math.round(Math.max(0, Math.min(1, o.bar)) * 100)}%`;
  box.classList.add("on");
  clearTimeout(osdTimer);
  osdTimer = setTimeout(() => { box.classList.remove("on"); seekRun = null; }, OSD_MS);
}
function renderSpeed() {
  const badge = $("speedbadge");
  badge.hidden = rate === 1;
  if (!badge.hidden) badge.replaceChildren(icon("gauge"), osdSpeed(rate));
}
function seekWho(who, m) {
  const box = $("seekwho");
  const tot = total();
  if (!tot) return;
  box.replaceChildren(avatar(who, "sm"));
  box.style.left = Math.max(0, Math.min(100, (m / tot) * 100)) + "%";
  box.style.setProperty("--c", colorOf(colorIndex(who)));
  box.hidden = false;
  hideAfter(box, SEEKWHO_MS);
}
function setUndo(fn) {
  undo = { fn, at: Date.now() };
}
function runUndo() {
  if (!undo || Date.now() - undo.at > UNDO_MS) { osd("info", t("Nothing to undo")); return; }
  const u = undo;
  undo = null;
  u.fn();
}
function apply(cmd, m, who, prev = { playing: clock.playing, m: nowM() }) {
  live = false;
  if (cmd === "play") setClock(true, m);
  else if (cmd === "pause") setClock(false, m);
  else if (cmd === "seek") setClock(clock.playing, m);
  else return;
  if (cmd !== "seek" && who === st.id) osd(cmd, t(cmd === "play" ? "Play" : "Pause"));
  if (who !== st.id) {
    const name = nameOf(who);
    const where = [fmt(m), matchNow(m)].filter(Boolean).join(" · ");
    osd(null, cmd === "play" ? t("{name} played", { name }) : cmd === "pause" ? t("{name} paused", { name }) : t("{name} moved to {time}", { name, time: where }), { who });
    seekWho(who, m);
  }
  setUndo(() => act(prev.playing ? "play" : "pause", prev.m));
  renderInk();
}
function view() {
  const set = playSet();
  return { set: set ? set.id : null, playing: clock.playing, live: followingLive(), wall_us: wallOf(nowM()), at_us: nowUs(), rate, focus: play.view ? play.keys[play.view - 1] : null, mode: play.mode, slots: play.slots, order: play.keys };
}
let partySends = Promise.resolve();
function share() {
  if (!watchParty || !st || !st.party.on) return;
  const args = { view: view() };
  partySends = partySends.then(() => invoke("party_act", args)).catch(fail);
}
function joinParty() {
  partyLast = null;
  partyLayout = null;
  const args = { on: watchParty, view: view() };
  partySends = partySends.then(() => invoke("party_on", args)).catch(fail);
  return partySends;
}
function applyParty(p) {
  const first = !partyLast;
  partyLast = p;
  const mine = p.origin === st.id;
  if (mine && p.stamp < actedAt) return;
  if (quick && !mine) quick.moved = true;
  const set = playSet();
  if (!set || p.set !== set.id) return;
  partyLayout = { set: p.set, mode: p.mode, slots: p.slots, order: p.order, focus: p.focus };
  if (p.rate && p.rate !== rate) setRate(p.rate);
  if (typeof p.wall_us !== "number") return;
  if (p.live && set.live) { if (!live) goLive(); return; }
  const move = partyMove(p, { base: base(), now: nowUs(), rate, m: nowM(), playing: clock.playing, quiet: mine || first || p.seq === 0 });
  if (move && move.quiet) setClock(p.playing, move.m);
  else if (move) apply(move.cmd, move.m, p.origin);
}
function act(cmd, m, prev) {
  actedAt = nowUs();
  if (!playSet()) return;
  if (cmd === "play") played = true;
  apply(cmd, m, st.id, prev);
  share();
}
listenMain("party", () => refresh());
const togglePlay = () => act(clock.playing ? "pause" : "play", !clock.playing && !playSet()?.live && total() && nowM() >= total() ? 0 : nowM());
const seekTo = (m) => act("seek", Math.max(0, m));
function seekEdge(end) {
  if (end && playSet().live) { goLive(); share(); renderPlayer(); return; }
  seekTo(end ? total() : 0);
  osd(end ? "rotate-cw" : "rotate-ccw", fmt(nowM()), { kind: "seek", bar: end ? 1 : 0 });
}
function jump(delta) {
  const run = seekSum(seekRun, delta);
  seekTo(Math.min(total(), nowM() + delta));
  osd(delta < 0 ? "rotate-ccw" : "rotate-cw", osdSeek(run), { kind: "seek", bar: total() ? nowM() / total() : undefined });
  seekRun = run;
}

function indexOf(state) {
  const byKey = new Map(state.events.map((e) => [e.key, e]));
  return { byKey, items: new Map() };
}
const setLength = (s) => fmt(((s.end_us || nowUs()) - s.start_us) / 1e6);
const pluginName = (id, stored) => t((plugins.find((p) => p.id === id) || {}).name || stored || id);
const viewContext = () => ({ me: st.id, name: nameOf, online: (id) => id === st.id || sendsPicture(peerOf(id)) });
function itemOf(e, set) {
  const r = role(e);
  const at = (us) => (us === null || us === undefined || !set ? null : (us - set.start_us) / 1e6);
  return { ...e, role: r, setId: set ? set.id : null, m: at(e.local_us), end: at(e.end_us), who: e.plugin ? pluginName(e.plugin.id, e.plugin.name) : nameOf(e.author), body: e.kind === "mark" ? t("Marked") : eventText(e), speaker: e.file ? nameOf(e.file) : nameOf(e.author) };
}
function setItems(set) {
  if (!set) return [];
  if (!idx.items.has(set.id)) {
    const keys = new Set(set.events);
    idx.items.set(set.id, labelMatchEnds(st.events.filter((e) => keys.has(e.key)).map((e) => itemOf(e, set))));
  }
  return idx.items.get(set.id);
}
const listed = (r) => r === "moment" || r === "chapter";
const matchesNow = (tot, live) => matchChapters(matchEnds(items), tot, live, matchStarts(items));
function chapters(tot) {
  const set = playSet();
  const matchList = matchesNow(tot, !!(set && set.live)).map((c) => ({ key: "match" + c.n, m: c.m, end: c.end, body: matchLabel(c.n), match: true, n: c.n }));
  return [...matchList, ...chaptersOf(items)].sort((a, b) => a.m - b.m);
}
function matchNow(m) {
  const c = matchesNow(total(), !!(playSet() && playSet().live)).find((x) => x.m <= m && m < x.end);
  return c ? matchLabel(c.n) : "";
}
function renderEvents() {
  const set = playSet();
  items = setItems(set);
  renderTimeline(Math.round(total()));
  renderReview(items.filter((n) => listed(n.role)).sort((x, y) => (x.m ?? -1) - (y.m ?? -1)));
  renderAnalysis(set);
  renderDebriefs(set);
}
function renderTimeline(tot) {
  const list = chapters(tot);
  if (changed("chapters", [list.map((n) => [n.key, n.m, n.end, n.body]), tot])) renderChapters(list, tot);
  if (changed("lane", [items.filter((n) => n.role === "moment").map((n) => [n.key, n.m, n.body, n.author, n.kind]), tot, st.colors])) renderLane(tot);
}
function renderReview(shown) {
  const set = playSet();
  if (!set || $("reviewlist").querySelector("input")) return;
  if (chip && chip.set !== selected) chip = null;
  const found = matchStarts(items);
  const list = chip ? [] : matchesNow(total(), !!set.live).filter((c) => !found.includes(c.m));
  if (!changed("review", [shown, list, lang, selected, st.colors, st.names, plugins.map((p) => p.name), chip])) return;
  shown = shown.filter((n) => chipMatches(n, chip));
  const starts = list.map((c) => ({ m: c.m, row: clickable(el("div", { className: "ev sys" }, el("span"), el("button", { className: "at", textContent: fmt(c.m), onclick: () => seekTo(c.m) }), el("span", { className: "tx" }, icon("flag", "sm"), matchLabel(c.n))), () => seekTo(c.m)) }));
  const rows = [...shown.map((n) => ({ m: n.m ?? -1, row: eventRow(n) })), ...starts].sort((a, b) => a.m - b.m).map((r) => r.row);
  const bar = chip ? [el("div", { className: "chipbar" }, el("button", { type: "button", className: "tag on", textContent: (chip.kind === "tag" ? tagName(chip.value) : "@" + chip.value) + " ×", onclick: () => pickChip(chip.kind, chip.value, chip.ids) }))] : [];
  $("reviewlist").replaceChildren(...bar, ...(rows.length ? rows : [el("div", { className: "none", textContent: t("No notes yet") })]));
  for (const b of $("reviewlist").querySelectorAll(".ev .tag")) b.classList.toggle("on", chipIs(chip, b.dataset.kind, b.dataset.value));
}
function sectionHead(name, title, n) {
  return el("div", { className: "phd" }, icon(name, "sm"), el("span", { textContent: title }), n ? el("span", { className: "pill gray", textContent: String(n) }) : null);
}
function progressBar(a) {
  return typeof a.progress === "number" ? el("div", { className: "bar" }, el("i", { style: `width:${Math.floor(a.progress * 100)}%` })) : null;
}
function analysisMenu(set, a, e) {
  const info = () => ({ name: pluginName(a.plugin), version: (plugins.find((p) => p.id === a.plugin) || {}).version, who: nameOf, runner: (plugins.find((p) => p.id === a.plugin) || {}).runner, recording: sessionName(set.id), finished: (us) => formatWhen(new Date(us / 1000), new Date(), lang), results: items.filter((n) => n.plugin && n.plugin.id === a.plugin).length });
  const run = {
    details: { label: t("Details"), icon: "info", run: () => showDetails(a, analysisDetails(a, info())) },
    again: { label: t("Run again"), icon: "rotate-cw", run: () => retryAnalysis(set, a.plugin) },
    copy: { label: t("Copy error"), icon: "copy", run: () => copyText(a.error, t("Copied")) },
  };
  openMenu(analysisActions(a).map((x) => ({ ...run[x.id], disabled: x.disabled })), e);
}
function showDetails(a, rows) {
  $("detailbody").replaceChildren(...rows.flatMap((r) => [el("dt", { textContent: r.label }), el("dd", {}, r.key === "runner" ? nameChip(avatar(plugins.find((p) => p.id === a.plugin).runner, "sm", null, false, true), r.value) : r.value)]));
  $("detailcopy").onclick = () => copyText(analysisDetailText(rows), t("Copied"));
  $("detaildlg").showModal();
}
function renderAnalysis(set) {
  const list = set ? set.analysis || [] : [];
  $("analysis").hidden = !list.length;
  if (!changed("analysis", [list, plugins.map((p) => [p.id, p.name, p.runner, p.version]), st.names, lang, items.length])) return;
  $("analysis").replaceChildren(sectionHead("cpu", t("Analysis"), list.length), ...list.map((a) => {
    const runner = (plugins.find((p) => p.id === a.plugin) || {}).runner;
    const pill = analysisPill(a);
    const why = [analysisState(a, runner, nameOf), a.error].filter(Boolean).join("\n");
    const row = el("div", { className: "prow " + a.state, tabIndex: 0, title: why, oncontextmenu: (e) => { e.preventDefault(); analysisMenu(set, a, e); } },
      el("span", { className: "pic" }, icon(a.state === "done" ? "check" : a.state === "failed" ? "alert" : a.state === "running" ? "spinner" : "cpu", "sm")),
      el("span", { className: "nm", textContent: pluginName(a.plugin) }),
      el("span", { className: "st " + pill.tone, textContent: pill.text }),
      a.state === "failed" || a.state === "idle" ? el("button", { className: "sm", textContent: t(a.state === "idle" ? "Analyze" : "Retry"), onclick: () => retryAnalysis(set, a.plugin) }) : null,
      progressBar(a));
    return row;
  }));
}
function renderDebriefs(set) {
  const list = set ? debriefsOf(set.id) : [];
  $("debriefs").hidden = !list.length;
  if (!changed("debriefs", [list.map((d) => [d.id, d.end_us, onThisPc(d)]), lang])) return;
  $("debriefs").replaceChildren(sectionHead("mic", t("Debriefs"), list.length), ...list.map((d) => el("div", { className: "prow", tabIndex: 0, oncontextmenu: (e) => { e.preventDefault(); debriefMenu(d, e); } },
    el("span", { className: "pic" }, icon("mic", "sm")),
    el("span", { className: "nm", textContent: t("{m} min", { m: Math.max(1, Math.round(((d.end_us || nowUs()) - d.start_us) / 6e7)) }) }),
    el("span", { className: "st", textContent: sessionWhen(d.id), title: fullWhen(sessionStart(d.id)) }),
    downloadButton(d) || el("button", { className: "sm", textContent: t("Save"), onclick: () => exportFolder(d) }))));
}
function debriefMenu(d, at) {
  const here = onThisPc(d);
  openMenu([
    { label: t("Export as video…"), icon: "film", disabled: !here, run: () => exportFolder(d, true) },
    { label: t("Save voices…"), icon: "mic", disabled: !here, run: () => exportFolder(d) },
  ], at);
}
function timeText(n) {
  if (n.m === null) return "";
  return n.end !== null && n.role === "chapter" ? `${fmt(n.m)}–${fmt(n.end)}` : fmt(n.m);
}
function goTo(n) {
  if (n.m === null) return;
  if (n.setId !== selected) { openSession(n.setId); openSet(n.setId, n.m); }
  else seekTo(n.m);
}
let chip = null;
function pickChip(kind, value, ids) {
  if (boardWindow) { window.__TAURI__.event.emit("chip", { kind, value, ids }).catch(warn); return; }
  const here = page === "player" && !!playSet();
  const id = here ? selected : selected || (recordingRows()[0] || {}).id;
  if (!id) return;
  if (!here) openSession(id);
  chip = nextChip(chip, kind, value, ids, id, here);
  if (chip) showPanel(true);
  renderEvents();
  $("reviewlist").scrollTop = 0;
}
listenMain("chip", (e) => pickChip(e.payload.kind, e.payload.value, e.payload.ids));
function eventRow(n) {
  const at = el("button", { className: "at", textContent: timeText(n), disabled: !n.seekable || n.m === null });
  at.onclick = () => goTo(n);
  const sys = !!n.plugin || n.kind === "mark";
  const tags = n.kind === "note" ? [] : shownTags(n).map((tg) => chipEl("tag", tg, tagName(tg)));
  const tx = el("span", { className: "tx" }, sys ? icon("flag", "sm") : null, ...inline(n.body, n.mentions), n.source === "ai" ? el("span", { className: "aimark", textContent: t("AI") }) : null, ...tags);
  const by = el("span", { className: "by", title: n.who }, authorOf(n).kind === "extension" ? icon("puzzle", "sm") : avatar(n.author, "xs", null, false, true));
  tx.prepend(el("span", { className: "nm", textContent: n.who }));
  const row = el("div", { className: "ev" + (sys ? " sys" : "") + (n.role === "chapter" ? " chapter" : "") }, by, at, tx);
  if (n.seekable && n.m !== null) clickable(row, () => goTo(n));
  row.oncontextmenu = (e) => {
    e.preventDefault();
    const label = { go: "Go to this moment", copy: "Copy text", link: "Copy link to this moment", edit: "Edit", delete: "Delete" };
    const icons = { go: "play", copy: "copy", link: "link", edit: "edit", delete: "trash" };
    const run = {
      go: () => goTo(n),
      copy: () => copyText(n.text, t("Copied")),
      link: () => copyText(momentLink(n.setId, n.m, null), t("Link copied")),
      edit: () => editNote(n, row),
      delete: () => deleteNote(n),
    };
    openMenu(rowActions(n).map((a) => ({ label: t(label[a]), icon: icons[a], run: run[a], danger: a === "delete" })), e);
  };
  if (n.mine && n.kind !== "mark") {
    const acts = el("span", { className: "acts" });
    acts.append(iconButton("edit", t("Edit"), () => editNote(n, row)), iconButton("trash", t("Delete"), () => deleteNote(n)));
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
    c.style.left = `${Math.max(0, (n.m / tot) * 100)}%`;
    c.style.width = `${Math.max(0, ((Math.min(n.end, tot) - n.m) / tot) * 100)}%`;
    c.dataset.m = n.m;
    c.dataset.end = n.end;
    c.dataset.match = n.match ? "1" : "";
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
    if (on && c.dataset.match) match = c.textContent;
  }
  $("matchchip").hidden = !match;
  if (match && changed("matchchip", [match])) $("matchchip").replaceChildren(el("span", { textContent: "· " + match }), icon("chevron-right", "sm"));
  const notes = activeAt(items.filter((n) => n.role === "moment" && n.text && !n.plugin), m).slice(-OVERLAY_MAX);
  const typists = (watchParty && st.party.on ? st.party.typing || [] : []).filter((id) => id !== st.id);
  if (changed("overlay", [notes.map((n) => [n.key, n.m, n.body, n.author]), typists.map(nameOf), st.colors, lang])) $("overlay").replaceChildren(
    ...notes.map((n) => el("p", {}, avatar(n.author, "sm", null, false, true), el("span", {}, ...inline(n.body, n.mentions)))),
    ...typists.map((id) => el("p", { className: "typing", textContent: t("{name} is typing…", { name: nameOf(id) }) })));
}
function openLink(link) {
  if (!setById(link.set)) { flash(t("That recording is not on this PC.")); return; }
  openSession(link.set);
  openSet(link.set, link.m);
  if (link.view) setTimeout(() => setView(play, play.keys.indexOf(link.view) + 1), TICK_MS);
}
function editNote(n, row) {
  const input = el("input", { value: n.text, maxLength: LIMITS.note, ariaLabel: t("Edit") });
  let done = false;
  const close = (save) => {
    if (done) return;
    done = true;
    const text = cleanText(input.value, LIMITS.note);
    if (save && text && text !== n.text) run("event_action", { kind: "edit", id: n.id, text });
    input.remove();
    delete sigs.review;
    renderEvents();
  };
  input.onkeydown = (e) => {
    e.stopPropagation();
    const act = editKey(e);
    if (act) { e.preventDefault(); close(act === "save"); }
  };
  input.onblur = () => close(true);
  row.replaceChildren(input, el("span", { className: "acts" }, cancelButton(() => close(false))));
  input.focus();
}
function stepTo(times, dir) {
  const next = stepTarget(times, nowM(), dir, NOTE_STEP_GAP);
  if (next !== undefined) seekTo(next);
  else osd("info", dir > 0 ? t("No next one") : t("No previous one"));
}
const stepNote = (dir) => stepTo(noteMoments(items).map((n) => n.m), dir);
function stepMatch(dir) {
  const ends = matchEnds(items);
  if (ends.length) { stepTo(matchTargets(ends), dir); return; }
  stepTo(items.filter((n) => ((n.role === "moment" && n.kind !== "note") || n.role === "chapter") && n.m !== null).map((n) => n.m), dir);
}
function matchMenu(anchor) {
  const list = chapters(total()).filter((c) => c.match);
  openMenu(list.map((c) => ({ label: c.body, right: `${fmt(c.m)}–${fmt(c.end)}`, run: () => seekTo(c.m) })), anchor);
}
function speedMenu(anchor) {
  openMenu(SPEEDS.map((s) => ({ label: `${s}×`, check: s === rate, run: () => { setRate(s); share(); } })), anchor);
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
  $("quicktime").textContent = quick.timed ? fmt(quick.m) : t("Whole recording");
  const box = $("quicktext");
  const hint = hintAt(box.value, box.selectionStart, [...new Set(st.events.flatMap((e) => e.tags || []))], memberNames(), TAG_HINTS);
  hintIndex = Math.min(hintIndex, Math.max(0, hint.items.length - 1));
  $("taghints").replaceChildren(...hint.items.map((v, i) => el("button", { type: "button", className: "tag" + (i === hintIndex ? " on" : ""), textContent: hint.trigger + v, title: hint.trigger + v, onmousedown: (e) => e.preventDefault(), onclick: () => pickHint(hint, v) })));
  $("taghints").hidden = !hint.items.length;
  return hint;
}
let hintIndex = 0;
function pickHint(hint, value) {
  const box = $("quicktext");
  const r = applyHint(box.value, box.selectionStart, hint, value);
  box.value = r.text;
  box.setSelectionRange(r.caret, r.caret);
  hintIndex = 0;
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
  const text = cleanText($("quicktext").value, LIMITS.note);
  const q = quick;
  if (!text) { closeQuick(true); return; }
  $("quicktext").value = "";
  closeQuick(true);
  run("add_event", { kind: "note", text, target: q.timed ? { t_us: q.wall } : { set: q.set }, values: null }, () => { osd("note", q.timed ? t("Note added at {time}", { time: fmt(q.m) }) : t("Note added")); refresh(); });
}
function cancelQuick() {
  $("quicktext").value = "";
  closeQuick(true);
}
$("quicktext").after(cancelButton(cancelQuick));
$("quicktext").oninput = () => { hintIndex = 0; renderQuick(); };
$("quicktext").onkeydown = (e) => {
  e.stopPropagation();
  if (composing(e)) return;
  if (e.key === "Escape") { e.preventDefault(); cancelQuick(); }
  else if (e.key === "Tab") {
    e.preventDefault();
    const hint = renderQuick();
    if (hint.items.length) pickHint(hint, hint.items[hintIndex]);
    else { quick.timed = !quick.timed; renderQuick(); }
  } else if (e.key === "Enter") {
    e.preventDefault();
    const hint = renderQuick();
    if (hint.items.length) pickHint(hint, hint.items[hintIndex]);
    else submitQuick();
  } else if ((e.key === "ArrowDown" || e.key === "ArrowUp") && !$("taghints").hidden) {
    e.preventDefault();
    const n = $("taghints").children.length;
    hintIndex = (hintIndex + (e.key === "ArrowDown" ? 1 : n - 1)) % n;
    renderQuick();
  }
};
$("quicktime").onclick = () => { quick.timed = !quick.timed; renderQuick(); $("quicktext").focus(); };
$("quicktext").onblur = () => setTimeout(() => { if (quick && document.activeElement !== $("quicktext") && !$("quicktext").value.trim()) closeQuick(false); }, DOUBLE_CLICK_MS);

function layoutIcon(id, n) {
  const l = LAYOUTS.find((x) => x.id === id);
  const count = Math.min(l.max, Math.max(l.min, n));
  const r = rects(count, id, 20, 14, 1);
  return svg("svg", { viewBox: "0 0 20 14", class: "lic" }, ...r.tiles.map((b, i) => svg("rect", { x: b.x, y: b.y, width: b.w, height: b.h, rx: 1, class: i === 0 && id !== "grid" && id !== "row" ? "big" : "" })));
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
      b.onclick = () => { closeMenu(); saveLayout(s, l.id, count > present ? count : 0); osd("grid", t(l.name), { kind: "view" }); };
      return b;
    }));
    box.append(counts, kinds);
    openMenu([{ element: box }], anchor, true);
  };
  show(Math.max(present, s.slots || 0));
}

function openMenu(list, at, keepFocus) {
  const m = $("menu");
  const target = at instanceof Event ? (at.currentTarget instanceof Element ? at.currentTarget : at.target) : at;
  if (!keepFocus || m.hidden) {
    menuReturn = document.activeElement;
    anchorMenu(target.hasAttribute("aria-haspopup") ? target : null);
  }
  m.replaceChildren(...list.map((it) => {
    if (it.separator) return el("hr");
    if (it.heading) return el("div", { className: "heading", textContent: it.heading });
    if (it.element) return it.element;
    const lead = it.check !== undefined ? el("span", { className: "ck" }, it.check ? icon("check", "sm") : null) : it.icon ? icon(it.icon, "sm") : null;
    const b = el("button", { role: "menuitem", disabled: !it.run || !!it.disabled, className: it.danger ? "danger" : "" }, lead, el("span", { className: "label", textContent: it.label }), it.right !== undefined ? el("span", { className: "right", textContent: it.right }) : null);
    b.onclick = () => { closeMenu(); it.run(); };
    return b;
  }));
  const point = at instanceof Event && at.type === "contextmenu" && (at.clientX || at.clientY);
  openPopup(m, point ? { left: at.clientX, right: at.clientX, top: at.clientY, bottom: at.clientY } : target, false);
  const first = m.querySelector("button:not(:disabled)");
  if (first && !keepFocus) first.focus();
}
function anchorMenu(a) {
  if (menuAnchor) menuAnchor.removeAttribute("aria-expanded");
  menuAnchor = a;
  if (a) a.setAttribute("aria-expanded", "true");
}
function closeMenu() {
  if ($("menu").hidden) return;
  $("menu").hidden = true;
  anchorMenu(null);
  if (menuReturn && menuReturn.isConnected) menuReturn.focus();
}
let reopening = null;
document.addEventListener("mousedown", (e) => {
  reopening = [$("menu"), $("pop")].map((box) => !box.hidden && box.anchor && box.anchor.at instanceof Element && box.anchor.at.contains(e.target) ? box.anchor.at : null).find(Boolean) || null;
  if (!$("menu").contains(e.target)) closeMenu();
  if (!$("pop").contains(e.target)) closePop();
});
document.addEventListener("click", (e) => {
  const opener = reopening;
  reopening = null;
  if (opener && opener.contains(e.target)) e.stopPropagation();
}, true);
function closePop() {
  if ($("pop").hidden) return;
  $("pop").hidden = true;
  if (popReturn && popReturn.isConnected) popReturn.focus();
  popReturn = null;
  if (popClosed) { const f = popClosed; popClosed = null; f(); }
}
let popClosed = null;
function stageMenu(e, key) {
  const set = playSet();
  if (!set) return;
  const pick = !play.view && play.keys.length > 1;
  openMenu([
    ...(pick ? [{ label: t("Switch app sound"), icon: "volume-2", check: soundKey(0, play.keys, st.id, gameSound) === key, run: () => chooseSound(key) }, { separator: true }] : []),
    { label: t("Add note"), icon: "note", right: "C", run: openQuick },
    { label: t("Copy link to this moment"), icon: "link", run: () => navigator.clipboard.writeText(momentLink(set.id, nowM(), play.view ? play.keys[play.view - 1] : null)).then(() => osd("link", t("Link copied"))).catch(fail) },
    { label: t("Clear drawing"), icon: "eraser", right: "X", run: () => clearInk(false) },
  ], e);
}

let lastError = null;
function flash(msg, kind = "info", action = null) {
  const text = typeof msg === "object" && msg && msg.code ? codeText(msg) : t(String(msg));
  lastError = kind === "error" ? { text } : null;
  $("toasttext").textContent = text;
  $("toasticon").replaceChildren(icon({ error: "alert", update: "refresh" }[kind] || "check"));
  $("toastaction").hidden = !action;
  if (action) { $("toastaction").textContent = action.label; $("toastaction").onclick = () => { $("toast").hidden = true; action.run(); }; }
  $("toast").hidden = false;
  hideAfter($("toast"), TOAST_MS);
}
function codeText(m) {
  if (m.code === "removed") return t("{by} removed {name} from the room", m);
  if (m.code === "removed_you") return t("{by} removed you from the room", m);
  return m.code;
}
function downloadLine(label, d) {
  return d ? `${label} ${downloadStats(d).text}` : label;
}
function notices() {
  const list = [];
  if (st.ffmpeg === "downloading") list.push({ key: "ffmpeg", level: "info", icon: "spinner", spin: true, text: downloadLine(t("Downloading video tools…"), st.ffmpeg_progress), sticky: true });
  if (st.ffmpeg === "failed") list.push({ key: "ffmpegfail", level: "error", icon: "fail", text: t("Couldn't download the video tools. Check your connection."), action: { label: t("Retry"), run: () => run("retry_ffmpeg", null) } });
  if (st.crashed) {
    const seen = () => { dismissed.add("crash"); run("crash_seen", null); };
    list.push({ key: "crash", level: "warn", icon: "alert", text: t("The app did not close normally last time."), action: { label: t("Send report…"), run: () => { seen(); openFeedback("bug"); } }, close: seen });
  }
  if (st.notice) list.push({ key: "n:" + st.notice, level: "warn", icon: "alert", text: t(st.notice) });
  for (const p of st.peers.filter((x) => x.connected && x.other_version)) list.push({ key: "v:" + p.id, level: "warn", icon: "alert", text: t("{name} is on a different version. Both of you need the latest one.", { name: nameOf(p.id) }) });
  if (available) list.push({ key: "update", level: "info", icon: "download", text: t("Version {version} is available", { version: available.version }), action: { label: t("Install"), primary: true, run: installUpdate } });
  if (updateText) list.push({ key: "updating", level: "info", icon: "spinner", spin: true, text: updateText, sticky: true });
  return list.filter((n) => !dismissed.has(n.key));
}
function renderNotices() {
  const list = notices();
  if (!changed("notices", [list.map((n) => [n.key, n.text, n.level, n.action && n.action.label]), lang])) return;
  $("notices").replaceChildren(...list.map((n) => el("div", { className: "notice " + n.level, role: n.level === "error" ? "alert" : "status" },
    icon(n.icon, n.spin ? "spin" : ""), el("span", { className: "text", textContent: n.text }),
    n.action ? el("button", { className: "sm" + (n.action.primary ? " primary" : ""), textContent: n.action.label, onclick: n.action.run }) : null,
    n.sticky ? null : iconButton("x", t("Close"), () => { dismissed.add(n.key); if (n.close) n.close(); delete sigs.notices; renderNotices(); }))));
}
function exportOptions(set, video) {
  const shown = video ? setById(set.of) : set;
  const owners = shown ? shown.players.map((p) => p.owner) : [];
  const choice = exportChoice(stored("exportOptions", null), owners);
  let folder = null;
  const show = () => {
    const v = exportView(set, folder, video);
    $("exportopts").hidden = !v.options;
    $("exportrangerow").hidden = v.video;
    $("exportburn").hidden = !v.video;
    $("exportpath").textContent = v.path;
    $("exportyes").disabled = !v.ready;
  };
  const where = () => invoke("export_target", { room: set.room, set: set.id }).then((f) => { folder = f; show(); });
  $("exporttitle").dataset.t = video ? "Export as video" : "Export to folder";
  $("exporttitle").textContent = t($("exporttitle").dataset.t);
  $("exportrange").value = choice.range;
  $("exportquality").value = choice.quality;
  $("exportmember").replaceChildren(el("option", { value: "", textContent: t(video ? "As shown" : "Everyone") }), ...owners.map((o) => el("option", { value: o, textContent: nameOf(o) })));
  $("exportmember").value = choice.member;
  for (const k of ["notes", "marks", "matches"]) $("export" + k).checked = choice[k];
  $("exportpick").onclick = () => dialog("pick_export_base").then((base) => base && where()).catch(fail);
  show();
  const d = $("exportdlg");
  d.showModal();
  where().catch((e) => { d.close(); fail(e); });
  return new Promise((done) => {
    $("exportyes").onclick = () => {
      const picked = exportChoice({ range: $("exportrange").value, quality: $("exportquality").value, member: $("exportmember").value, notes: $("exportnotes").checked, marks: $("exportmarks").checked, matches: $("exportmatches").checked }, owners);
      const v = exportView(set, folder, video);
      if (v.options) store("exportOptions", picked);
      d.close();
      done({ folder, ...(v.options ? exportArgs(picked, video) : {}) });
    };
    d.onclose = () => done(null);
  });
}
async function exportFolder(set, video = false) {
  if (!set) { flash(t("Open a recording first")); return; }
  try {
    await invoke("export_check");
    const options = await exportOptions(set, video);
    if (!options) return;
    exporting = { set: set.id, percent: 0, folder: options.folder };
    refresh();
    invoke("export_folder", { room: set.room, set: set.id, ...options }).then((r) => {
      if (r.skipped.length) flash(t("Exporting without {names}: not downloaded yet", { names: r.skipped.join(", ") }));
    }).catch((e) => { if (exporting) { exporting = null; fail(e); refresh(); } });
  } catch (e) {
    fail(e);
  }
}
listenMain("export", (e) => {
  const x = e.payload;
  if (x.done) {
    exporting = null;
    if (x.error) fail(x.error);
    else flash(t("Export finished"), "info", x.folder ? { label: t("Open folder"), run: () => dialog("open_folder", { path: x.folder }).catch(fail) } : null);
    refresh();
    return;
  }
  if (exporting) exporting.percent = x.percent;
  renderSessions();
});
async function checkUpdate(manual) {
  if (updateText) return;
  checkedAt = Date.now();
  try {
    const version = await invoke("check_update");
    available = version ? { version } : null;
  } catch (e) {
    warn(e);
    if (manual) fail(t("Couldn't check for updates"));
    return;
  }
  if (!available && manual) flash(t("You have the latest version"), "update");
  dismissed.delete("update");
  renderNotices();
}
async function installUpdate() {
  const update = available;
  available = null;
  updateText = t("Installing {version}… The app will restart.", { version: update.version });
  renderNotices();
  await invoke("install_update").catch((e) => { warn(e); updateText = ""; available = update; fail(e); renderNotices(); });
}

function refresh() {
  if (refreshing) { refreshAgain = true; return refreshing; }
  refreshing = (async () => {
    try { do { refreshAgain = false; await update(); } while (refreshAgain); }
    finally { refreshing = null; }
  })();
  return refreshing;
}
let refreshing = null;
let refreshAgain = false;
const steps = () => [renderShell, renderTitle, renderStart, renderSide, renderNotices, renderPlayer, renderEvents, renderTools, renderRoom, renderRecovery, renderOpenDialogs];
async function update() {
  st = await invoke("state");
  clockShift = st.now_us - Date.now() * 1000;
  idx = indexOf(st);
  titles = Object.fromEntries(st.sets.filter((x) => x.title).map((x) => [x.id, x.title]));
  importedIds = new Set(st.sets.flatMap((x) => x.players.filter((p) => p.imported).map((p) => p.owner)));
  for (const p of st.peers) if (p.speaking) speaking.add(p.id); else speaking.delete(p.id);
  if (lang !== st.lang) { lang = st.lang; setLanguage(lang); for (const k of Object.keys(sigs)) delete sigs[k]; }
  if (boardWindow) { for (const step of [renderShell, renderBoard]) try { step(); } catch (e) { logError(e, `board ${step.name}`); } return; }
  if (!ready) { ready = true; pollPlugins(); }
  if (selected && !setById(selected)) selected = null;
  if (!st.recording) played = false;
  const l = liveSet();
  const voice = st.active_set && st.active_set.voice;
  if (nowView && !voice) followNow();
  if (nowView && !l && nowSet) {
    const ended = setById(nowSet);
    nowView = false;
    nowSet = null;
    if (ended && !ended.of && page === "player") openFromStart(ended.id);
  }
  if (voice) debriefOpen = st.active_set.id;
  const roomId = st.room ? st.room.id : "";
  if (partyRoom !== roomId) { partyRoom = roomId; editing = null; if (roomId) await joinParty(); st = await invoke("state"); idx = indexOf(st); }
  if (!drag && watchParty && st.party.on && !sameParty(st.party.shared, partyLast)) applyParty(st.party.shared);
  for (const step of steps()) try { step(); } catch (e) { logError(e, `render ${step.name || "step"}`); }
  markLive();
  if (restored) saveLast();
  else { restored = true; if (inRoom()) restoreLast(); }
}
function renderTitle() {
  const set = page === "player" ? playSet() : null;
  const title = windowTitle(st.app, inRoom() ? st.room.name : "", set && !set.live ? sessionName(set.id) : "");
  if (changed("wintitle", title)) window.__TAURI__.window.getCurrentWindow().setTitle(title).catch(warn);
}
function saveLast() {
  if (!inRoom()) return;
  const last = { room: st.room.id, page, tab: roomTab, set: page === "player" && !nowView ? selected : null, scroll: Math.round($("reviewlist").scrollTop) };
  if (changed("last", last)) store("last", last);
}
function restoreLast() {
  const last = stored("last", null);
  if (!last || last.room !== st.room.id) return;
  const set = last.set && setById(last.set);
  if (set && onThisPc(set)) {
    openSession(set.id);
    refresh().then(() => { $("reviewlist").scrollTop = last.scroll || 0; });
  } else if (last.page === "room" && last.tab) showRoomTab(last.tab);
}
function renderRecovery() {
  $("recovery").hidden = !st.recovery;
  $("recoverymsg").textContent = st.recovery ? t(st.recovery) : "";
  $("rinfo").textContent = st.previous_version ? t("Version {version}, previous run {previous}", { version: st.version, previous: st.previous_version }) : t("Version {version}", { version: st.version });
}

function toggleFull() {
  if (onBoard()) { toggleBoardFull(); return; }
  if (page !== "player" && !full) go("player");
  full = !full;
  $("playerpage").classList.toggle("full", full);
  renderShell();
  delete sigs.fullicon;
  setFull(full);
  wake();
}
function wake() {
  const body = document.body;
  body.classList.remove("still", "idle");
  $("exitfull").hidden = !(full || boardFull);
  clearTimeout(idleTimer);
  if (full || boardFull) idleTimer = setTimeout(() => {
    body.classList.add("still");
    if (boardFull || clock.playing) body.classList.add("idle");
  }, IDLE_MS);
}
document.addEventListener("mousemove", wake);
function go(p) {
  if (p !== page) closeQuick(false);
  if (p !== "player" && full) toggleFull();
  if (boardFull && !(p === "room" && roomTab === "board")) toggleBoardFull();
  page = p;
  if (p !== "player") nowView = false;
  closeMenu();
  if (st) { delete sigs.sessions; renderShell(); renderSide(); place(rec); place(play); }
  refresh();
}
function openLive() {
  nowView = false;
  if (selected) selectSet(null);
  go("player");
}
function showPanel(on) {
  if (innerWidth > WIDE_PANEL) { panel = on; store("panel", panel); } else panelOpen = on;
  renderShell();
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
  $("confirmyes").className = danger ? "danger" : "primary";
  d.showModal();
  $(danger ? "confirmno" : "confirmyes").focus();
  return new Promise((done) => {
    $("confirmyes").onclick = () => { d.close(); done(true); };
    d.onclose = () => done(false);
  });
}
function chime(action) {
  if (!sessionSound) return;
  try {
    const ctx = audio();
    CHIME[action].forEach((hz, i) => {
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      const at = ctx.currentTime + i * CHIME_NOTE_S;
      osc.frequency.value = hz;
      gain.gain.setValueAtTime(CHIME_GAIN, at);
      gain.gain.exponentialRampToValueAtTime(0.0001, at + CHIME_NOTE_S);
      osc.connect(gain).connect(ctx.destination);
      osc.start(at);
      osc.stop(at + CHIME_NOTE_S);
    });
  } catch (e) {
    warn(e);
  }
}
function setSpeaking(id, on) {
  if (on) speaking.add(id); else speaking.delete(id);
  markLive(id);
}
listenMain("speaking", (e) => setSpeaking(e.payload.from, e.payload.on));
function anchor() {
  clock.m = nowM();
  clock.at = performance.now();
}
function hold(on) {
  if (on === held) return;
  anchor();
  held = on;
}
function setRate(r) {
  anchor();
  rate = r;
  $("rate").textContent = osdSpeed(r);
  renderSpeed();
  applyAudio();
}
const changeRate = (dir) => { setRate(SPEEDS[Math.max(0, Math.min(SPEEDS.length - 1, SPEEDS.indexOf(rate) + dir))]); osd("gauge", osdSpeed(rate), { kind: "speed" }); share(); };
const toggleMute = () => { muted = !muted; applyAudio(); osdVolumeNow(); };
const osdVolumeNow = () => osd(muted || volume === 0 ? "volume-x" : "volume-2", osdVolume(volume, muted), { kind: "volume", bar: muted ? 0 : volume });
function setVolume(v) {
  volume = Math.max(0, Math.min(1, v));
  muted = false;
  store("volume", volume);
  applyAudio();
  osdVolumeNow();
}
document.addEventListener("mouseover", (e) => {
  for (let n = e.target; n instanceof HTMLElement && n !== document.body; n = n.parentElement) {
    if (n.scrollWidth > n.clientWidth && getComputedStyle(n).textOverflow === "ellipsis") { n.title = n.title || n.textContent; return; }
  }
});
const EDITABLE = "input:not([type=range]):not([type=checkbox]):not([type=radio]):not([type=button]), textarea";
function editMenu(e, field) {
  const has = field ? field.selectionStart !== field.selectionEnd : String(getSelection()).length > 0;
  const editable = !!field && !field.readOnly && !field.disabled;
  const paste = () => navigator.clipboard.readText().then((text) => { field.setRangeText(text, field.selectionStart, field.selectionEnd, "end"); field.dispatchEvent(new Event("input", { bubbles: true })); }).catch(warn);
  openMenu([
    ...(field ? [{ label: t("Cut"), icon: "scissors", disabled: !editable || !has, run: () => document.execCommand("cut") }] : []),
    { label: t("Copy"), icon: "copy", disabled: !has, run: () => document.execCommand("copy") },
    ...(field ? [{ label: t("Paste"), icon: "clipboard", disabled: !editable, run: paste }, { label: t("Select all"), run: () => field.select() }] : []),
  ], e);
}
document.addEventListener("contextmenu", (e) => {
  if (e.defaultPrevented) return;
  e.preventDefault();
  const field = e.target.closest ? e.target.closest(EDITABLE) : null;
  if (field || String(getSelection()).length) editMenu(e, field);
});
const reviewing = () => !!(st.party.on && st.party.shared && st.party.shared.set);
async function toggleRecording() {
  if (!st) return;
  const ending = !!st.active_set;
  const debrief = !ending && reviewing();
  const voice = ending && st.active_set.voice;
  const title = voice ? t("Stop the debrief recording?") : ending ? t("Stop recording for everyone?") : debrief ? t("Record the debrief for everyone?") : t("Start recording for everyone?");
  const action = voice ? t("Stop debrief") : ending ? t("Stop recording") : debrief ? t("Record debrief") : t("Start recording");
  if (!(await ask(title, action, ending))) return;
  run(ending ? "stop_recording" : "start_recording");
}
const current = () => (playSet() ? play : rec);

$("play").onclick = togglePlay;
$("gamesound").onclick = (e) => openMix(e.currentTarget);
$("back").onclick = () => jump(-STEP.normal);
$("fwd").onclick = () => jump(STEP.normal);
$("addnote").onclick = openQuick;
$("live").onclick = () => { if (!playSet()) return; goLive(); share(); renderPlayer(); };
$("rate").onclick = (e) => speedMenu(e.currentTarget);
$("matchchip").onclick = (e) => matchMenu(e.currentTarget);
$("volume").oninput = () => setVolume(Number($("volume").value));
$("mute").onclick = toggleMute;
$("panelbtn").onclick = togglePanel;
$("full").onclick = toggleFull;
$("exitfull").onclick = toggleFull;
$("tool").onclick = cycleTool;
$("clearink").onclick = () => clearInk(false);
$("toast").onmouseenter = () => clearTimeout($("toast").hideTimer);
$("toast").onmouseleave = () => hideAfter($("toast"), TOAST_MS);
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
    osd(null, fmt(nowM()), { kind: "seek", bar: total() ? nowM() / total() : undefined });
  };
};
const peek = { slot: slot(el("video", { muted: true, preload: "auto" }), "peek"), at: null };
peek.slot.video.onseeked = () => {
  const c = $("peek").querySelector("canvas");
  c.getContext("2d").drawImage(peek.slot.video, 0, 0, c.width, c.height);
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
  peek.at = { p, wall: base(list) + m * 1e6 };
}
$("seek").onmousemove = (e) => peekAt(e.clientX);
$("seek").onmouseleave = () => { $("peek").hidden = true; peek.at = null; };
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
  p: once(cycleTool),
  x: () => clearInk(false),
  "Shift+p": () => stepSet(-1),
  "Shift+n": () => stepSet(1),
  Home: () => seekEdge(false),
  End: () => seekEdge(true),
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
  else if ((e.key === "ArrowDown" || e.key === "ArrowUp") && list.length) { e.preventDefault(); const down = e.key === "ArrowDown"; list[i < 0 ? (down ? 0 : list.length - 1) : (i + (down ? 1 : -1) + list.length) % list.length].focus(); }
}
function escapeKey() {
  const step = escapeStep({ menu: !$("menu").hidden, dialog: !!document.querySelector("dialog[open]"), pop: !$("pop").hidden, typing: !!typing(), full: full || boardFull, window: boardWindow, panel: panelOpen });
  if (step === "pop") closePop();
  else if (step === "blur") document.activeElement.blur();
  else if (step === "full") (full ? toggleFull : toggleBoardFull)();
  else if (step === "close") quiet("close_board_window");
  else if (step === "panel") showPanel(false);
}
document.addEventListener("focusin", (e) => {
  if (!e.target.matches(":focus-visible") && !typing() && !e.target.closest("dialog, .menu")) e.target.blur();
});
document.addEventListener("keydown", (e) => {
  if (composing(e) || listening) return;
  if (!$("menu").hidden) { menuKey(e); return; }
  const ctrl = e.ctrlKey && !e.altKey && !e.metaKey;
  if (ctrl && e.key === ",") { e.preventDefault(); openSettings(); return; }
  if (document.querySelector("dialog[open]")) return;
  if (e.key === "?" && !typing() && !e.ctrlKey && !e.metaKey && !e.altKey) { e.preventDefault(); openSettings("shortcuts"); return; }
  if (!inRoom()) return;
  if (ctrl && ["1", "2", "3", "4", "b", "r"].includes(e.key.toLowerCase())) { e.preventDefault(); return; }
  if (ctrl && e.key.toLowerCase() === "z" && !typing()) { e.preventDefault(); runUndo(); return; }
  if (e.key === "Escape") { escapeKey(); return; }
  if (e.key === "F11" && !e.ctrlKey && !e.altKey && !e.metaKey && (full || boardFull || onBoard() || page === "player")) { e.preventDefault(); toggleFull(); return; }
  if (typing() || nativeKey(e) || e.metaKey || e.altKey) return;
  const player = page === "player" && !!playSet();
  const key = e.key.normalize("NFKC");
  const k = key.length === 1 ? key.toLowerCase() : key;
  let action = null;
  if (e.ctrlKey) action = player && (key === "ArrowLeft" || key === "ArrowRight") ? () => jump(key === "ArrowLeft" ? -STEP.huge : STEP.huge) : null;
  else if (onBoard() && k === "f") action = toggleBoardFull;
  else if (st.recording && !player && (k === "m" || k === "v")) action = once(() => $(k === "m" ? "micbtn" : "screenbtn").click());
  else if (page === "player") action = stageKey(e, key) || (player && (PLAYER_KEYS[(e.shiftKey ? "Shift+" : "") + k] || PLAYER_KEYS[k])) || null;
  if (!action) return;
  e.preventDefault();
  action(e);
});

function probeTiles() {
  if (document.hidden) return;
  const slots = [...Object.values(play.tiles), ...Object.values(rec.tiles)].map((tl) => tl.media).filter((x) => x.video.isConnected && !x.video.paused);
  playerProbe(slots, slots.length, !!(play.mini && play.mini.box && !play.mini.box.hidden));
}

async function reportCodecs() {
  const types = { av1: CODECS.av01, hevc: CODECS.hvc1, h264: CODECS.avc1 };
  const list = [];
  for (const [name, codec] of Object.entries(types)) {
    try {
      const info = await navigator.mediaCapabilities.decodingInfo({ type: "media-source", video: { contentType: `video/mp4; codecs="${codec}"`, ...CODEC_PROBE } });
      if (info.supported) list.push(name);
    } catch (e) { warn(e); }
  }
  quiet("set_codecs", { list: list.length ? list : ["h264"] });
}

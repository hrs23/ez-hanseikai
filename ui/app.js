const STEP = { small: 1, normal: 5, big: 10, huge: 60 };
const VOLUME_STEP = 0.05;
const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3];
const MAX_KEYED = 6;
const TICK_MS = 150;
const REFRESH_MS = 1000;
const IDLE_MS = 2000;
const OSD_MS = 2500;
const SEEKWHO_MS = 4000;
const TOAST_MS = 4000;
const UNDO_MS = 8000;
const UPDATE_CHECK_DELAY_MS = 4000;
const UPDATE_EVERY_MS = 4 * 3600 * 1000;
const UPDATE_RECHECK_MS = 10 * 60 * 1000;
const DOUBLE_CLICK_MS = 250;
const JUMP_MIN = 1;
const NOTE_STEP_GAP = 0.5;
const JOBS_MS = 5000;
const DOWNLOAD_POLL_MS = 1000;
const CHIME = { start: [660, 880], end: [880, 660] };
const CHIME_NOTE_S = 0.14;
const CHIME_GAIN = 0.15;
const TAG_HINTS = 5;
const AVATARS_SHOWN = 5;
const ROOM_NAME_MAX = 40;
const DRAG_START_PX = 4;
const CODEC_PROBE = { width: 1280, height: 720, bitrate: 6000000, framerate: 30 };
const GAP = 6;
const WIDE_PANEL = 1240;

const tileGrid = (box, kind) => ({ el: box, kind, tiles: {}, keys: [], mode: "grid", slots: 0, view: 0, last: 1, free: [] });
const rec = tileGrid($("livetiles"), "record");
const play = tileGrid($("playtiles"), "player");
const clock = { playing: false, m: 0, at: 0 };
const liveClock = { pos: null, rate: 1, held: false, late: [], at: 0 };
const voices = {};
let st = null;
let idx = null;
let ready = false;
let lang = "";
let page = "player";
let nowView = false;
let nowSet = null;
let selected = null;
let review = null;
let rate = 1;
let volume = stored("volume", 1);
let sessionSound = stored("sessionSound", true);
let chimeAudio = null;
let muted = false;
let live = false;
let watchParty = stored("watchParty", true);
let partyLast = null;
let partyLayout = null;
let drag = null;
let items = [];
let osdTimer = null;
let whoTimer = null;
let toastTimer = null;
let idleTimer = null;
let undo = null;
let available = null;
let installAfter = false;
let askingUpdate = false;
let checkedAt = 0;
let updateText = "";
let menuReturn = null;
let menuAnchor = null;
let exporting = null;
let importedIds = new Set();
let side = "review";
let scope = "session";
let subtitles = stored("subtitles", true);
let receive = stored("receive", {});
let sidebar = stored("sidebar", true);
let panel = stored("panel", true);
let panelOpen = false;
let clickTimer = null;
let quick = null;
let partyRoom = null;
let jobInfo = null;
let plugins = [];
let pluginsGen = 0;
let dismissed = new Set();
let full = false;
let listening = false;

const peerOf = (id) => st.peers.find((p) => p.id === id);
const nameOf = (id) => (id === st.id ? st.name || t("You") : st.names[id] || shortId(id));
const people = () => [st.id, ...st.peers.map((p) => p.id)];
const liveSet = () => st.sets.find((s) => s.live) || null;
const inRoom = () => !!st && !!st.room && !st.room.pending;
const fail = (e) => flash(e, "error");
const run = (cmd, args, then = refresh) => invoke(cmd, args).then(then).catch(fail);
const quiet = (cmd, args) => invoke(cmd, args).catch(warn);
const showingNow = () => nowView || (!playSet() && !review);
const followingLive = () => live && !review && !!playSet() && playSet().live;
const livePinned = () => followingLive() && liveClock.pos !== null;
const setById = (id) => (st && st.sets.find((s) => s.id === id)) || null;
const reviewSet = () => (review ? setById(review.id) : null);
const reviewing = () => !!(watchParty && st && st.party.on && st.party.shared && typeof st.party.shared.set === "string");

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

function avatars(ids, size = "sm", max = AVATARS_SHOWN) {
  return el("span", { className: "avs" }, ...ids.slice(0, max).map((id) => avatar(id, size)));
}

function iconButton(name, label, onclick, cls = "icon sm") {
  return el("button", { className: cls, ariaLabel: label, title: label, onclick }, icon(name, "sm"));
}

function pill(cls, iconName, text) {
  return el("span", { className: "pill " + cls }, iconName ? icon(iconName, "sm") : null, text);
}

function tile(s, key) {
  if (s.tiles[key]) return s.tiles[key];
  const tl = { key, media: slot(el("video", { muted: true, preload: "auto" })), mic: null, name: el("span", { className: "nm" }), av: el("span"), dot: el("i", { className: "rd", hidden: true }), imp: el("span", { className: "imp", hidden: true }), who: el("div", { className: "who" }), what: el("div", { className: "what" }), sub: el("div", { className: "subt", hidden: true }) };
  tl.lab = el("div", { className: "nl" }, tl.av, tl.name, tl.imp, tl.dot);
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
    tl.box.onclick = (e) => { if (e.target.closest(".choose")) return; focus(); };
    tl.box.ondblclick = toggleFull;
  }
  tl.lab.onpointerdown = (e) => dragTile(s, key, e);
  s.el.append(tl.box);
  return (s.tiles[key] = tl);
}
function nameTile(tl, key, recording) {
  tl.name.textContent = nameOf(key);
  tl.who.textContent = nameOf(key);
  tl.dot.hidden = !recording;
  tl.imp.hidden = !importedIds.has(key);
  tl.imp.textContent = t("Imported");
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
  if (s === play) logLayout();
}
const fromSaved = (l) => ({ mode: (l && l.mode) || "grid", slots: (l && l.slots) || 0, order: (l && l.order) || [] });
const playLayout = (set) => fromSaved(stored("layout/" + set.id, null) || set.layout);
function logLayout() {
  const set = playSet();
  const size = (key, sideName) => Math.round(play.tiles[key] ? play.tiles[key].box[sideName] : 0);
  if (set && !review) quiet("log_layout", { layout: { set: set.id, mode: play.view ? "focus" : play.mode, slots: play.slots, tiles: play.keys.map((key, i) => ({ id: key, index: i, w: size(key, "clientWidth"), h: size(key, "clientHeight") })) } });
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
  return keys.map((key) => ({ key, file: files[key] && files[key].size > 0 ? files[key] : null, recording: recOf(key) && (key !== st.id || st.capture.working), sharing: key !== st.id && !!(peerOf(key) || {}).sharing }));
}
const liveStart = (key, part) => (part && typeof part.start_us === "number" ? (part.start_us - (key === st.id ? 0 : (peerOf(key) || {}).offset_us || 0)) / 1e6 : null);
function liveMembers() {
  if (playSet()) {
    return players().filter((p) => play.tiles[p.key]).map((p) => {
      const tl = play.tiles[p.key];
      const last = p.file.parts[p.file.parts.length - 1] || {};
      const on = (s, part) => !!(s && part && pathOf(s.part) === pathOf(part));
      const slots = [[tl.media, last.v], [tl.mic, last.m]].filter(([s, part]) => on(s, part)).map(([s]) => [(s.part.start_us - p.off) / 1e6, s.video]);
      return { key: p.key, avail: on(tl.media, last.v) ? liveAvail(slots) : null, ended: !p.recording };
    });
  }
  return livePlayers().filter((p) => rec.tiles[p.key]).map(({ key, file }) => ({ key, avail: file ? liveAvail([[liveStart(key, rec.tiles[key].media.part), rec.tiles[key].media.video]]) : null, ended: false }));
}
function goLive() {
  live = true;
  setClock(true, Math.max(0, total() - LIVE_HEADROOM_S - LIVE_SHRINK_GAP_S));
}
function tickLive() {
  const at = performance.now();
  const on = playSet() ? followingLive() : !review && Object.keys(rec.tiles).length > 0;
  if (!on) { Object.assign(liveClock, { pos: null, rate: 1, held: false, late: [], at }); return; }
  Object.assign(liveClock, liveStep(liveClock, liveMembers(), nowUs() / 1e6, (at - liveClock.at) / 1000), { at });
  if (changed("liverate", liveClock.rate)) applyAudio();
  if (livePinned()) Object.assign(clock, { playing: true, m: nowM(), at });
  if (playSet()) return;
  for (const p of livePlayers()) {
    const tl = rec.tiles[p.key];
    if (!tl) continue;
    tl.box.classList.toggle("late", liveClock.late.includes(p.key));
    if (!p.file) continue;
    const v = tl.media.video;
    const start = liveStart(p.key, tl.media.part);
    const edge = start === null || liveClock.pos === null;
    const step = edge ? liveFollow(v.duration, v.currentTime, tl.media.loadSec) : { target: liveClock.pos - start, playing: !liveClock.held };
    sync(v, step.target, step.playing);
    v.playbackRate = edge ? 1 : liveClock.rate;
  }
}
function liveWhat(p) {
  if (p.key === st.id) {
    if (!st.capture.source) return [el("div", { className: "big" }, icon("monitor-off")), el("b", { textContent: t("{name} · Screen off", { name: nameOf(st.id) }) }), el("button", { className: "sm choose", textContent: t("Choose screen"), onclick: (e) => { e.stopPropagation(); openSource($("screenmore")); } })];
    return [el("span", { textContent: captureProblem(st.capture) || t("Starting…") })];
  }
  return [el("span", { textContent: p.sharing ? t("Starting…") : t("Screen off") })];
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
    if (changed("livewhat/" + p.key, [!!p.file, p.sharing, st.capture.source, captureProblem(st.capture), nameOf(p.key), lang])) tl.what.replaceChildren(...(p.file ? [t("Waiting for {name}…", { name: nameOf(p.key) })] : liveWhat(p)));
    tl.box.classList.toggle("wait", !p.file);
    tl.box.classList.toggle("mine", p.key === st.id && !st.capture.source);
    if (!p.file) continue;
    const start = liveStart(p.key, p.file);
    if (tl.media.src === srcOf(p.file)) tl.media.part = p.file;
    feed(tl.media, p.file, true, p.file, () => (start === null || liveClock.pos === null ? 0 : liveClock.pos - start));
  }
  place(rec);
}

function renderShell() {
  const room = inRoom();
  $("start").hidden = room;
  $("app").hidden = !room;
  $("roompage").hidden = page !== "room";
  $("playerpage").hidden = page !== "player";
  document.body.classList.toggle("noside", !sidebar);
  document.body.classList.toggle("nopanel", !panel || (page === "player" && !playSet() && !review));
  document.body.classList.toggle("panelopen", panelOpen && (!!playSet() || !!review));
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
    if (p.file.end_us) ends.push((p.file.end_us - p.off - b) / 1e6);
    const tl = play.tiles[p.key];
    if (tl && tl.media.part && isFinite(tl.media.video.duration)) ends.push(tl.media.video.duration - shiftOf(p, tl.media.part, b));
  }
  return ends.length ? Math.max(...ends) : 0;
}
function total(list = players()) {
  return Math.max(received(list), list.some((p) => p.recording) ? (nowUs() - base(list)) / 1e6 : 0);
}
const nowM = () => (livePinned() ? liveClock.pos - base() / 1e6 : clock.playing ? clock.m + ((performance.now() - clock.at) / 1000) * rate : clock.m);
const wallOf = (m) => Math.round(base() + m * 1e6);
function setClock(playing, m) {
  clock.playing = playing;
  clock.m = Math.max(0, m);
  clock.at = performance.now();
  tick();
}
const reviewM = () => (review ? (review.playing ? review.m + ((performance.now() - review.at) / 1000) * rate : review.m) : 0);
const reviewTotal = () => { const s = reviewSet(); return s ? ((s.end_us || nowUs()) - s.start_us) / 1e6 : 0; };
function setReview(playing, m) {
  if (!review) return;
  review.playing = playing;
  review.m = Math.max(0, Math.min(m, reviewTotal()));
  review.at = performance.now();
  tick();
}
const shownM = () => (review ? reviewM() : nowM());
const shownTotal = () => (review ? reviewTotal() : total());

const recv = (key) => ({ app: { volume: 1, muted: false }, voice: { volume: 1, muted: false }, ...(receive[key] || {}) });
function applyAudio() {
  const away = page !== "player";
  play.keys.forEach((key, i) => {
    const tl = play.tiles[key];
    if (!tl) return;
    const heard = key === soundKey(play.view, play.keys, st.id);
    const mine = recv(key);
    for (const [s, on, mix] of [[tl.media, heard, mine.app], [tl.mic, true, mine.voice]]) if (s) for (const v of [s.video, s.pending && s.pending.video]) if (v) {
      v.muted = away || muted || !on || mix.muted;
      v.volume = volume * mix.volume;
      v.playbackRate = livePinned() ? liveClock.rate : rate;
    }
  });
  for (const [key, s] of Object.entries(voices)) {
    const mix = recv(key).voice;
    for (const v of [s.video, s.pending && s.pending.video]) if (v) { v.muted = away || muted || mix.muted; v.volume = volume * mix.volume; v.playbackRate = rate; }
  }
  if (changed("mute", [muted || volume === 0])) $("mute").replaceChildren(icon(muted || volume === 0 ? "volume-x" : "volume-2"));
  if (document.activeElement !== $("volume")) $("volume").value = muted ? 0 : volume;
  rangeFill($("volume"));
  $("speed").textContent = `${rate}×`;
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
function openReceive(key, anchor) {
  showPop([el("div", { className: "head" }, avatar(key, "sm"), el("b", { textContent: nameOf(key) })), mixRow(key, "app", t("App sound")), mixRow(key, "voice", t("Voice"))], anchor, true);
}
function openMix(anchor) {
  const keys = [...new Set([...play.keys, ...Object.keys(voices)])];
  showPop([el("div", { className: "head" }, el("b", { textContent: t("Volume per member") })), ...keys.flatMap((key) => [el("div", { className: "who" }, avatar(key, "sm"), nameOf(key)), mixRow(key, "app", t("App sound")), mixRow(key, "voice", t("Voice"))])], anchor, true);
}
function showPop(children, anchor, above, cls = "") {
  const pop = $("pop");
  pop.className = "pop " + cls;
  pop.replaceChildren(...children);
  pop.hidden = false;
  const r = anchor.getBoundingClientRect();
  pop.style.left = Math.max(4, Math.min(r.left, innerWidth - pop.offsetWidth - 4)) + "px";
  const top = above ? r.top - pop.offsetHeight - 6 : r.bottom + 6;
  pop.style.top = Math.max(4, Math.min(top, innerHeight - pop.offsetHeight - 4)) + "px";
}

function renderPlayer() {
  renderLive();
  const set = playSet();
  const idle = !set && !review;
  const list = players();
  const flow = review ? reviewFlow() : null;
  const shared = !review && watchParty && st.party.on && set && partyLayout && partyLayout.set === set.id;
  const lay = flow ? { mode: flow.mode, slots: flow.slots, order: flow.order } : set ? (shared ? { mode: partyLayout.mode || "grid", slots: partyLayout.slots || 0, order: partyLayout.order || [] } : playLayout(set)) : fromSaved(null);
  play.keys = ordered(list.map((p) => p.key), lay.order);
  play.mode = lay.mode;
  play.slots = lay.slots;
  if (flow) play.view = flow.focus ? play.keys.indexOf(flow.focus) + 1 : 0;
  else if (shared && partyLayout.focus !== undefined) { play.view = partyLayout.focus ? play.keys.indexOf(partyLayout.focus) + 1 : 0; delete partyLayout.focus; }
  prune(play, play.keys);
  for (const p of list) {
    const tl = tile(play, p.key);
    nameTile(tl, p.key, p.recording);
    feedParts(tl, p, nowM(), base(list), clock.playing);
  }
  place(play);
  feedVoices();
  applyAudio();
  if (page !== "player") return;
  renderHeader(set);
  renderViews();
  $("playtiles").hidden = !set;
  $("toolbar").hidden = idle;
  $("views").hidden = $("tools").hidden = !set;
  renderTransport(set, idle);
}
function renderTransport(set, idle) {
  const liveSetShown = !!(set && set.live && !review);
  const tot = shownTotal();
  const edge = liveEdge(total(), nowM());
  if (set && !set.live) live = false;
  for (const id of ["play", "back", "fwd", "addnote", "mixbtn", "cc", "seek", "lane", "chapters", "matchchip"]) $(id).classList.toggle("nodisp", idle);
  $("speed").classList.toggle("nodisp", idle || followingLive());
  $("addnote").classList.toggle("nodisp", idle || !!review);
  $("transport").classList.toggle("idle", idle);
  $("clocknow").parentElement.hidden = idle;
  $("live").hidden = !(idle || liveSetShown);
  const atLive = idle || live;
  $("live").className = "livebtn " + (atLive ? "at" : "behind");
  if (changed("livebtn", [idle, atLive, idle ? 0 : Math.round(edge.behind), lang])) $("live").replaceChildren(el("i"), idle ? t("Live") : atLive ? t("Latest") : t("Jump to latest"), atLive ? "" : el("span", { className: "lagt", textContent: "−" + fmt(edge.behind) }));
  $("seek").classList.toggle("onlive", liveSetShown && atLive);
  $("seek").querySelector(".grow").hidden = !(liveSetShown || (review && reviewSet() && reviewSet().live));
  press($("cc"), subtitles);
  const frac = set && !review && set.progress.total ? Math.min(1, set.progress.have / set.progress.total) : 1;
  $("seek").querySelector(".got").style.width = frac * 100 + "%";
  $("seek").querySelector(".missing").hidden = !(set && !review && set.progress.stuck);
  $("seek").querySelector(".missing").style.left = frac * 100 + "%";
  renderBands(tot);
}
function headerChips(set) {
  const a = st.active_set;
  if (review) { const r = reviewSet(); return [pill("accent", "mic", t("Debrief")), r && r.live ? pill("rec", "rec", t("Recording")) : null]; }
  if (!set) return [];
  if (set.live) {
    const edge = liveEdge(total(), nowM());
    return [pill("rec", "rec", t("Recording {time}", { time: fmt((nowUs() - a.start_us) / 1e6) })), live ? null : pill("gray", null, t("Playing back · {time} behind the latest", { time: fmt(edge.behind) }))];
  }
  if (set.imported_by) return [pill("gray", "download", t("Imported by {name}", { name: set.imported_by }))];
  return [isReview(set) ? pill("accent", "mic", t("Debrief")) : pill("gray", "film", t("Recorded"))];
}
function headerSub(set) {
  if (review) {
    const r = reviewSet();
    return r ? [fmt(reviewTotal()), t("Watch Party replay"), t("voices of {names}", { names: r.players.map((p) => nameOf(p.owner)).join(", ") })].join(" · ") : "";
  }
  if (!set) return st.active_set ? t("{n} online", { n: 1 + st.peers.filter((p) => p.connected).length }) : "";
  const matches = matchEnds(items).length;
  if (set.live) return [t("This recording"), matchNow(nowM())].filter(Boolean).join(" · ");
  return [setLength(set), matches ? t(matches === 1 ? "{n} match" : "{n} matches", { n: matches }) : null].filter(Boolean).join(" · ");
}
function fitSub() {
  const s = $("psub");
  s.style.visibility = "";
  if (s.scrollWidth > s.clientWidth) s.style.visibility = "hidden";
}
new ResizeObserver(fitSub).observe($("playerpage"));
function renderHeader(set) {
  const title = review ? sessionName(review.id) : !set ? t("Live") : set.live ? t("Recording") : sessionName(set.id);
  $("ptitle").textContent = title;
  const chips = headerChips(set);
  const csig = chips.map((c) => c && c.outerHTML);
  if (changed("pchips", csig)) $("pchips").replaceChildren(...chips.filter(Boolean));
  const sub = headerSub(set);
  if ($("psub").textContent !== sub) $("psub").textContent = sub;
  fitSub();
  const a = st.active_set;
  $("reviewrec").hidden = !(a && a.voice);
  if (a && a.voice) $("reviewrec").replaceChildren(el("i", { className: "recdot" }), t("Debrief {time}", { time: fmt((nowUs() - a.start_us) / 1e6) }));
  renderParty(set);
  $("panelbtn").hidden = !set && !review;
  $("panelbtn").classList.toggle("on", panelShown());
  if (changed("fullicon", [full])) $("full").replaceChildren(icon(full ? "minimize" : "maximize"));
}
function renderParty(set) {
  const on = watchParty && st.party.on;
  const box = $("wpbox");
  box.hidden = !set && !review;
  $("shared").classList.toggle("wp", !!(on && (set || review)));
  const watching = (st.party.watching || []).filter((id) => id !== st.id);
  const status = on ? st.party.status || [] : [];
  if (changed("partystatus", [status, st.names, lang])) $("partystatus").replaceChildren(...status.map((s) => pill(s.kind === "incomplete" ? "warn" : "gray", s.kind === "catching" ? "spinner" : "download",
    s.kind === "catching" ? t("Catching up with {name}", { name: nameOf(s.who) }) : s.kind === "incomplete" ? t("{name} incomplete at {n}%", { name: nameOf(s.who), n: s.percent }) : t("{name} receiving {n}%", { name: nameOf(s.who), n: s.percent }))));
  const sig = [on, watching, st.names, st.colors, lang];
  if (!changed("wpbox", sig)) return;
  if (on) {
    const all = [st.id, ...watching];
    box.replaceChildren(el("div", { className: "wpbar" }, icon("users", "sm"), el("span", { textContent: "Watch Party" }), avatars(all), el("span", { className: "n", textContent: String(all.length) }), el("button", { className: "sm", onclick: () => setParty(false) }, icon("logout", "sm"), t("Leave"))));
  } else {
    box.replaceChildren(el("button", { className: "wpoff", onclick: () => setParty(true) }, icon("users", "sm"), el("span", { textContent: "Watch Party" }), watching.length ? avatars(watching) : null, watching.length ? el("span", { className: "muted", textContent: t("{n} watching", { n: watching.length }) }) : null, el("span", { className: "join", textContent: t("Join") })));
  }
}
function setParty(on) {
  watchParty = on;
  store("watchParty", watchParty);
  delete sigs.wpbox;
  joinParty().then(refresh);
}
const panelShown = () => (innerWidth > WIDE_PANEL ? panel : panelOpen);
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

function openSession(id) {
  nowView = false;
  const set = setById(id);
  if (set && isReview(set)) { startReview(set); share(); go("player"); return; }
  endReview();
  if (id !== selected) openSet(id);
  go("player");
}
function openNow() {
  if (st.active_set && st.active_set.voice) return openLive();
  endReview();
  nowView = true;
  const l = liveSet();
  if (l) {
    selectSet(l.id);
    nowSet = l.id;
    goLive();
  } else if (selected) selectSet(null);
  share();
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
  if (id) quiet("focus_set", { set: id });
}
function startReview(set, m = 0) {
  if (!review || review.id !== set.id) {
    review = { id: set.id, playing: false, m, at: performance.now() };
    for (const k of Object.keys(sigs)) if (["events", "views", "panel", "bands", "flow"].includes(k)) delete sigs[k];
    side = "talk";
  }
  refresh();
}
function endReview() {
  if (!review) return;
  review = null;
  for (const [key, s] of Object.entries(voices)) { drop(s); delete voices[key]; }
  delete sigs.bands;
  side = "review";
}
function reviewFlow() {
  const set = reviewSet();
  return set ? flowAt(set.party, set.start_us + reviewM() * 1e6) : null;
}
function followFlow() {
  const flow = reviewFlow();
  if (!flow) return;
  if (flow.set && flow.set !== selected && setById(flow.set)) { selectSet(flow.set); renderPlayer(); }
  if (!flow.set || flow.set !== selected) return;
  const m = (flow.wall_us - base()) / 1e6;
  const playing = review.playing && flow.playing;
  if (playing !== clock.playing || Math.abs(m - nowM()) > JUMP_MIN) setClock(playing, m);
  if (flow.rate !== rate) setRate(flow.rate);
}
function feedVoices() {
  const set = reviewSet();
  const list = playersOf(set);
  for (const key of Object.keys(voices)) if (!list.some((p) => p.key === key)) { drop(voices[key]); delete voices[key]; }
  if (!set) return;
  const wall = set.start_us + reviewM() * 1e6;
  for (const p of list) {
    const part = partFor(p, "m", wall);
    if (!part || !part.size) continue;
    if (!voices[p.key]) voices[p.key] = slot(document.body.appendChild(el("audio", { preload: "auto" })));
    const s = voices[p.key];
    feed(s, part, review.playing && s.src === srcOf(part), part, (wall - part.start_us + p.off) / 1e6);
  }
}
function syncVoices() {
  const set = reviewSet();
  if (!set) return;
  const wall = set.start_us + reviewM() * 1e6;
  for (const p of playersOf(set)) {
    const s = voices[p.key];
    const part = partFor(p, "m", wall);
    if (!s || !part) continue;
    if (pathOf(s.part) !== pathOf(part)) { feed(s, part, false, part, (wall - part.start_us + p.off) / 1e6); continue; }
    if (!switching(s)) sync(s.video, (wall - part.start_us + p.off) / 1e6, review.playing && page === "player");
  }
}
function stepSet(dir) {
  const list = st.sets.filter((s) => !s.live);
  const i = list.findIndex((s) => s.id === (review ? review.id : selected));
  const next = i < 0 ? list[0] : list[i - dir];
  if (next) { openSession(next.id); osd(sessionName(next.id)); }
}

function tick() {
  if (!st) return;
  tickLive();
  if (review) followFlow();
  const list = players();
  const tot = total(list);
  let m = nowM();
  if (clock.playing && tot && m >= tot && !review) { clock.playing = false; clock.m = tot; m = tot; }
  if (review && review.playing && reviewM() >= reviewTotal() && !(reviewSet() || {}).live) setReview(false, reviewTotal());
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
    const pinned = livePinned();
    const pictured = !!(partAt(p, wall) || {}).v;
    const behind = !pinned && !!p.file.total && p.file.have < p.file.total && target > have;
    tl.box.classList.toggle("wait", !tl.media.part || !pictured || behind);
    tl.box.classList.toggle("late", !!tl.media.part && pinned && liveClock.late.includes(p.key));
    const n = p.file.total ? Math.floor((p.file.have / p.file.total) * 100) : 0;
    const what = p.file.stuck ? ["stuck", n] : !pictured ? ["voice"] : pinned ? ["wait"] : ["here", n];
    if (changed("what/" + p.key, [...what, nameOf(p.key), lang])) tl.what.replaceChildren(...(what[0] === "stuck" ? [icon("alert", "sm"), t("{name} stopped at {n}%", { name: nameOf(p.key), n })] : what[0] === "here" ? [icon("download", "sm"), t("{name} {n}% here", { name: nameOf(p.key), n })] : what[0] === "voice" ? [icon("mic", "sm"), t("Voice only")] : [t("Waiting for {name}…", { name: nameOf(p.key) })]));
    for (const s of [tl.media, tl.mic]) if (s && s.part && !switching(s)) sync(s.video, m + shiftOf(p, s.part, b), clock.playing && !away && !(pinned && liveClock.held));
  }
  if (review) syncVoices();
  if (st.active_set) tickSide();
  if (away) return;
  const shown = shownM();
  const stot = shownTotal();
  $("clocknow").textContent = fmt(shown);
  $("clocktotal").textContent = ` / ${fmt(stot)}`;
  const frac = stot ? Math.min(1, shown / stot) : 0;
  $("seek").querySelector(".fill").style.width = frac * 100 + "%";
  $("seek").querySelector(".thumb").style.left = frac * 100 + "%";
  $("seek").querySelector(".grow").style.left = (stot ? Math.min(1, (review ? reviewTotal() : received(list)) / stot) : 1) * 100 + "%";
  const playing = review ? review.playing : clock.playing;
  $("seek").classList.toggle("paused", !playing);
  $("seek").setAttribute("aria-valuemax", Math.round(stot));
  $("seek").setAttribute("aria-valuenow", Math.round(shown));
  $("seek").setAttribute("aria-valuetext", fmt(shown));
  if (changed("playicon", [playing])) $("play").replaceChildren(icon(playing ? "pause" : "play"));
  $("play").setAttribute("aria-label", playing ? t("Pause") : t("Play"));
  renderMoment(m);
  renderFlowBar();
}

function osd(text, who) {
  $("osdtext").textContent = text;
  $("osdav").replaceChildren(who ? avatar(who, "sm") : "");
  $("osd").hidden = false;
  clearTimeout(osdTimer);
  osdTimer = setTimeout(() => { $("osd").hidden = true; }, OSD_MS);
}
function seekWho(who, m) {
  const box = $("seekwho");
  const tot = total();
  if (!tot) return;
  box.replaceChildren(avatar(who, "sm"));
  box.style.left = Math.max(0, Math.min(100, (m / tot) * 100)) + "%";
  box.style.setProperty("--c", colorOf(colorIndex(who)));
  box.hidden = false;
  clearTimeout(whoTimer);
  whoTimer = setTimeout(() => { box.hidden = true; }, SEEKWHO_MS);
}
function setUndo(fn) {
  undo = { fn, at: Date.now() };
}
function runUndo() {
  if (!undo || Date.now() - undo.at > UNDO_MS) { osd(t("Nothing to undo")); return; }
  const u = undo;
  undo = null;
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
  if (who !== st.id) {
    const name = nameOf(who);
    const where = [fmt(m), matchNow(m)].filter(Boolean).join(" · ");
    osd(cmd === "play" ? t("{name} played", { name }) : cmd === "pause" ? t("{name} paused", { name }) : t("{name} moved to {time}", { name, time: where }), who);
    seekWho(who, m);
  }
  setUndo(() => act(prev.playing ? "play" : "pause", prev.m));
  renderInk();
}
function view() {
  const set = playSet();
  if (review) {
    const r = reviewSet();
    return { set: review.id, playing: review.playing, wall_us: Math.round((r ? r.start_us : 0) + reviewM() * 1e6), at_us: nowUs(), rate, focus: null, mode: play.mode, slots: play.slots, order: play.keys };
  }
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
  return invoke("party_on", { on: watchParty, view: view() }).catch(fail);
}
const newer = (a, b) => !b || (a.seq !== b.seq ? a.seq > b.seq : a.stamp !== b.stamp ? a.stamp > b.stamp : a.origin > b.origin);
function applyParty(p) {
  if (p.set && p.set !== selected && !setById(p.set)) return;
  const first = !partyLast;
  partyLast = p;
  const mine = p.origin === st.id;
  if (quick && !mine) quick.moved = true;
  const target = p.set ? setById(p.set) : null;
  if (target && isReview(target)) {
    if (!review || review.id !== target.id) { nowView = false; startReview(target); if (page !== "player" && !mine && !first) go("player"); }
    if (typeof p.wall_us === "number") {
      const m = (p.wall_us - target.start_us) / 1e6 + (p.playing ? ((nowUs() - p.at_us) / 1e6) * (p.rate || 1) : 0);
      if (p.playing !== review.playing || Math.abs(m - reviewM()) > JUMP_MIN) setReview(p.playing, m);
    }
    return;
  }
  if (review && p.set !== review.id) endReview();
  if (p.set && p.set !== selected) {
    selectSet(p.set);
    if (nowView && !(liveSet() && liveSet().id === p.set)) nowView = false;
    if (!mine && !first) osd(t("{name} opened {set}", { name: nameOf(p.origin), set: sessionName(p.set) }), p.origin);
  }
  partyLayout = { set: p.set, mode: p.mode, slots: p.slots, order: p.order, focus: p.focus };
  if (p.rate && p.rate !== rate) setRate(p.rate);
  const set = playSet();
  if (!set || !p.set || typeof p.wall_us !== "number") return;
  if (p.live && set.live) { if (!live) goLive(); return; }
  const m = Math.max(0, (p.wall_us - base()) / 1e6 + (p.playing ? ((nowUs() - p.at_us) / 1e6) * rate : 0));
  if (mine || first || p.seq === 0) { if (p.playing !== clock.playing || Math.abs(m - nowM()) > JUMP_MIN) setClock(p.playing, m); }
  else if (p.playing !== clock.playing) apply(p.playing ? "play" : "pause", m, p.origin);
  else if (Math.abs(m - nowM()) > JUMP_MIN) apply("seek", m, p.origin);
  else setClock(p.playing, m);
}
function act(cmd, m, prev) {
  if (review) {
    const before = { playing: review.playing, m: reviewM() };
    if (cmd === "play") setReview(true, m);
    else if (cmd === "pause") setReview(false, m);
    else if (cmd === "seek") setReview(review.playing, m);
    if (cmd !== "seek") bezel(cmd === "play");
    setUndo(() => act(before.playing ? "play" : "pause", before.m));
    share();
    return;
  }
  if (!playSet()) return;
  apply(cmd, m, st.id, prev);
  share();
}
window.__TAURI__.event.listen("party", () => refresh());
const togglePlay = () => act((review ? review.playing : clock.playing) ? "pause" : "play", shownM());
const seekTo = (m) => act("seek", Math.max(0, m));
const jump = (delta) => seekTo(Math.min(shownTotal(), shownM() + delta));

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
  const matchList = matchChapters(matchEnds(items), tot, !!(set && set.live)).map((c) => ({ key: "match" + c.n, m: c.m, end: c.end, body: t("Match {n}", { n: c.n }), match: true, n: c.n }));
  return [...matchList, ...chaptersOf(items)].sort((a, b) => a.m - b.m);
}
function matchNow(m) {
  const c = matchChapters(matchEnds(items), total(), !!(playSet() && playSet().live)).find((x) => x.m <= m && m < x.end);
  return c ? t("Match {n}", { n: c.n }) : "";
}
function renderEvents() {
  const set = playSet();
  items = setItems(set);
  const words = $("filter").value.toLowerCase().split(/\s+/).filter(Boolean);
  const pool = scope === "all" ? allItems() : items;
  const order = (x, y) => (x.setId === y.setId ? (x.m ?? -1) - (y.m ?? -1) : x.setId < y.setId ? 1 : -1);
  const shown = groupDrawings(pool.filter((n) => listed(n.role) && matches(n, words)).sort(order));
  const rset = reviewSet();
  const speech = (rset ? setItems(rset) : items).filter((n) => n.role === "speech").sort((x, y) => x.m - y.m);
  renderPanelTabs(shown, speech);
  if (!review) renderTimeline(Math.round(total()));
  else if (changed("chapters", ["review"])) { $("chapters").hidden = true; $("lane").replaceChildren(); }
  renderEventList(shown, words);
  renderConvo(speech);
  renderReviewTab();
  renderFlowList();
}
function renderPanelTabs(shown, speech) {
  const notes = shown.filter((n) => n.kind === "note").length;
  const tabs = review ? [["talk", t("Talk")], ["notes", t("Notes"), notes], ["shown", t("Shown")]] : [["review", t("Review")], ["notes", t("Notes"), notes], ...(speech.length ? [["talk", t("Talk")]] : [])];
  if (!tabs.some(([k]) => k === side)) side = tabs[0][0];
  if (changed("paneltabs", [tabs, side, lang])) $("paneltabs").replaceChildren(...tabs.map(([k, label, n]) => {
    const b = el("button", { className: k === side ? "on" : "", role: "tab", ariaSelected: k === side }, label, n ? el("em", { textContent: String(n) }) : null);
    b.onclick = () => { side = k; renderEvents(); };
    return b;
  }));
  const hint = side === "review" ? [el("kbd", { textContent: "A" }), el("kbd", { textContent: "D" }), el("span", { textContent: t("Match") }), el("kbd", { textContent: "Q" }), el("kbd", { textContent: "E" }), el("span", { textContent: t("Note") })] : [];
  if (changed("panelhint", [side, lang])) $("panelhint").replaceChildren(...hint);
  $("panelhint").hidden = !hint.length;
  $("reviewlist").hidden = side !== "review";
  $("notelist").hidden = side !== "notes";
  $("filterrow").hidden = side !== "notes";
  $("convolist").hidden = side !== "talk";
  $("flowlist").hidden = side !== "shown";
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
  if (!shown.length) { $("notelist").replaceChildren(el("div", { className: "none", textContent: words.length ? t("Nothing found") : t("No notes yet") })); return; }
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
  if (!speech.length) { $("convolist").replaceChildren(el("div", { className: "none", textContent: t("No transcript yet") })); return; }
  $("convolist").replaceChildren(...speech.map((n) => {
    const row = el("div", { className: "ev talk", tabIndex: 0 }, el("span", { className: "at", textContent: fmt(n.m) }), el("span", { className: "tx" }, el("span", { className: "who" }, el("i", { style: `background:${colorOf(colorIndex(n.file || n.author))}` }), n.speaker), el("span", { textContent: n.body })));
    row.dataset.key = n.key;
    row.onclick = () => seekTo(n.m);
    return row;
  }));
}
function renderReviewTab() {
  if (side !== "review" || review) return;
  const set = playSet();
  if (!set) return;
  const tot = total();
  const list = matchChapters(matchEnds(items), tot, !!set.live);
  const notes = items.filter((n) => (n.kind === "note" || n.kind === "mark") && n.role === "moment" && n.m !== null).sort((a, b) => a.m - b.m);
  const ends = items.filter((n) => n.plugin && (n.tags || []).includes(MATCH_TAG) && n.m !== null);
  const debriefs = st.sets.filter((s) => isReview(s) && s.party.some((r) => r.shared.set === set.id));
  const now = nowM();
  const cur = list.find((c) => c.m <= now && now < c.end);
  const sig = [list, notes.map((n) => [n.key, n.m, n.body, n.author]), debriefs.map((d) => d.id), cur && cur.n, lang, st.colors];
  if (!changed("reviewtab", sig)) return;
  const noteRow = (n) => {
    const row = el("div", { className: "ev" }, el("button", { className: "at", textContent: fmt(n.m), onclick: () => seekTo(n.m) }), el("span", { className: "tx" }, ...inline(n.body), el("span", { className: "by" }, el("i", { className: "dt", style: `background:${colorOf(colorIndex(n.author))}` }), n.who)));
    return row;
  };
  const groups = list.length ? list.map((c) => ({ title: t("Match {n}", { n: c.n }), range: `${fmt(c.m)}–${fmt(c.end)}`, m: c.m, cur: cur === c, rows: [...notes.filter((n) => n.m >= c.m && n.m < c.end).map(noteRow), ...ends.filter((e) => Math.abs(e.m - c.end) < JUMP_MIN).map((e) => el("div", { className: "ev sys" }, el("button", { className: "at", textContent: fmt(e.m), onclick: () => seekTo(e.m) }), el("span", { className: "tx" }, icon("flag", "sm"), t("Match end"))))] })) : [{ title: t("Whole recording"), range: fmt(tot), m: 0, cur: true, rows: notes.map(noteRow) }];
  const out = groups.map((g) => el("div", { className: "match" + (g.cur ? " cur" : "") },
    el("div", { className: "mh" }, el("b", { textContent: g.title }), el("small", { textContent: g.range }), el("button", { className: "sm", onclick: () => { seekTo(g.m); if (!clock.playing) act("play", g.m); } }, icon("play", "sm"), t("Watch"))),
    ...(g.rows.length ? g.rows : [el("div", { className: "none", textContent: t("No notes") })])));
  if (debriefs.length) {
    out.push(el("div", { className: "h3", textContent: t("Debriefs of this recording") }));
    out.push(...debriefs.map((d) => el("button", { className: "debrief", onclick: () => openSession(d.id) }, icon("mic", "sm"), el("span", { textContent: sessionName(d.id) }), pill("accent", null, t("Debrief")), el("span", { className: "sp" }), el("span", { className: "muted", textContent: setLength(d) }), icon("play", "sm"))));
  }
  out.push(el("button", { className: "ghost tolink", onclick: () => editBoard("room") }, icon("board", "sm"), t("Write to the game plan")));
  $("reviewlist").replaceChildren(...out);
}
function stepText(step) {
  const set = setById(step.set);
  const at = set ? (step.wall_us - base(playersOf(set))) / 1e6 : 0;
  const name = nameOf(step.by);
  if (step.kind === "open") return t("{name} opened {set} at {time}", { name, set: sessionName(step.set), time: fmt(at) });
  if (step.kind === "live") return t("{name} went to Live", { name });
  if (step.kind === "play") return t("{name} played", { name });
  if (step.kind === "pause") return t("{name} paused", { name });
  return t("{name} moved to {time}", { name, time: fmt(at) });
}
function renderFlowList() {
  if (side !== "shown") return;
  const set = reviewSet();
  if (!set || !changed("flow", [set.party.length, lang, st.names])) return;
  $("flowlist").replaceChildren(...flowSteps(set.party).map((step) => {
    const m = (step.t_us - set.start_us) / 1e6;
    const row = el("div", { className: "ev" }, el("button", { className: "at", textContent: fmt(m), onclick: () => seekTo(m) }), el("span", { className: "tx", textContent: stepText(step) }));
    return row;
  }));
}
function renderFlowBar() {
  const flow = review ? reviewFlow() : null;
  $("flowbar").hidden = !flow;
  if (!flow) return;
  const set = setById(flow.set);
  const text = set ? [t("Watching: {set}", { set: sessionName(set.id) }), matchNow(nowM()), fmt(nowM())].filter(Boolean).join(" · ") : t("Watching live");
  if ($("flowtext").textContent !== text) $("flowtext").textContent = text;
  $("flowopen").hidden = !set;
}
function renderBands(tot) {
  const set = reviewSet();
  const bands = set ? flowBands(set.party, set.start_us, set.end_us || nowUs()) : [];
  const ids = [...new Set(bands.map((b) => b.set))];
  if (!changed("bands", [bands, Math.round(tot), lang])) return;
  const span = set ? (set.end_us || nowUs()) - set.start_us : 1;
  const colorFor = (id) => `var(--band${ids.indexOf(id) % 4})`;
  $("bands").replaceChildren(...bands.map((b) => {
    const e = el("i", { title: sessionName(b.set) });
    Object.assign(e.style, { left: ((b.from - set.start_us) / span) * 100 + "%", width: ((b.to - b.from) / span) * 100 + "%", background: colorFor(b.set) });
    return e;
  }));
  $("bandlabels").hidden = !bands.length;
  $("bandlabels").replaceChildren(...ids.map((id) => el("span", {}, el("i", { style: `background:${colorFor(id)}` }), t("Watching {set}", { set: sessionName(id) }))), el("span", { className: "sp" }), set ? el("span", { className: "muted", textContent: t("Voices: {names}", { names: set.players.map((p) => nameOf(p.owner)).join(" · ") }) }) : "");
}
function timeText(n) {
  if (n.m === null) return t("Whole recording");
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
  if (review) match = null;
  $("matchchip").hidden = !match;
  if (match && changed("matchchip", [match])) $("matchchip").replaceChildren(el("span", { textContent: "· " + match }), icon("chevron-right", "sm"));
  const notes = activeAt(items.filter((n) => n.role === "moment" && n.text && !n.plugin), m).slice(-OVERLAY_MAX);
  const typists = (watchParty && st.party.on ? st.party.typing || [] : []).filter((id) => id !== st.id);
  if (changed("overlay", [notes.map((n) => [n.key, n.m, n.body, n.author]), typists.map(nameOf), st.colors, lang])) $("overlay").replaceChildren(
    ...notes.map((n) => el("p", {}, avatar(n.author, "sm"), ...inline(n.body))),
    ...typists.map((id) => el("p", { className: "typing", textContent: t("{name} is typing…", { name: nameOf(id) }) })));
  const rset = reviewSet();
  const rm = rset ? reviewM() : m;
  const speech = subtitles ? activeAt((rset ? setItems(rset) : items).filter((n) => n.role === "speech"), rm) : [];
  for (const key of play.keys) {
    const tl = play.tiles[key];
    if (!tl) continue;
    const lines = rset ? [] : speech.filter((n) => (n.file || n.author) === key).map((n) => n.body);
    tl.sub.hidden = !lines.length;
    if (tl.sub.textContent !== lines.join("\n")) tl.sub.textContent = lines.join("\n");
  }
  if (rset) {
    const text = speech.map((n) => `${n.speaker}: ${n.body}`).join("\n");
    const tl = play.tiles[play.view ? play.keys[play.view - 1] : play.keys[0]];
    if (tl) { tl.sub.hidden = !text; if (tl.sub.textContent !== text) tl.sub.textContent = text; }
  }
  const current = speech.length ? speech[speech.length - 1].key : null;
  for (const row of $("convolist").children) {
    const on = row.dataset.key === current;
    if (on && !row.classList.contains("cur") && (review ? review.playing : clock.playing)) row.scrollIntoView({ block: "nearest" });
    row.classList.toggle("cur", on);
  }
}
function filterTag(tag) {
  if (!playSet()) return;
  $("filter").value = "#" + tag;
  side = "notes";
  showPanel(true);
  if (page !== "player") go("player");
  renderEvents();
}
function openLink(link) {
  if (!setById(link.set)) { flash(t("That recording is not on this PC.")); return; }
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
function matchMenu(anchor) {
  const list = chapters(total()).filter((c) => c.match);
  openMenu(list.map((c) => ({ label: c.body, right: `${fmt(c.m)}–${fmt(c.end)}`, run: () => seekTo(c.m) })), anchor);
}
function speedMenu(anchor) {
  openMenu(SPEEDS.map((s) => ({ label: `${s}×`, check: s === rate, run: () => { setRate(s); share(); } })), anchor);
}

function openQuick() {
  const set = playSet();
  if (!set || review) return;
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
  if (!keepFocus || m.hidden) {
    menuReturn = document.activeElement;
    const t = at instanceof Event ? at.currentTarget : at;
    anchorMenu(t instanceof Element && t.hasAttribute("aria-haspopup") ? t : null);
  }
  m.replaceChildren(...list.map((it) => {
    if (it.separator) return el("hr");
    if (it.heading) return el("div", { className: "heading", textContent: it.heading });
    if (it.element) return it.element;
    const lead = it.check !== undefined ? el("span", { className: "ck" }, it.check ? icon("check", "sm") : null) : it.icon ? icon(it.icon, "sm") : null;
    const b = el("button", { role: "menuitem", disabled: !it.run || !!it.disabled, className: it.danger ? "danger" : "" }, lead, el("span", { className: "label", textContent: it.label }), it.right !== undefined ? el("span", { className: "right", textContent: it.right }) : null, it.more ? icon("chevron-right", "sm") : null);
    b.onclick = (e) => { if (it.more) { it.run(b); return; } closeMenu(); it.run(e); };
    return b;
  }));
  m.hidden = false;
  const r = at instanceof Event && at.clientX ? { left: at.clientX, bottom: at.clientY, top: at.clientY, right: at.clientX } : (at instanceof Event ? at.currentTarget || at.target : at).getBoundingClientRect();
  const sideways = !(at instanceof Event) && at.closest && at.closest(".menu");
  m.style.left = Math.max(4, Math.min(sideways ? r.right + 4 : r.left, innerWidth - m.offsetWidth - 4)) + "px";
  const below = sideways ? r.top : r.bottom + 4;
  m.style.top = (below + m.offsetHeight > innerHeight - 4 ? Math.max(4, r.top - m.offsetHeight - 4) : below) + "px";
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
document.addEventListener("mousedown", (e) => {
  if (!$("menu").contains(e.target)) closeMenu();
  if (!$("pop").contains(e.target)) closePop();
});
function closePop() {
  if ($("pop").hidden) return;
  $("pop").hidden = true;
  if (popClosed) { const f = popClosed; popClosed = null; f(); }
}
let popClosed = null;
function stageMenu(e) {
  const set = playSet();
  if (!set) return;
  openMenu([
    ...(review ? [] : [{ label: t("Add note"), icon: "note", right: "C", run: openQuick }]),
    { label: t("Copy link to this moment"), icon: "link", run: () => navigator.clipboard.writeText(momentLink(set.id, nowM(), play.view ? play.keys[play.view - 1] : null)).then(() => osd(t("Link copied"))).catch(fail) },
    { label: t("Clear drawing"), icon: "eraser", right: "X", run: () => clearInk(false) },
  ], e);
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
  for (const p of plugins.filter((x) => x.download)) list.push({ key: "download:" + p.id, level: "info", icon: "spinner", spin: true, text: downloadLine(t("Downloading {name}…", { name: t(p.name) }), p.download), sticky: true });
  if (st.ffmpeg === "failed") list.push({ key: "ffmpegfail", level: "error", icon: "fail", text: t("Couldn't download the video tools. Check your connection."), action: { label: t("Retry"), run: () => run("retry_ffmpeg", null) } });
  if (st.notice) list.push({ key: "n:" + st.notice, level: "warn", icon: "alert", text: t(st.notice) });
  for (const p of st.peers.filter((x) => x.connected && x.other_version)) list.push({ key: "v:" + p.id, level: "warn", icon: "alert", text: t("{name} is on a different version. Both of you need the latest one.", { name: nameOf(p.id) }) });
  if (available && installAfter) list.push({ key: "after", level: "info", icon: "download", text: t("Version {version} installs after recording", { version: available.version }), action: { label: t("Cancel"), run: () => { installAfter = false; renderNotices(); } }, sticky: true });
  else if (available) list.push({ key: "update", level: "info", icon: "download", text: t("Version {version} is available", { version: available.version }), action: st.recording ? { label: t("Install after recording"), run: () => { installAfter = true; renderNotices(); } } : { label: t("Install"), primary: true, run: installUpdate } });
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
function exportFolder(set) {
  if (!set) { flash(t("Open a recording first")); return; }
  invoke("export_check").then(() => dialog("pick_folder")).then((folder) => {
    if (!folder) return;
    exporting = { set: set.id, percent: 0 };
    refresh();
    return invoke("export_folder", { room: set.room, set: set.id, folder }).then((r) => {
      if (r.skipped.length) flash(t("Exporting without {names}: not on this PC yet", { names: r.skipped.join(", ") }));
    }).catch((e) => { if (exporting) { exporting = null; fail(e); refresh(); } });
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
    const version = await invoke("check_update");
    available = version ? { version } : null;
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
  return st.recording || party || clock.playing || (review && review.playing) || !!exporting || !!(jobInfo && jobInfo.running);
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
const steps = () => [renderShell, renderStart, renderSide, renderNotices, renderPlayer, renderEvents, renderTools, renderRoom, renderBoard, renderRecovery, renderOpenDialogs];
async function update() {
  st = await invoke("state");
  clockShift = st.now_us - Date.now() * 1000;
  idx = indexOf(st);
  titles = Object.fromEntries(st.sets.filter((x) => x.title).map((x) => [x.id, x.title]));
  importedIds = new Set(st.sets.flatMap((x) => x.players.filter((p) => p.imported).map((p) => p.owner)));
  if (lang !== st.lang) { lang = st.lang; setLanguage(lang); for (const k of Object.keys(sigs)) delete sigs[k]; }
  if (!ready) { ready = true; pollPlugins(); }
  if (installAfter && available && !busy() && !askingUpdate) askUpdate();
  if (selected && !setById(selected)) selected = null;
  if (review && !reviewSet()) endReview();
  const l = liveSet();
  const voice = st.active_set && st.active_set.voice;
  if (nowView && l && !voice && selected !== l.id) { selectSet(l.id); nowSet = l.id; goLive(); share(); }
  if (nowView && !l && nowSet) { nowView = false; nowSet = null; }
  const roomId = st.room ? st.room.id : "";
  if (partyRoom !== roomId) { partyRoom = roomId; cancelBoard(); await joinParty(); st = await invoke("state"); idx = indexOf(st); }
  if (!drag && watchParty && st.party.on && newer(st.party.shared, partyLast)) applyParty(st.party.shared);
  for (const step of steps()) try { step(); } catch (e) { console.error(e); }
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
  window.__TAURI__.window.getCurrentWindow().setFullscreen(full).catch(warn);
  wake();
}
function wake() {
  $("playerpage").classList.remove("idle");
  clearTimeout(idleTimer);
  if (full) idleTimer = setTimeout(() => { if (clock.playing || (review && review.playing)) $("playerpage").classList.add("idle"); }, IDLE_MS);
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
  endReview();
  nowView = false;
  if (selected) selectSet(null);
  share();
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

function ask(title, action, danger, text) {
  const d = $("confirm");
  $("confirmtitle").textContent = title;
  $("confirmtext").hidden = !text;
  $("confirmtext").textContent = text || "";
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
function chime(action) {
  if (!sessionSound) return;
  try {
    chimeAudio = chimeAudio || new AudioContext();
    CHIME[action].forEach((hz, i) => {
      const osc = chimeAudio.createOscillator();
      const gain = chimeAudio.createGain();
      const at = chimeAudio.currentTime + i * CHIME_NOTE_S;
      osc.frequency.value = hz;
      gain.gain.setValueAtTime(CHIME_GAIN, at);
      gain.gain.exponentialRampToValueAtTime(0.0001, at + CHIME_NOTE_S);
      osc.connect(gain).connect(chimeAudio.destination);
      osc.start(at);
      osc.stop(at + CHIME_NOTE_S);
    });
  } catch (e) {
    warn(e);
  }
}
function setRate(r) {
  clock.m = nowM();
  clock.at = performance.now();
  if (review) { review.m = reviewM(); review.at = performance.now(); }
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
const EDITABLE = "input:not([type=range]):not([type=checkbox]):not([type=radio]):not([type=button]), textarea";
function editMenu(e, field) {
  const has = field ? field.selectionStart !== field.selectionEnd : String(getSelection()).length > 0;
  const editable = !!field && !field.readOnly && !field.disabled;
  const paste = () => navigator.clipboard.readText().then((text) => { field.setRangeText(text, field.selectionStart, field.selectionEnd, "end"); field.dispatchEvent(new Event("input", { bubbles: true })); }).catch(warn);
  openMenu([
    ...(field ? [{ label: t("Cut"), icon: "x", disabled: !editable || !has, run: () => document.execCommand("cut") }] : []),
    { label: t("Copy"), icon: "copy", disabled: !has, run: () => document.execCommand("copy") },
    ...(field ? [{ label: t("Paste"), icon: "download", disabled: !editable, run: paste }, { label: t("Select all"), run: () => field.select() }] : []),
  ], e);
}
document.addEventListener("contextmenu", (e) => {
  if (e.defaultPrevented) return;
  e.preventDefault();
  const field = e.target.closest ? e.target.closest(EDITABLE) : null;
  if (field || String(getSelection()).length) editMenu(e, field);
});
async function toggleRecording(kind) {
  if (!st) return;
  const ending = !!st.active_set;
  const debrief = !ending && (kind ? kind === "debrief" : reviewing());
  const title = ending ? t("Stop recording for everyone?") : debrief ? t("Record the debrief for everyone?") : t("Start recording for everyone?");
  const action = ending ? t("Stop recording") : debrief ? t("Record debrief") : t("Start recording");
  const text = ending ? t("Recording stops for everyone.") : debrief ? t("Only the voice and the viewing flow are recorded.") : t("Everyone in the room starts recording.");
  if (!(await ask(title, action, ending, text))) return;
  if (!ending) return run("start_recording", { debrief });
  const { id, start_us } = st.active_set;
  const seconds = (nowUs() - start_us) / 1e6;
  await run("stop_recording");
  if (discardable(seconds) && (await ask(t("Discard this recording?"), t("Discard"), true))) run("discard_set", { room: st.room.id, set: id });
}
const current = () => (playSet() ? play : rec);

$("play").onclick = togglePlay;
$("back").onclick = () => jump(-STEP.normal);
$("fwd").onclick = () => jump(STEP.normal);
$("addnote").onclick = openQuick;
$("live").onclick = () => { if (!playSet()) return; goLive(); share(); renderPlayer(); };
$("speed").onclick = (e) => speedMenu(e.currentTarget);
$("matchchip").onclick = (e) => matchMenu(e.currentTarget);
$("mixbtn").onclick = (e) => openMix(e.currentTarget);
$("volume").oninput = () => setVolume(Number($("volume").value));
$("mute").onclick = toggleMute;
$("cc").onclick = () => { subtitles = !subtitles; store("subtitles", subtitles); renderPlayer(); };
$("panelbtn").onclick = togglePanel;
$("full").onclick = toggleFull;
$("layoutplay").onclick = (e) => pickLayout(current(), e.currentTarget);
$("scope").onchange = () => { scope = $("scope").value; renderEvents(); };
$("flowopen").onclick = () => { const flow = reviewFlow(); if (!flow || !flow.set) return; const m = nowM(); endReview(); openSession(flow.set); openSet(flow.set, m); };
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
  if ((!playSet() && !review) || drag) return;
  drag = review ? { review: true } : { playing: clock.playing, m: nowM() };
  bar.setPointerCapture(e.pointerId);
  const move = (x) => { live = false; if (review) setReview(review.playing, seekFrac(x) * reviewTotal()); else setClock(clock.playing, seekFrac(x) * total()); };
  move(e.clientX);
  bar.onpointermove = (m) => move(m.clientX);
  bar.onpointerup = bar.onpointercancel = () => {
    bar.onpointermove = bar.onpointerup = bar.onpointercancel = null;
    const from = drag;
    drag = null;
    if (from.review) share();
    else act("seek", nowM(), from);
  };
};
const peek = { video: el("video", { muted: true, preload: "auto" }), want: null };
peek.video.onseeked = () => {
  const c = $("peek").querySelector("canvas");
  c.getContext("2d").drawImage(peek.video, 0, 0, c.width, c.height);
  if (peek.want !== null && Math.abs(peek.want - peek.video.currentTime) > SEEK_TOLERANCE) peek.video.currentTime = peek.want;
};
function peekAt(x) {
  if (review) { $("peek").hidden = false; $("peek").querySelector("canvas").hidden = true; $("peek").style.left = Math.max(0, Math.min($("seek").clientWidth, x - $("seek").getBoundingClientRect().left)) + "px"; $("peektime").textContent = fmt(seekFrac(x) * reviewTotal()); $("peekchapter").textContent = ""; return; }
  $("peek").querySelector("canvas").hidden = false;
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
  closePop();
  if (typing()) document.activeElement.blur();
  else if (full) toggleFull();
  else if (boardFull) toggleBoardFull();
  else if (panelOpen) showPanel(false);
}
document.addEventListener("keydown", (e) => {
  if (composing(e) || listening) return;
  if (!$("menu").hidden) { menuKey(e); return; }
  const ctrl = e.ctrlKey && !e.altKey && !e.metaKey;
  if (ctrl && e.key === ",") { e.preventDefault(); openSettings(); return; }
  if (document.querySelector("dialog[open]")) return;
  if (!inRoom()) return;
  if (ctrl && ["1", "2", "3", "4", "b", "r"].includes(e.key.toLowerCase())) { e.preventDefault(); return; }
  if (ctrl && e.key === "Enter" && onBoard() && boardEditing) { e.preventDefault(); saveBoard(); return; }
  if (ctrl && e.key.toLowerCase() === "z" && !typing()) { e.preventDefault(); runUndo(); return; }
  if (e.key === "Escape") { escapeKey(); return; }
  if (typing() || nativeKey(e) || e.metaKey || e.altKey) return;
  const player = page === "player" && (!!playSet() || !!review);
  const key = e.key.normalize("NFKC");
  const k = key.length === 1 ? key.toLowerCase() : key;
  let action = null;
  if (e.ctrlKey) action = player && (key === "ArrowLeft" || key === "ArrowRight") ? () => jump(key === "ArrowLeft" ? -STEP.huge : STEP.huge) : null;
  else if (key === "?") action = () => openSettings("shortcuts");
  else if (onBoard() && !boardEditing && k === "f") action = toggleBoardFull;
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

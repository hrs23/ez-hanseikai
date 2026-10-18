const { invoke, convertFileSrc } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

const STEP = { small: 1, normal: 5, big: 10, huge: 60 };
const VOLUME_STEP = 0.05;
const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2];
const MAX_KEYED = 6;
const TICK_MS = 150;
const REFRESH_MS = 1000;
const IDLE_MS = 2000;
const OSD_MS = 2000;
const NOTICE_MS = 5000;
const COPIED_MS = 1500;
const UPDATE_CHECK_DELAY_MS = 4000;
const LIVE_LAG = 1.5;
const LIVE_SLACK = 3 * LIVE_LAG;
const RELOAD_AHEAD = 1;
const RELOAD_MIN_MS = 1000;
const RELOAD_GROWTH = 300000;
const RETRY_MS = 3000;
const FRAME_WAIT_MS = 1000;
const NEXT_TIMEOUT_MS = 10000;
const SEEK_TOLERANCE = 0.05;
const DRIFT_MAX = 0.4;
const CONTROL_REPEAT_MS = 4000;
const PRESENCE_MS = 2.5 * CONTROL_REPEAT_MS;
const PEAK_HOLD_MS = 300;
const OFFLINE_FAILURES = 2;
const SKELETON_ROWS = 3;
const NOTE_STEP_GAP = 0.5;
const CODEC_PROBE = { width: 1280, height: 720, bitrate: 6000000, framerate: 30 };
const GAP = { record: 8, player: 4 };

const stage = (el, kind) => ({ el, kind, tiles: {}, keys: [], mode: "grid", view: 0, last: 1, free: [], ring: null });
const rec = stage($("rectiles"), "record");
const play = stage($("playtiles"), "player");
const clock = { playing: false, m: 0, at: 0 };
const preview = { box: $("preview"), video: null };
const presence = {};
const peaks = { game: { v: 0, at: 0 }, mic: { v: 0, at: 0 } };
let st = null;
let ready = false;
let tab = "record";
let selected = null;
let rate = 1;
let volume = 1;
let muted = false;
let live = false;
let watchParty = true;
let lastCtl = null;
const sigs = {};
let noteItems = [];
let osdTimer = null;
let osdUndo = null;
let idleTimer = null;
let flashText = "";
let flashInfo = false;
let flashTimer = null;
let dismissed = "";
let pending = null;
let updateText = "";
let menuReturn = null;
let exporting = null;

function el(tag, props = {}, ...children) {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...children.filter((c) => c !== null && c !== undefined && c !== false));
  return e;
}
function svg(tag, attrs = {}, ...children) {
  const e = document.createElementNS("http://www.w3.org/2000/svg", tag);
  for (const [k, v] of Object.entries(attrs)) e.setAttribute(k, v);
  e.append(...children);
  return e;
}
function fmt(sec) {
  sec = Math.max(0, Math.floor(sec));
  const h = Math.floor(sec / 3600);
  const m = String(Math.floor((sec % 3600) / 60)).padStart(h ? 2 : 1, "0");
  return `${h ? h + ":" : ""}${m}:${String(sec % 60).padStart(2, "0")}`;
}
const gb = (bytes) => (bytes / 1e9).toFixed(1) + " GB";
const nowUs = () => Date.now() * 1000;
const setTime = (id) => { const [d, t] = id.split(" "); return t ? `${d.slice(5).replace("-", "/")} ${t.replaceAll("-", ":").slice(0, 5)}` : id; };
const peerOf = (id) => st.peers.find((p) => p.id === id);
const nameOf = (id) => st.names[id] || id.slice(0, 10);
function changed(name, value) {
  const sig = JSON.stringify(value);
  if (sigs[name] === sig) return false;
  sigs[name] = sig;
  return true;
}
function press(e, on) {
  e.classList.toggle("on", on);
  e.setAttribute("aria-pressed", on);
}
const typing = () => { const a = document.activeElement; return a && (a.tagName === "SELECT" || a.tagName === "TEXTAREA" || (a.tagName === "INPUT" && a.type !== "range") || a.isContentEditable); };

function ordered(present, saved) {
  const keep = (saved || []).filter((k) => present.includes(k));
  return [...keep, ...present.filter((k) => !keep.includes(k))];
}

function tile(s, key) {
  if (s.tiles[key]) return s.tiles[key];
  const t = { key, video: el("video", { muted: true, preload: "auto" }), name: el("span"), dot: el("i", { className: "dot" }), badge: el("span", { className: "badge" }), who: el("div", { className: "who" }), what: el("div", { className: "what" }) };
  t.box = el("div", { className: "tile", draggable: true, tabIndex: 0 }, t.video, el("div", { className: "waitbox" }, t.who, t.what), el("div", { className: "lab" }, t.dot, t.name), t.badge);
  const focus = () => { const i = s.keys.indexOf(key) + 1; setView(s, s.view === i ? 0 : i); };
  t.box.onclick = focus;
  t.box.onkeydown = (e) => { if (e.key === "Enter") focus(); };
  t.box.ondblclick = toggleFull;
  t.box.ondragstart = (e) => e.dataTransfer.setData("text/plain", key);
  t.box.ondragover = (e) => { e.preventDefault(); t.box.classList.add("over"); };
  t.box.ondragleave = () => t.box.classList.remove("over");
  t.box.ondrop = (e) => { e.preventDefault(); t.box.classList.remove("over"); const from = e.dataTransfer.getData("text/plain"); if (from && from !== key && s.tiles[from]) swap(s, from, key); };
  s.el.append(t.box);
  return (s.tiles[key] = t);
}
function prune(s, keys) {
  for (const key of Object.keys(s.tiles)) if (!keys.includes(key)) { s.tiles[key].box.remove(); delete s.tiles[key]; }
}
function place(s) {
  const n = s.keys.length;
  if (s.view > n) s.view = 0;
  const focus = s.view ? s.keys[s.view - 1] : null;
  const order = focus ? [focus, ...s.keys.filter((k) => k !== focus)] : s.keys;
  const mode = focus && (s.mode === "grid" || s.mode === "row" || !layoutFits(s.mode, n)) ? (n > 1 ? "bottom" : "grid") : s.mode;
  const r = rects(n, mode, s.el.clientWidth, s.el.clientHeight, GAP[s.kind]);
  const at = (e, b) => Object.assign(e.style, { left: b.x + "px", top: b.y + "px", width: b.w + "px", height: b.h + "px" });
  order.forEach((key, i) => {
    const t = s.tiles[key];
    if (!t) return;
    at(t.box, r.tiles[i]);
    t.box.classList.toggle("active", key === focus);
    t.badge.textContent = s.keys.indexOf(key) + 1;
  });
  while (s.free.length > r.free.length) s.free.pop().remove();
  while (s.free.length < r.free.length) s.free.push(s.el.appendChild(el("div", { className: s.kind === "record" ? "tile free" : "tile blank", textContent: s.kind === "record" ? "Free" : "" })));
  r.free.forEach((b, i) => at(s.free[i], b));
  if (s.ring) {
    const all = [...r.tiles, ...r.free];
    s.ring.hidden = !watchParty || !all.length;
    if (all.length) {
      const x = Math.min(...all.map((b) => b.x)), y = Math.min(...all.map((b) => b.y));
      at(s.ring, { x, y, w: Math.max(...all.map((b) => b.x + b.w)) - x, h: Math.max(...all.map((b) => b.y + b.h)) - y });
    }
  }
}
play.ring = play.el.appendChild(el("div", { className: "ring" }));
for (const s of [rec, play]) new ResizeObserver(() => place(s)).observe(s.el);

function setView(s, view) {
  s.view = Math.max(0, Math.min(view, s.keys.length));
  if (s.view) s.last = s.view;
  place(s);
  if (s === play) { applyAudio(); renderViews(); logLayout(); }
}
function swap(s, a, b) {
  const list = [...s.keys];
  const i = list.indexOf(a), j = list.indexOf(b);
  [list[i], list[j]] = [list[j], list[i]];
  s.keys = list;
  saveLayout(s, s.mode);
}
function saveLayout(s, mode) {
  s.mode = mode;
  if (s === rec) {
    const old = (st.layout && st.layout.order) || [];
    st.layout = { mode, order: [...s.keys, ...old.filter((id) => !s.keys.includes(id))] };
    invoke("set_layout", { layout: st.layout }).catch(() => {});
  } else {
    const set = playSet();
    if (set) try { localStorage.setItem("layout/" + set.id, JSON.stringify({ mode, order: s.keys })); } catch {}
  }
  place(s);
  renderLayoutButtons();
  if (s === play) logLayout();
}
const fromSaved = (l) => ({ mode: (l && l.mode) || "grid", order: ((l && l.order) || []).map((id) => (id === "me" ? st.id : id)) });
function playLayout(set) {
  let own = null;
  try { own = JSON.parse(localStorage.getItem("layout/" + set.id)); } catch {}
  return fromSaved(own || set.layout);
}
function logLayout() {
  const set = playSet();
  const size = (key, side) => Math.round(play.tiles[key] ? play.tiles[key].box[side] : 0);
  if (set) invoke("log_layout", { layout: { set: set.id, mode: play.view ? "focus" : play.mode, tiles: play.keys.map((key, i) => ({ id: key, index: i, w: size(key, "clientWidth"), h: size(key, "clientHeight") })) } }).catch(() => {});
}

function swapIn(t, file, at) {
  const old = t.video;
  const v = el("video", { muted: old.muted, volume: old.volume, preload: "auto" });
  v.playbackRate = old.playbackRate;
  t.next = v;
  t.size = file.size;
  t.loadedAt = Date.now();
  v.onloadedmetadata = () => { const want = at ?? old.currentTime; v.currentTime = Math.min(want, v.duration || want); };
  v.onseeked = () => {
    if (t.next !== v) return;
    v.onseeked = null;
    if (!old.paused) v.play().catch(() => {});
    const done = () => {
      if (t.next !== v) return;
      t.loadSec = (Date.now() - t.loadedAt) / 1000;
      t.next = null;
      old.remove();
      t.video = v;
    };
    if (v.requestVideoFrameCallback) v.requestVideoFrameCallback(done);
    setTimeout(done, FRAME_WAIT_MS);
  };
  v.onerror = () => { if (t.next === v) { t.next = null; v.remove(); } };
  v.src = t.src + "?r=" + Date.now();
  old.before(v);
}
function feed(t, file, following) {
  const src = convertFileSrc(file.path);
  if (t.next) {
    if (Date.now() - t.loadedAt < NEXT_TIMEOUT_MS) return;
    t.next.remove();
    t.next = null;
  }
  if (t.src !== src) {
    const v = t.video;
    const playing = following && t.src && !v.paused && !v.error;
    if (playing && !t.complete) { t.complete = true; swapIn(t, { size: t.size }); return; }
    if (playing && !v.ended && isFinite(v.duration) && v.duration - v.currentTime > (t.loadSec || 0)) return;
    const first = !t.src;
    t.src = src;
    t.complete = false;
    if (first) { t.size = file.size; t.loadedAt = Date.now(); t.video.src = src; }
    else swapIn(t, file, 0);
    return;
  }
  const v = t.video;
  const settled = t.seen === file.size;
  t.seen = file.size;
  const grew = file.size > t.size;
  const left = isFinite(v.duration) ? v.duration - v.currentTime : Infinity;
  const since = Date.now() - t.loadedAt;
  const stuck = v.error && since > RETRY_MS;
  const due = grew && since > RELOAD_MIN_MS && (following ? left < (t.loadSec || 0) + RELOAD_AHEAD || v.ended : v.paused && (settled || file.size > t.size + RELOAD_GROWTH));
  if (stuck || due) swapIn(t, file);
}
function follow(t) {
  const v = t.video;
  if (!v || v.readyState < 1 || !isFinite(v.duration)) return;
  const lag = LIVE_LAG + 2 * (t.loadSec || 0);
  if (v.duration - v.currentTime > lag + LIVE_SLACK) v.currentTime = Math.max(0, v.duration - lag);
  if (v.paused && v.currentTime < v.duration - SEEK_TOLERANCE) v.play().catch(() => {});
}

function recOf(id) {
  const set = st.sets.find((s) => s.live);
  return !!(set && set.players.some((p) => p.owner === id && p.recording));
}
function recordPlayers() {
  const files = { [st.id]: st.capture.live };
  for (const p of st.peers) if (p.live) files[p.id] = p.live;
  const keys = [st.id, ...st.peers.filter((p) => p.connected || files[p.id]).map((p) => p.id)];
  return keys.map((key) => ({ key, file: files[key] && files[key].size > 0 ? files[key] : null, recording: recOf(key) }));
}
function dotOf(key, recording) {
  if (recording) return "dot rec";
  if (key === st.id) return "dot" + (st.capture.sharing ? " live" : "");
  const p = peerOf(key);
  if (!p || !p.connected) return "dot";
  return "dot" + (p.path === "relay" ? " relay" : p.sharing ? " live" : "");
}
function renderRecord() {
  const list = recordPlayers();
  const lay = fromSaved(st.layout);
  rec.keys = ordered(list.map((p) => p.key), lay.order);
  rec.mode = lay.mode;
  prune(rec, rec.keys);
  for (const p of list) {
    const t = tile(rec, p.key);
    t.name.textContent = nameOf(p.key);
    t.who.textContent = nameOf(p.key);
    t.what.textContent = p.key === st.id ? (st.capture.source ? (st.share ? "Starting" : "Sharing off") : "No source") : "connected";
    t.dot.className = dotOf(p.key, p.recording);
    t.box.classList.toggle("wait", !p.file);
    if (p.file) feed(t, p.file, true);
  }
  place(rec);
  const own = list[0].file;
  if (own) {
    if (!preview.video) preview.video = preview.box.appendChild(el("video", { muted: true, preload: "auto" }));
    feed(preview, own, true);
  } else if (preview.video) { preview.video.remove(); preview.video = null; preview.src = null; }
  renderMembers();
}

function renderMembers() {
  const rows = [{ key: st.id, name: `${st.name || "You"}`, you: true, state: st.recording ? "Recording" : "Idle" }, ...st.peers.map((p) => ({
    key: p.id, name: nameOf(p.id), path: p.connected ? p.path : "", message: p.message,
    state: !p.connected ? (p.failures > OFFLINE_FAILURES ? "Offline" : "Connecting...") : recOf(p.id) ? "Recording" : "Idle",
  }))];
  if (!changed("members", [rows, rows.map((r) => dotOf(r.key, recOf(r.key)))])) return;
  $("members").replaceChildren(...rows.map((r) => {
    const row = el("div", { className: "member", tabIndex: r.you ? -1 : 0 },
      el("i", { className: dotOf(r.key, recOf(r.key)) }),
      el("span", { className: "name", textContent: r.name }),
      r.you ? el("span", { className: "muted", textContent: "you" }) : null,
      r.path ? el("span", { className: "muted path", textContent: r.path }) : null,
      r.message ? el("span", { className: "warn", textContent: r.message }) : null,
      el("span", { className: "state", textContent: r.state }));
    if (!r.you) row.oncontextmenu = (e) => { e.preventDefault(); openMenu([{ label: "Remove from room", danger: true, run: () => invoke("remove_peer", { id: r.key }) }], e); };
    return row;
  }), ...(st.peers.length ? [] : [el("div", { className: "nobody", textContent: "Nobody here yet" })]));
}

function renderRooms() {
  const inRoom = !!st.room;
  $("room").hidden = !inRoom;
  $("norooms").hidden = inRoom;
  if (!inRoom && $("rooms").parentElement !== $("norooms")) $("norooms").append($("rooms"));
  if (inRoom && $("rooms").parentElement === $("norooms")) $("roomsdlg").append($("rooms"));
  if (!changed("rooms", st.rooms)) return;
  $("roomlist").replaceChildren(...st.rooms.map((r) => {
    const b = el("button", { className: "roomrow" + (r.active ? " on" : "") }, el("span", { textContent: r.name }), el("span", { className: "muted", textContent: r.members }));
    b.onclick = () => { if (!r.active) invoke("switch_room", { id: r.id }).then(() => $("roomsdlg").close()).catch(flash); };
    b.oncontextmenu = (e) => { e.preventDefault(); openMenu([{ label: "Leave room", danger: true, run: () => invoke("leave_room", { id: r.id }) }], e); };
    return b;
  }));
}

function playSet() { return (st && st.sets.find((s) => s.id === selected)) || null; }
function players() {
  const set = playSet();
  if (!set) return [];
  return set.players.filter((p) => p.size > 0).map((p) => ({ key: p.owner, start: p.start_us, off: p.offset_us || 0, file: p, recording: p.recording }));
}
function base(list = players()) {
  return list.length ? Math.min(...list.map((p) => p.start - p.off)) : nowUs();
}
const deltaOf = (p, b) => (b + p.off - p.start) / 1e6;
function total(list = players()) {
  const b = base(list);
  const ends = list.map((p) => ({ p, v: play.tiles[p.key] && play.tiles[p.key].video })).filter((x) => x.v && isFinite(x.v.duration)).map((x) => x.v.duration - deltaOf(x.p, b));
  return ends.length ? Math.max(...ends) : 0;
}
const nowM = () => (clock.playing ? clock.m + ((performance.now() - clock.at) / 1000) * rate : clock.m);
const wallOf = (m) => Math.round(base() + m * 1e6);
function setClock(playing, m) {
  clock.playing = playing;
  clock.m = Math.max(0, m);
  clock.at = performance.now();
  tick();
}
function applyAudio() {
  const n = play.keys.length;
  play.keys.forEach((key, i) => {
    const t = play.tiles[key];
    if (!t) return;
    for (const v of [t.video, t.next]) if (v) {
      v.muted = muted || !(play.view === i + 1 || (play.view === 0 && n === 1));
      v.volume = volume;
      v.playbackRate = rate;
    }
  });
  $("mute").classList.toggle("alt", muted || volume === 0);
  if (document.activeElement !== $("volume")) $("volume").value = volume;
  $("speed").value = rate;
}

function renderPlayer() {
  const set = playSet();
  const list = players();
  const lay = set ? playLayout(set) : fromSaved(null);
  play.keys = ordered(list.map((p) => p.key), lay.order);
  play.mode = lay.mode;
  prune(play, play.keys);
  for (const p of list) {
    const t = tile(play, p.key);
    t.name.textContent = nameOf(p.key);
    t.dot.className = p.recording ? "dot rec" : "dot";
    t.dot.hidden = !p.recording;
    feed(t, p.file, clock.playing);
  }
  place(play);
  applyAudio();
  renderViews();
  $("empty").hidden = !!set;
  $("playtiles").hidden = !set;
  $("transport").hidden = !set;
  document.querySelector(".toolbar").hidden = !set;
  $("noteform").hidden = !set;
  $("live").disabled = !set || !set.live;
  if (set && !set.live) live = false;
  press($("live"), live);
  if (live && total() - nowM() > 2 * LIVE_LAG) setClock(true, total() - LIVE_LAG);
  renderSets();
  renderNotes();
  renderWatchParty();
}
function renderViews() {
  const names = play.keys.map(nameOf);
  if (!changed("views", [play.view, names])) return;
  const button = (i, text) => {
    const b = el("button", { className: play.view === i ? "on" : "", title: text, ariaPressed: play.view === i }, i <= MAX_KEYED ? el("kbd", { textContent: i }) : null, text);
    b.onclick = () => setView(play, i);
    return b;
  };
  $("views").replaceChildren(button(0, "All"), ...names.map((n, i) => button(i + 1, n)));
}

function renderSets() {
  if (!st) {
    $("setlist").replaceChildren(...[...Array(SKELETON_ROWS)].map(() => el("div", { className: "skel" })));
    return;
  }
  const now = nowUs();
  const rows = st.sets.map((s) => ({ id: s.id, room: s.room, live: s.live, sel: s.id === selected, notes: s.notes.length, dur: fmt(((s.end_us || now) - s.start_us) / 1e6), progress: s.progress.total && s.progress.have < s.progress.total ? s.progress.have / s.progress.total : null, ok: s.all_verified, left: !s.live && s.missing ? s.status : "" }));
  if (!changed("sets", rows)) return;
  if (!rows.length) { $("setlist").replaceChildren(el("div", { className: "none", textContent: "No sets yet" })); return; }
  $("setlist").replaceChildren(...rows.map((r) => {
    const top = r.live ? el("span", { className: "t" }, el("i", { className: "dot rec" }), "Live") : el("span", { className: "t" }, setTime(r.id), r.ok ? el("span", { className: "ok", textContent: "✓", title: "Everyone has it" }) : null);
    const sub = [r.live ? setTime(r.id) : "", r.dur, r.notes ? `${r.notes} note${r.notes === 1 ? "" : "s"}` : ""].filter(Boolean).join(" · ");
    const b = el("button", { className: "setrow" + (r.sel ? " sel" : ""), role: "option", ariaSelected: r.sel },
      top, el("small", { textContent: sub }),
      r.progress !== null ? el("span", { className: "bar" }, el("i", { style: `width:${(r.progress * 100).toFixed(1)}%` })) : null,
      r.left ? el("small", { className: "left-files", textContent: r.left, title: r.left }) : null);
    b.onclick = () => { selectSet(r.id); $("sets").classList.remove("open"); refresh(); };
    b.oncontextmenu = (e) => {
      e.preventDefault();
      openMenu([
        { label: "Export as one video...", disabled: r.live, run: () => exportSet(st.sets.find((s) => s.id === r.id)) },
        { label: "Open folder", run: () => invoke("open_set_folder", { room: r.room, set: r.id }) },
        { label: "Delete from this PC", danger: true, disabled: r.live, run: () => { if (confirm(`Delete the set ${setTime(r.id)} from this PC?`)) invoke("delete_set", { room: r.room, set: r.id }).catch(flash); } },
      ], e);
    };
    return b;
  }));
}
function selectSet(id) {
  if (id === selected) return;
  selected = id;
  clock.playing = false;
  clock.m = 0;
  live = false;
  lastCtl = null;
  delete sigs.notes;
  delete sigs.views;
  play.view = 0;
  prune(play, []);
}

function renderWatchParty() {
  const set = playSet();
  const viewers = 1 + Object.values(presence).filter((p) => set && p.set === set.id && Date.now() - p.at < PRESENCE_MS).length;
  press($("wp"), watchParty);
  $("wp").textContent = watchParty ? `Watch Party · ${viewers}` : "Watch Party";
  if (play.ring) play.ring.hidden = !watchParty;
}

function tick() {
  if (!st) return;
  if (tab === "record") for (const t of [...Object.values(rec.tiles), preview]) follow(t);
  else for (const t of [...Object.values(rec.tiles), preview]) if (t.video && !t.video.paused) t.video.pause();
  const list = players();
  const tot = total(list);
  let m = nowM();
  if (clock.playing && tot && m >= tot) { clock.playing = false; clock.m = tot; m = tot; }
  const b = base(list);
  for (const p of list) {
    const t = play.tiles[p.key];
    if (!t) continue;
    const v = t.video;
    const target = m + deltaOf(p, b);
    const inRange = target >= 0 && (!isFinite(v.duration) || target <= v.duration);
    if (!inRange || !clock.playing) {
      if (!v.paused) v.pause();
      if (v.readyState > 0 && Math.abs(v.currentTime - Math.max(0, target)) > SEEK_TOLERANCE) v.currentTime = Math.max(0, Math.min(target, v.duration || target));
    } else if (v.paused) {
      v.currentTime = target;
      v.play().catch(() => {});
    } else if (Math.abs(v.currentTime - target) > DRIFT_MAX) {
      v.currentTime = target;
    }
  }
  $("clocknow").textContent = fmt(m);
  $("clocktotal").textContent = ` / ${fmt(tot)}`;
  const frac = tot ? Math.min(1, m / tot) : 0;
  $("seek").querySelector(".fill").style.width = frac * 100 + "%";
  $("seek").querySelector(".thumb").style.left = frac * 100 + "%";
  $("seek").setAttribute("aria-valuemax", Math.round(tot));
  $("seek").setAttribute("aria-valuenow", Math.round(m));
  $("seek").setAttribute("aria-valuetext", fmt(m));
  $("play").classList.toggle("alt", clock.playing);
  $("play").setAttribute("aria-label", clock.playing ? "Pause" : "Play");
  if (st.active_set) $("timer").textContent = fmt((nowUs() - st.active_set.start_us) / 1e6);
}

function osd(text, undo) {
  $("osdtext").textContent = text;
  osdUndo = undo;
  $("osdundo").hidden = !undo;
  $("osd").hidden = false;
  clearTimeout(osdTimer);
  osdTimer = setTimeout(() => { $("osd").hidden = true; osdUndo = null; }, OSD_MS);
}
function bezel(playing) {
  const b = $("bezel");
  b.replaceChildren(svg("svg", { viewBox: "0 0 16 16" }, svg("path", { d: playing ? "M4 3l9 5-9 5z" : "M4 3h3v10H4zM9 3h3v10H9z" })));
  b.hidden = true;
  void b.offsetWidth;
  b.hidden = false;
}
const VERBS = { play: "played", pause: "paused", seek: "jumped to" };
function apply(cmd, m, who) {
  const prev = { playing: clock.playing, m: nowM() };
  live = false;
  if (cmd === "play") setClock(true, m);
  else if (cmd === "pause") setClock(false, m);
  else if (cmd === "seek") setClock(clock.playing, m);
  else return;
  if (cmd !== "seek") bezel(cmd === "play");
  osd(`${who} ${VERBS[cmd]}${cmd === "seek" ? " " + fmt(m) : ""}`, () => act(prev.playing ? "play" : "pause", prev.m));
}
const offsetOf = (id) => { const p = peerOf(id); return (p && p.offset_us) || 0; };
function isNewer(c) {
  if (!lastCtl) return true;
  const a = c.sent_us - offsetOf(c.origin), b = lastCtl.sent_us - offsetOf(lastCtl.origin);
  return a > b || (a === b && c.origin > lastCtl.origin);
}
function sendCtl(c) {
  invoke("control", { cmd: c.cmd, wallUs: c.wall_us, set: c.set, sentUs: c.sent_us, origin: c.origin }).catch(() => {});
}
function act(cmd, m) {
  const set = playSet();
  if (!set) return;
  apply(cmd, m, "You");
  lastCtl = { cmd, wall_us: wallOf(m), set: set.id, sent_us: nowUs(), origin: st.id };
  if (watchParty) sendCtl(lastCtl);
}
window.__TAURI__.event.listen("control", (e) => {
  const c = e.payload;
  if (!st) return;
  if (c.from) presence[c.from] = { set: c.set, at: Date.now() };
  const set = playSet();
  if (c.cmd === "here" || !watchParty || !set || c.set !== set.id || c.origin === st.id || !isNewer(c)) return;
  lastCtl = c;
  const age = c.cmd === "play" ? (nowUs() - (c.sent_us - offsetOf(c.origin))) / 1e6 : 0;
  apply(c.cmd, (c.wall_us - offsetOf(c.origin) - base()) / 1e6 + Math.max(0, age), nameOf(c.origin));
});
setInterval(() => {
  const set = playSet();
  if (!st || !watchParty || !set || tab !== "player") return;
  if (lastCtl && lastCtl.set === set.id) sendCtl(lastCtl);
  sendCtl({ cmd: "here", wall_us: 0, set: set.id, sent_us: nowUs(), origin: st.id });
}, CONTROL_REPEAT_MS);
const toggle = () => act(clock.playing ? "pause" : "play", nowM());
const jump = (delta) => act("seek", Math.min(total(), Math.max(0, nowM() + delta)));

function renderNotes() {
  const set = playSet();
  const b = base();
  noteItems = st.notes
    .filter((n) => set && set.notes.includes(n.key))
    .map((n) => ({ id: n.id, mine: n.mine, m: (n.local_us - b) / 1e6, who: nameOf(n.author), text: n.text, seekable: n.seekable }))
    .sort((x, y) => x.m - y.m);
  $("notestoggle").textContent = $("noteshead").textContent = `Notes ${noteItems.length}`;
  if ($("notelist").querySelector("input") || !changed("notes", [noteItems, Math.round(total())])) return;
  $("notelist").replaceChildren(...noteItems.map((n) => {
    const when = el("button", { className: "when", textContent: fmt(n.m), disabled: !n.seekable, ariaLabel: `Go to ${fmt(n.m)}` });
    when.onclick = () => act("seek", Math.max(0, n.m));
    const row = el("div", { className: "note" }, when, el("div", { className: "body" }, el("span", { textContent: n.text }), el("span", { className: "who", textContent: n.who })));
    if (n.mine) {
      const edit = el("button", { textContent: "Edit" });
      edit.onclick = () => editNote(n, row);
      const del = el("button", { textContent: "Delete" });
      del.onclick = () => {
        invoke("note_action", { kind: "delete", id: n.id, text: "" });
        osd("Note deleted", () => invoke("note_action", { kind: "restore", id: n.id, text: "" }));
      };
      row.append(el("span", { className: "acts" }, edit, del));
    }
    return row;
  }));
  const tot = total();
  $("lane").replaceChildren(...(tot ? noteItems.filter((n) => n.seekable) : []).map((n) => {
    const mark = el("button", { className: "mark", title: `${fmt(n.m)} ${n.text}`, ariaLabel: `${fmt(n.m)} ${n.text}`, tabIndex: -1 });
    mark.style.left = Math.max(0, Math.min(100, (n.m / tot) * 100)) + "%";
    mark.onclick = () => act("seek", Math.max(0, n.m));
    return mark;
  }));
}
function editNote(n, row) {
  const input = el("input", { value: n.text, ariaLabel: "Edit note" });
  const done = () => { delete sigs.notes; renderNotes(); };
  input.onkeydown = (e) => {
    e.stopPropagation();
    if (e.key === "Enter") { invoke("note_action", { kind: "edit", id: n.id, text: input.value }); input.remove(); done(); }
    if (e.key === "Escape") { e.preventDefault(); input.remove(); done(); }
  };
  input.onblur = () => { if (input.isConnected) { input.remove(); done(); } };
  row.replaceChildren(input);
  input.focus();
}
function stepNote(dir) {
  const m = nowM();
  const list = noteItems.filter((n) => n.seekable).map((n) => n.m);
  const next = dir > 0 ? list.find((x) => x > m + NOTE_STEP_GAP) : [...list].reverse().find((x) => x < m - NOTE_STEP_GAP);
  if (next !== undefined) act("seek", Math.max(0, next));
}

function layoutIcon(id, n) {
  const l = LAYOUTS.find((x) => x.id === id);
  const count = Math.min(l.max, Math.max(l.min, n));
  const r = rects(count, id, 22, 14, 1.5);
  return svg("svg", { viewBox: "0 0 22 14", class: "lic" }, ...r.tiles.map((b, i) => svg("rect", { x: b.x, y: b.y, width: b.w, height: b.h, rx: 1, class: i === 0 && id !== "grid" && id !== "row" ? "big" : "" })));
}
function renderLayoutButtons() {
  for (const [id, s] of [["layoutrec", rec], ["layoutplay", play]]) {
    const mode = layoutFits(s.mode, s.keys.length) ? s.mode : "grid";
    if (!changed(id, mode + s.keys.length)) continue;
    $(id).replaceChildren(layoutIcon(mode, s.keys.length));
    $(id).title = LAYOUTS.find((l) => l.id === mode).name;
  }
}
function pickLayout(s, anchor) {
  const n = Math.max(1, s.keys.length);
  openMenu(LAYOUTS.filter((l) => layoutFits(l.id, n)).map((l) => ({ label: l.name, icon: layoutIcon(l.id, n), on: s.mode === l.id, run: () => saveLayout(s, l.id) })), anchor);
}

function openMenu(items, at) {
  const m = $("menu");
  menuReturn = document.activeElement;
  m.replaceChildren(...items.map((it) => {
    const b = el("button", { role: "menuitem", disabled: !it.run || !!it.disabled, className: [it.on ? "on" : "", it.danger ? "danger" : ""].join(" ").trim() }, it.icon || null, it.label);
    b.onclick = () => { closeMenu(); it.run(); };
    return b;
  }));
  m.hidden = false;
  const r = at instanceof Event ? { left: at.clientX || at.target.getBoundingClientRect().left, bottom: at.clientY || at.target.getBoundingClientRect().bottom } : at.getBoundingClientRect();
  m.style.left = Math.max(0, Math.min(r.left, innerWidth - m.offsetWidth - 4)) + "px";
  m.style.top = Math.max(0, Math.min(r.bottom + 4, innerHeight - m.offsetHeight - 4)) + "px";
  const first = m.querySelector("button:not(:disabled)");
  if (first) first.focus();
}
function closeMenu() {
  if ($("menu").hidden) return;
  $("menu").hidden = true;
  if (menuReturn && menuReturn.isConnected) menuReturn.focus();
}
document.addEventListener("mousedown", (e) => {
  if (!$("menu").contains(e.target)) closeMenu();
  if (!$("extra").contains(e.target) && e.target.closest("#more") === null) $("extra").classList.remove("open");
});

function flash(text, info) {
  flashText = String(text);
  flashInfo = !!info;
  clearTimeout(flashTimer);
  flashTimer = setTimeout(() => { flashText = ""; renderNotice(); }, NOTICE_MS);
  renderNotice();
}
function renderNotice() {
  const problems = st ? [st.notice, st.ffmpeg, st.capture.status, st.disk.low ? `Low disk space: ${gb(st.disk.free)} free` : "", ...st.peers.filter((p) => p.message).map((p) => `${nameOf(p.id)}: ${p.message}`)] : [];
  const update = pending ? `Version ${pending.version} is available` : updateText;
  const progress = exporting ? `Exporting ${Math.floor(exporting.percent)}%${exporting.skipped ? ` without ${exporting.skipped} (not on this PC yet)` : ""}` : "";
  const list = [flashText, ...problems, progress, update].filter(Boolean);
  const text = list.join("  ·  ");
  $("noticetext").textContent = text;
  $("notice").hidden = !text || (text === dismissed && !exporting);
  $("notice").classList.toggle("info", list.every((t) => (t === flashText && flashInfo) || t === update || t === progress));
  $("updatebtn").hidden = !pending;
  $("exportcancel").hidden = !exporting;
}
$("notice").onclick = (e) => { if (e.target.tagName !== "BUTTON") { dismissed = $("noticetext").textContent; renderNotice(); } };
$("exportcancel").onclick = () => invoke("cancel_export");
function exportSet(set) {
  if (!set) { flash("Choose a set in the Player first"); return; }
  invoke("export_set", { room: set.room, set: set.id }).then((r) => {
    if (r) { exporting = { percent: 0, ...exporting, skipped: r.skipped.join(", ") }; renderNotice(); }
  }).catch(flash);
}
window.__TAURI__.event.listen("export", (e) => {
  const x = e.payload;
  if (x.done) { exporting = null; flash(x.error || "Export finished", !x.error); return; }
  exporting = { ...exporting, percent: x.percent };
  renderNotice();
});
async function checkUpdate(manual) {
  try {
    pending = await window.__TAURI__.updater.check();
  } catch {
    if (manual) flash("Update check failed");
    return;
  }
  if (!pending && manual) flash("Up to date", true);
  renderNotice();
}
$("updatebtn").onclick = async (e) => {
  e.stopPropagation();
  const update = pending;
  pending = null;
  updateText = `Installing ${update.version}; the app will restart`;
  renderNotice();
  await update.downloadAndInstall().catch(() => { updateText = "Update failed"; renderNotice(); });
};

function renderTop() {
  const online = 1 + st.peers.filter((p) => p.connected).length;
  const pendingRoom = st.room && st.room.pending;
  $("roomlabel").replaceChildren(...(!st.room ? ["No room"] : pendingRoom ? ["Connecting..."] : [el("b", { textContent: st.room.name }), ` · ${online}${online > st.peers.length ? "" : "/" + (st.peers.length + 1)}`]));
  $("rec").disabled = !st.room || pendingRoom;
  $("rec").classList.toggle("on", st.recording);
  $("rec").lastElementChild.textContent = st.recording ? "Stop" : "Record";
  $("timer").hidden = !st.active_set;
  $("starter").textContent = st.active_set ? st.active_set.by : "";
  $("sharing").hidden = !(st.capture.source_kind === "display" && (st.capture.sharing || st.recording));
  $("tabrecord").classList.toggle("on", tab === "record");
  $("tabplayer").classList.toggle("on", tab === "player");
  $("tabrecord").setAttribute("aria-selected", tab === "record");
  $("tabplayer").setAttribute("aria-selected", tab === "player");
  $("recordtab").hidden = tab !== "record";
  $("playertab").hidden = tab !== "player";
}

function renderCapture() {
  const c = st.capture;
  const value = c.source ? JSON.stringify(c.source) : "";
  if (document.activeElement !== $("source")) {
    if (![...$("source").options].some((o) => o.value === value)) $("source").append(el("option", { value, textContent: c.source.kind === "window" ? `${c.source.title} (window)` : `Display ${c.source.index}` }));
    $("source").value = value;
  }
  $("source").classList.toggle("lost-src", !!c.source_lost);
  $("lost").hidden = !c.source_lost;
  if (document.activeElement !== $("mic")) $("mic").value = [...$("mic").options].some((o) => o.value === c.mic) ? c.mic : "";
  for (const kind of ["game", "mic"]) {
    if (document.activeElement !== $(kind + "vol")) $(kind + "vol").value = c.mix[kind].volume;
    $(kind + "mute").classList.toggle("alt", c.mix[kind].muted);
    $(kind + "mute").setAttribute("aria-pressed", c.mix[kind].muted);
    $(kind + "meter").classList.toggle("muted", c.mix[kind].muted);
  }
}
async function fillSources() {
  const list = await invoke("list_sources");
  $("source").replaceChildren(el("option", { value: "", textContent: "No source" }), ...list.map((s) => el("option", { value: JSON.stringify(s.id), textContent: s.name })));
  if (st) renderCapture();
}
async function fillMics() {
  const list = await invoke("list_mics");
  $("mic").replaceChildren(el("option", { value: "", textContent: "Default" }), ...list.map((m) => el("option", { value: m.id, textContent: m.name })));
  if (st) renderCapture();
}
async function fillProcesses() {
  const names = await invoke("list_processes");
  const current = st ? st.audio_process : $("proc").value;
  $("proc").replaceChildren(el("option", { value: "", textContent: "Automatic" }), ...names.map((n) => el("option", { textContent: n })));
  $("proc").value = current;
}
const setMix = (kind, mute) => invoke("set_mix", { kind, volume: Number($(kind + "vol").value), muted: mute }).then(refresh);
for (const kind of ["game", "mic"]) {
  $(kind + "vol").oninput = () => setMix(kind, st.capture.mix[kind].muted);
  $(kind + "mute").onclick = () => setMix(kind, !st.capture.mix[kind].muted);
}
$("source").onfocus = fillSources;
$("source").onchange = () => invoke("set_source", { source: $("source").value ? JSON.parse($("source").value) : null }).catch(flash);
$("mic").onfocus = fillMics;
$("mic").onchange = () => invoke("set_mic", { id: $("mic").value });
$("usedisplay").onclick = () => invoke("use_display").then(fillSources);
window.__TAURI__.event.listen("levels", (e) => {
  const now = performance.now();
  for (const kind of ["game", "mic"]) {
    const v = Math.max(0, Math.min(1, e.payload[kind] || 0));
    const p = peaks[kind];
    if (v >= p.v || now - p.at > PEAK_HOLD_MS) { p.v = v; p.at = now; }
    $(kind + "meter").style.setProperty("--v", v * 100 + "%");
    $(kind + "meter").style.setProperty("--p", p.v * 100 + "%");
  }
});

async function refresh() {
  st = await invoke("state");
  if (!ready) { ready = true; invoke("ui_ready"); }
  if (!selected || !st.sets.some((s) => s.id === selected)) {
    const first = st.sets[0];
    if (first) selectSet(first.id);
    else selected = null;
  }
  renderTop();
  renderRooms();
  renderNotice();
  if (st.room) renderRecord();
  renderCapture();
  renderPlayer();
  renderLayoutButtons();
  $("invitebox").hidden = !st.invites;
  $("recovery").hidden = !st.recovery;
  $("recoverymsg").textContent = st.recovery || "";
  $("rinfo").textContent = st.previous_version ? `Version ${st.version}, previous run ${st.previous_version}` : `Version ${st.version}`;
}

let full = null;
function toggleFull() {
  if (full) full.classList.remove("full");
  full = full ? null : tab === "player" ? $("stage") : $("rectiles");
  if (full) full.classList.add("full");
  window.__TAURI__.window.getCurrentWindow().setFullscreen(!!full).catch(() => {});
  wake();
}
function wake() {
  $("stage").classList.remove("idle");
  clearTimeout(idleTimer);
  if (full) idleTimer = setTimeout(() => $("stage").classList.add("idle"), IDLE_MS);
}
document.addEventListener("mousemove", wake);

const showCode = (code) => { $("invitebox").hidden = false; $("code").value = code; };
const makeInvite = () => invoke("create_invite", { reuse: false }).then(showCode);
const createRoom = () => invoke("create_room", { name: "" }).then(() => { showTab("record"); $("roomsdlg").close(); return refresh().then(makeInvite); }).catch(flash);
function showTab(name) {
  if (full) toggleFull();
  tab = name;
  if (st) { renderTop(); place(rec); place(play); }
  refresh();
}
function openRooms() {
  if (!st || !st.room) { showTab("record"); $("joincode").focus(); return; }
  $("roomsdlg").append($("rooms"));
  $("roomsdlg").showModal();
  $("joincode").focus();
}
function openSettings() {
  if (st) {
    $("name").value = st.name;
    $("share").checked = st.share;
    $("limit").value = st.limit_mbps || "";
    $("keepdays").value = st.keep_days || "";
  }
  fillProcesses();
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
const changeRate = (dir) => setRate(SPEEDS[Math.max(0, Math.min(SPEEDS.length - 1, SPEEDS.indexOf(rate) + dir))]);
const toggleMute = () => { muted = !muted; applyAudio(); };
function setVolume(v) {
  volume = Math.max(0, Math.min(1, v));
  muted = false;
  applyAudio();
}
const current = () => (tab === "player" ? play : rec);
function cycle(s, delta) {
  const n = s.keys.length;
  if (n) setView(s, s.view === 0 ? (delta > 0 ? 1 : n) : ((s.view - 1 + delta + n) % n) + 1);
}

$("rec").onclick = () => (st && st.recording ? invoke("stop_recording") : invoke("start_recording")).then(refresh).catch(flash);
$("tabrecord").onclick = () => showTab("record");
$("tabplayer").onclick = () => showTab("player");
$("play").onclick = toggle;
$("back").onclick = () => jump(-STEP.normal);
$("fwd").onclick = () => jump(STEP.normal);
$("wp").onclick = () => { watchParty = !watchParty; renderWatchParty(); place(play); };
$("osdundo").onclick = () => { const u = osdUndo; $("osd").hidden = true; osdUndo = null; if (u) u(); };
$("live").onclick = () => { live = !live; if (live) setClock(true, Math.max(0, total() - LIVE_LAG)); renderPlayer(); };
$("speed").onchange = (e) => setRate(Number(e.target.value));
$("volume").oninput = () => setVolume(Number($("volume").value));
$("mute").onclick = toggleMute;
$("more").onclick = () => $("extra").classList.toggle("open");
$("full").onclick = toggleFull;
$("layoutrec").onclick = (e) => pickLayout(rec, e.currentTarget);
$("layoutplay").onclick = (e) => pickLayout(play, e.currentTarget);
$("setsrail").onclick = () => $("setsrail").setAttribute("aria-expanded", $("sets").classList.toggle("open"));
$("notestoggle").onclick = () => $("notestoggle").setAttribute("aria-expanded", $("notes").classList.toggle("open"));
function seekAt(x) {
  const r = $("seek").getBoundingClientRect();
  act("seek", Math.max(0, Math.min(1, (x - r.left) / r.width)) * total());
}
$("seek").onpointerdown = (e) => {
  seekAt(e.clientX);
  $("seek").setPointerCapture(e.pointerId);
  $("seek").onpointermove = (m) => seekAt(m.clientX);
  $("seek").onpointerup = () => { $("seek").onpointermove = null; };
};
$("noteform").onsubmit = (e) => {
  e.preventDefault();
  if (playSet() && $("note").value.trim()) invoke("add_note", { text: $("note").value, atUs: wallOf(nowM()) });
  $("note").value = "";
  $("note").blur();
};
$("newroom").onclick = createRoom;
$("invite").onclick = makeInvite;
$("cancelinvite").onclick = () => invoke("cancel_invites").then(() => { $("invitebox").hidden = true; });
$("copycode").onclick = () => navigator.clipboard.writeText($("code").value).then(() => {
  $("copycode").textContent = "Copied";
  setTimeout(() => { $("copycode").textContent = "Copy"; }, COPIED_MS);
});
$("join").onclick = () => invoke("join_room", { code: $("joincode").value }).then(() => { $("joincode").value = ""; $("roomsdlg").close(); }).catch(flash);
$("joincode").onkeydown = (e) => { if (e.key === "Enter") $("join").click(); };
$("proc").onchange = () => invoke("set_audio_process", { name: $("proc").value });
$("keepdays").onchange = () => invoke("set_keep_days", { days: $("keepdays").value ? Number($("keepdays").value) : null });
$("name").onchange = () => invoke("set_name", { name: $("name").value });
$("share").onchange = () => invoke("set_share", { on: $("share").checked });
$("limit").onchange = () => invoke("set_limit", { mbps: $("limit").value ? Number($("limit").value) : null });
$("rfolder").onclick = () => invoke("open_data_folder");
$("rreport").onclick = () => invoke("save_report", { includeAddresses: false, audioProcess: "" }).then((p) => { $("rinfo").textContent = p; });
$("rreset").onclick = () => invoke("reset", { everything: false, confirm: "" });
$("rcontinue").onclick = () => invoke("dismiss_recovery").then(refresh);

const menuActions = {
  "settings": openSettings,
  "create-room": createRoom,
  "join-room": openRooms,
  "copy-invite": () => invoke("create_invite", { reuse: true }).then((code) => { showCode(code); return navigator.clipboard.writeText(code); }).then(() => flash("Invite code copied", true)),
  "cancel-invite": () => invoke("cancel_invites").then(() => { $("invitebox").hidden = true; flash("Invite cancelled", true); }),
  "leave-room": () => st && st.room && invoke("leave_room", { id: st.room.id }).then(refresh),
  "export-set": () => exportSet(playSet()),
  "fullscreen": toggleFull,
  "all-players": () => setView(current(), 0),
  "next-player": () => cycle(current(), 1),
  "prev-player": () => cycle(current(), -1),
  "shortcuts": () => toggleDialog($("shortcuts")),
  "update": () => checkUpdate(true),
};
window.__TAURI__.event.listen("menu", (e) => { if (menuActions[e.payload]) menuActions[e.payload](); });
window.__TAURI__.event.listen("joined", (e) => flash(`${e.payload} joined`, true));

document.addEventListener("keydown", (e) => {
  if (!$("menu").hidden) {
    const items = [...$("menu").querySelectorAll("button:not(:disabled)")];
    const i = items.indexOf(document.activeElement);
    if (e.key === "Escape" || e.key === "Tab") { e.preventDefault(); closeMenu(); }
    else if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); items[(i + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length].focus(); }
    return;
  }
  if (document.querySelector("dialog[open]")) {
    if (e.key === "?" && $("shortcuts").open) { $("shortcuts").close(); e.preventDefault(); }
    return;
  }
  if (e.key === "Escape") {
    if (typing()) document.activeElement.blur();
    else if (full) toggleFull();
    else { $("sets").classList.remove("open"); $("notes").classList.remove("open"); $("extra").classList.remove("open"); }
    return;
  }
  if (typing() || e.ctrlKey || e.metaKey || e.altKey || !st) return;
  const onButton = document.activeElement && document.activeElement.tagName === "BUTTON";
  const key = e.key.normalize("NFKC");
  const k = key.length === 1 ? key.toLowerCase() : key;
  const s = current();
  const player = tab === "player";
  const shift = (a, b) => (e.shiftKey ? a : b);
  if (key === "?") toggleDialog($("shortcuts"));
  else if (/^[0-9]$/.test(key) && Number(key) <= Math.min(MAX_KEYED, s.keys.length)) setView(s, Number(key));
  else if (key === "Tab" && document.activeElement === document.body) setView(s, s.view ? 0 : s.last);
  else if (k === "f") toggleFull();
  else if (!player) return;
  else if ((key === " " && !onButton) || k === "k") { if (!e.repeat) toggle(); }
  else if (key === "ArrowLeft") jump(-shift(STEP.small, STEP.normal));
  else if (key === "ArrowRight") jump(shift(STEP.small, STEP.normal));
  else if (k === "j") jump(-shift(STEP.huge, STEP.big));
  else if (k === "l") jump(shift(STEP.huge, STEP.big));
  else if (key === "<" || key === ",") changeRate(-1);
  else if (key === ">" || key === ".") changeRate(1);
  else if (k === "m") toggleMute();
  else if (key === "ArrowUp") setVolume(volume + VOLUME_STEP);
  else if (key === "ArrowDown") setVolume(volume - VOLUME_STEP);
  else if (k === "c") { $("notes").classList.add("open"); $("note").focus(); }
  else if (k === "q") stepNote(-1);
  else if (k === "e") stepNote(1);
  else if (k === "u") $("osdundo").click();
  else return;
  e.preventDefault();
});

async function reportCodecs() {
  const types = { av1: 'video/mp4; codecs="av01.0.08M.08"', hevc: 'video/mp4; codecs="hvc1.1.6.L93.B0"', h264: 'video/mp4; codecs="avc1.640028"' };
  const list = [];
  for (const [name, contentType] of Object.entries(types)) {
    try {
      const info = await navigator.mediaCapabilities.decodingInfo({ type: "file", video: { contentType, ...CODEC_PROBE } });
      if (info.supported) list.push(name);
    } catch {}
  }
  invoke("set_codecs", { list: list.length ? list : ["h264"] });
}
renderSets();
reportCodecs();
setInterval(() => { if (!document.hidden) tick(); }, TICK_MS);
setInterval(() => { if (!document.hidden) refresh().catch(() => {}); }, REFRESH_MS);
refresh();
fillSources();
fillMics();
setTimeout(() => checkUpdate(false), UPDATE_CHECK_DELAY_MS);

const STEP = { small: 1, normal: 5, big: 10, huge: 60 };
const VOLUME_STEP = 0.05;
const SPEEDS = [0.5, 0.75, 1, 1.25, 1.5, 2, 3];
const MAX_KEYED = 6;
const TICK_MS = 150;
const REFRESH_MS = 1000;
const IDLE_MS = 2000;
const OSD_MS = 2000;
const NOTICE_MS = 5000;
const UPDATE_CHECK_DELAY_MS = 4000;
const DOUBLE_CLICK_MS = 250;
const JUMP_MIN = 1;
const METER_MIN_DB = -60;
const METER_TICKS = [-60, -40, -20, -10, -5, 0];
const METER_YELLOW_DB = -20;
const METER_RED_DB = -9;
const PEAK_HOLD_MS = 1500;
const CLIP_HOLD_MS = 3000;
const OFFLINE_FAILURES = 2;
const SKELETON_ROWS = 3;
const NOTE_STEP_GAP = 0.5;
const SPEED_WINDOW_MS = 10000;
const AVATARS_SHOWN = 6;
const JOBS_MS = 5000;
const TAG_HINTS = 5;
const CODEC_PROBE = { width: 1280, height: 720, bitrate: 6000000, framerate: 30 };
const GAP = { record: 8, player: 4 };

const stage = (el, kind) => ({ el, kind, tiles: {}, keys: [], mode: "grid", slots: 0, view: 0, last: 1, free: [], ring: null });
const rec = stage($("rectiles"), "record");
const play = stage($("playtiles"), "player");
const clock = { playing: false, m: 0, at: 0 };
const preview = { box: $("preview"), slot: null };
const meters = { app: { v: 0, peak: 0, at: 0, clip: 0 }, mic: { v: 0, peak: 0, at: 0, clip: 0 } };
let st = null;
let idx = null;
let ready = false;
let lang = "";
let tab = "room";
const TABS = ["room", "record", "player"];
const HOME_SETS = 5;
const HOME_NOTES = 6;
let selected = null;
let rate = 1;
let volume = 1;
let muted = false;
let live = false;
let watchParty = true;
let partyLast = null;
let drag = null;
let partyLayout = null;
let items = [];
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
let side = "events";
let subtitles = stored("subtitles", true);
let receive = stored("receive", {});
let clickTimer = null;
let quick = null;
let speedSamples = [];
let partyRoom = null;
let askName = false;
let jobInfo = null;
let pluginsGen = 0;
let refreshing = null;
let refreshAgain = false;
const pluginNames = {};

const peerOf = (id) => st.peers.find((p) => p.id === id);
const nameOf = (id) => (id === st.id ? t("You") : st.names[id] || String(id).slice(0, 10));
const people = () => [st.id, ...st.peers.map((p) => p.id)];
const run = (cmd, args, then = refresh) => invoke(cmd, args).then(then).catch(flash);

function ordered(present, saved) {
  const keep = (saved || []).filter((k) => present.includes(k));
  return [...keep, ...present.filter((k) => !keep.includes(k))];
}

function avatar(id, small) {
  const p = id === st.id ? null : peerOf(id);
  const on = id === st.id || (p && p.connected);
  const a = el("span", { className: "av" + (on ? "" : " off") + (small ? " small" : ""), textContent: ((id === st.id ? st.name : st.names[id]) || "?").trim().charAt(0).toUpperCase() || "?", title: nameOf(id) });
  a.style.background = colorOf(colorIndex(id));
  if (recOf(id)) a.append(el("i"));
  return a;
}

function tile(s, key) {
  if (s.tiles[key]) return s.tiles[key];
  const tl = { key, media: slot(el("video", { muted: true, preload: "auto" })), mic: null, name: el("span"), dot: el("i", { className: "dot" }), badge: el("span", { className: "badge" }), who: el("div", { className: "who" }), what: el("div", { className: "what" }), sub: el("div", { className: "sub", hidden: true }) };
  tl.lab = el("div", { className: "lab", draggable: true }, tl.dot, tl.name);
  tl.box = el("div", { className: "tile", tabIndex: 0 }, tl.media.video, el("div", { className: "waitbox" }, tl.who, tl.what), tl.sub, tl.lab, tl.badge);
  const focus = () => { const i = s.keys.indexOf(key) + 1; setView(s, s.view === i ? 0 : i); };
  tl.box.onkeydown = (e) => { if (e.key === "Enter") focus(); };
  if (s === play) {
    tl.box.onpointerdown = (e) => inkDown(tl, e);
    tl.box.onpointermove = inkMove;
    tl.box.onpointerup = (e) => { if (inkUp(e)) tl.drew = true; };
    tl.box.onpointercancel = tl.box.onlostpointercapture = inkUp;
    tl.box.onclick = (e) => {
      if (e.target.closest(".lab")) return;
      if (tl.drew) { tl.drew = false; return; }
      clearTimeout(clickTimer);
      clickTimer = setTimeout(toggle, DOUBLE_CLICK_MS);
    };
    tl.box.ondblclick = (e) => { if (!e.target.closest(".lab")) { clearTimeout(clickTimer); focus(); } };
    tl.lab.onclick = (e) => { e.stopPropagation(); openReceive(key, tl.lab); };
    tl.box.oncontextmenu = (e) => { e.preventDefault(); stageMenu(e); };
  } else {
    tl.box.onclick = focus;
    tl.box.ondblclick = toggleFull;
  }
  tl.lab.ondragstart = (e) => e.dataTransfer.setData("text/plain", key);
  tl.box.ondragover = (e) => { e.preventDefault(); tl.box.classList.add("over"); };
  tl.box.ondragleave = () => tl.box.classList.remove("over");
  tl.box.ondrop = (e) => { e.preventDefault(); tl.box.classList.remove("over"); const from = e.dataTransfer.getData("text/plain"); if (from && from !== key && s.tiles[from]) swap(s, from, key); };
  s.el.append(tl.box);
  return (s.tiles[key] = tl);
}
function prune(s, keys) {
  for (const [key, tl] of Object.entries(s.tiles)) if (!keys.includes(key)) { drop(tl.media); drop(tl.mic); tl.box.remove(); delete s.tiles[key]; }
}
function effectiveMode(s) {
  const n = Math.max(s.keys.length, s.slots || 0);
  return layoutFits(s.mode, n) ? s.mode : "grid";
}
function place(s) {
  const n = s.keys.length;
  if (s.view > n) s.view = 0;
  const focus = s.view ? s.keys[s.view - 1] : null;
  const order = focus ? [focus, ...s.keys.filter((k) => k !== focus)] : s.keys;
  const slots = focus ? n : Math.max(n, s.slots || 0);
  const mode = focus ? (n > 1 ? (s.mode === "side" ? "side" : "bottom") : "grid") : effectiveMode(s);
  const r = rects(n, mode, s.el.clientWidth, s.el.clientHeight, GAP[s.kind], slots);
  const at = (e, b) => Object.assign(e.style, { left: b.x + "px", top: b.y + "px", width: b.w + "px", height: b.h + "px" });
  order.forEach((key, i) => {
    const tl = s.tiles[key];
    if (!tl) return;
    at(tl.box, r.tiles[i]);
    tl.box.classList.toggle("active", key === focus);
    tl.badge.textContent = s.keys.indexOf(key) + 1;
  });
  while (s.free.length > r.free.length) s.free.pop().remove();
  while (s.free.length < r.free.length) s.free.push(s.el.appendChild(el("div", { className: s.kind === "record" ? "tile free" : "tile blank" })));
  r.free.forEach((b, i) => { at(s.free[i], b); s.free[i].textContent = s.kind === "record" ? t("Free") : ""; });
  if (s.ring) {
    const all = [...r.tiles, ...r.free];
    s.ring.hidden = !watchParty || !all.length || !playSet();
    if (all.length) {
      const x = Math.min(...all.map((b) => b.x)), y = Math.min(...all.map((b) => b.y));
      at(s.ring, { x, y, w: Math.max(...all.map((b) => b.x + b.w)) - x, h: Math.max(...all.map((b) => b.y + b.h)) - y });
    }
  }
  if (s === play && st) renderInk();
}
play.ring = play.el.appendChild(el("div", { className: "ring" }));
for (const s of [rec, play]) new ResizeObserver(() => place(s)).observe(s.el);

function setView(s, view) {
  s.view = Math.max(0, Math.min(view, s.keys.length));
  if (s.view) s.last = s.view;
  place(s);
  if (s === play) { applyAudio(); renderViews(); logLayout(); share(); }
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
    invoke("set_layout", { layout: st.layout }).catch(() => {});
  } else {
    const set = playSet();
    if (set && watchParty) { partyLayout = { set: set.id, mode, slots, order: s.keys }; share(); }
    else if (set) store("layout/" + set.id, { mode, slots, order: s.keys });
  }
  place(s);
  renderLayoutButtons();
  if (s === play) logLayout();
}
const fromSaved = (l) => ({ mode: (l && l.mode) || "grid", slots: (l && l.slots) || 0, order: (l && l.order) || [] });
function playLayout(set) {
  return fromSaved(stored("layout/" + set.id, null) || set.layout);
}
function logLayout() {
  const set = playSet();
  const size = (key, sideName) => Math.round(play.tiles[key] ? play.tiles[key].box[sideName] : 0);
  if (set) invoke("log_layout", { layout: { set: set.id, mode: play.view ? "focus" : play.mode, slots: play.slots, tiles: play.keys.map((key, i) => ({ id: key, index: i, w: size(key, "clientWidth"), h: size(key, "clientHeight") })) } }).catch(() => {});
}

function recOf(id) {
  const set = st.sets.find((s) => s.live);
  return !!(set && set.players.some((p) => p.owner === id && p.recording));
}
function recordPlayers() {
  const files = { [st.id]: st.capture.live };
  for (const p of st.peers) if (p.live) files[p.id] = p.live;
  const keys = [st.id, ...st.peers.filter((p) => p.connected || files[p.id]).map((p) => p.id)];
  return keys.map((key) => ({ key, file: files[key] && files[key].size > 0 ? files[key] : null, recording: recOf(key) && (key !== st.id || st.capture.working) }));
}
function dotOf(key, recording) {
  if (recording) return "dot rec";
  if (key === st.id) return "dot" + (st.capture.sharing ? " live" : "");
  const p = peerOf(key);
  if (!p || !p.connected) return "dot";
  return "dot" + (p.path === "relay" ? " relay" : p.sharing ? " live" : "");
}
function renderRecord() {
  if (!st.room || st.room.pending) return;
  const list = recordPlayers();
  const lay = fromSaved(st.layout);
  rec.keys = ordered(list.map((p) => p.key), lay.order);
  rec.mode = lay.mode;
  rec.slots = lay.slots;
  prune(rec, rec.keys);
  for (const p of list) {
    const tl = tile(rec, p.key);
    tl.name.textContent = nameOf(p.key);
    tl.who.textContent = nameOf(p.key);
    tl.what.textContent = p.key === st.id ? (st.capture.source ? (st.share ? t("Starting") : t("Sharing off")) : t("No source")) : t("Connected");
    tl.dot.className = dotOf(p.key, p.recording);
    tl.box.classList.toggle("wait", !p.file);
    if (p.file) feed(tl.media, p.file, true);
  }
  place(rec);
  const own = list[0].file;
  if (own) {
    if (!preview.slot) preview.slot = slot(preview.box.appendChild(el("video", { muted: true, preload: "auto" })));
    feed(preview.slot, own, true);
  } else if (preview.slot) { drop(preview.slot); preview.slot = null; }
}

function memberState(id) {
  if (id === st.id) return { state: recOf(id) ? t("Recording") : t("Online"), path: "" };
  const p = peerOf(id);
  if (!p || !p.connected) return { state: p && p.failures > OFFLINE_FAILURES ? t("Offline") : t("Connecting..."), path: "" };
  const path = `${p.path === "relay" ? t("Relay") : t("Direct")} ${p.rtt_ms} ms`;
  return { state: recOf(id) ? t("Recording") : t("Online"), path, relay: p.path === "relay", message: p.message };
}
function roomMenu(anchor) {
  const others = st.rooms.filter((r) => !r.active);
  openMenu([
    ...(others.length ? [{ heading: t("Switch room") }, ...others.map((r) => ({ label: r.name, right: String(r.members), run: () => run("switch_room", { id: r.id }) })), { separator: true }] : []),
    { label: t("Create room"), run: createRoom },
    { label: t("Join room..."), run: openJoin },
  ], anchor);
}
function homeMenu(anchor) {
  openMenu([
    ...(st.invites ? [{ label: t("Cancel invite"), run: cancelInvite }] : []),
    { label: t("Rename room..."), run: openRename },
    { label: t("Check connections..."), run: openCheck },
    { label: t("Plugins..."), run: openPlugins },
    { separator: true },
    { label: t("Leave room"), danger: true, run: leaveRoom },
  ], anchor);
}
function memberMenu(id, anchor) {
  openMenu([{ heading: nameOf(id) }, { label: t("Remove from room"), danger: true, run: () => { if (confirm(t("Remove {name} from the room for everyone?", { name: nameOf(id) }))) run("remove_peer", { id }); } }], anchor);
}

function renderTop() {
  const inRoom = !!st.room && !st.room.pending;
  $("roombtn").hidden = !inRoom;
  if (inRoom) {
    $("roomname").textContent = st.room.name;
    if (changed("avatars", [people(), st.peers.map((p) => p.connected), st.sets.filter((s) => s.live).map((s) => s.players.map((p) => p.recording)), st.names, st.colors])) {
      const all = people();
      $("avatars").replaceChildren(...all.slice(0, AVATARS_SHOWN).map((id) => avatar(id)), ...(all.length > AVATARS_SHOWN ? [el("span", { className: "av more", textContent: `+${all.length - AVATARS_SHOWN}` })] : []));
    }
  }
  $("rec").hidden = !inRoom;
  $("rec").classList.toggle("on", st.recording);
  $("rec").lastElementChild.textContent = st.recording ? t("Stop") : t("Record");
  $("timer").hidden = !st.active_set;
  $("starter").textContent = st.active_set && st.active_set.by ? t("{name} started", { name: st.active_set.by }) : "";
  $("sharing").hidden = !(st.capture.source_kind === "display" && (st.capture.sharing || st.recording));
  for (const [name, b] of [["room", $("tabroom")], ["record", $("tabrecord")], ["player", $("tabplayer")]]) {
    b.classList.toggle("on", tab === name);
    b.setAttribute("aria-selected", tab === name);
  }
  $("start").hidden = inRoom;
  $("roomtab").hidden = !inRoom || tab !== "room";
  $("recordtab").hidden = !inRoom || tab !== "record";
  $("playertab").hidden = !inRoom || tab !== "player";
  renderSync();
}

function receiving() {
  let have = 0, total = 0;
  for (const s of st.sets) if (!s.live && !s.progress.stuck && s.progress.total) { have += s.progress.have; total += s.progress.total; }
  const now = Date.now();
  speedSamples = [...speedSamples.filter((x) => now - x.at < SPEED_WINDOW_MS && x.have <= have), { at: now, have }];
  if (!total || have >= total) return null;
  const first = speedSamples[0];
  const speed = now > first.at ? ((have - first.have) * 1000) / (now - first.at) : 0;
  return { percent: Math.floor((have / total) * 100), speed, left: speed > 0 ? (total - have) / speed : null };
}
function renderSync() {
  const lines = [];
  const p = st.party;
  const shownLive = p && p.on && st.sets.some((s) => s.live && s.id === p.shared.set);
  if (watchParty && p && p.on) for (const s of p.status || []) {
    if ((s.who === st.id || shownLive) && s.kind !== "catching") continue;
    const name = nameOf(s.who);
    if (s.kind === "catching") lines.push(t("Catching up with {name}", { name }));
    else if (s.kind === "receiving") lines.push(t("{name} receiving {n}%", { name, n: s.percent }));
    else lines.push(t("{name} incomplete at {n}%", { name, n: s.percent }));
  }
  const job = jobInfo && jobInfo.running;
  if (job) lines.push(`${pluginNames[job.plugin] || job.plugin} ${Math.floor((job.progress || 0) * 100)}%`);
  const r = receiving();
  if (r) lines.push([t("Receiving {n}%", { n: r.percent }), r.speed ? `${bytes(r.speed)}/s` : "", r.left !== null ? t("{time} left", { time: fmt(r.left) }) : ""].filter(Boolean).join(" · "));
  $("sync").hidden = !lines.length;
  $("synctext").textContent = lines.join("  ·  ");
}

function renderStart() {
  const pendingRoom = st.room && st.room.pending;
  if (!st.name) askName = true;
  $("namefield").hidden = !askName;
  if (document.activeElement !== $("startname")) $("startname").value = st.name;
  const named = !!st.name || !!$("startname").value.trim();
  $("startpending").hidden = !pendingRoom;
  $("startform").hidden = !!pendingRoom;
  $("pendingtext").textContent = pendingRoom ? (st.room.waiting ? t("Waiting for approval...") : t("Connecting...")) : "";
  $("newroom").disabled = !named;
  $("join").disabled = !named || !$("joincode").value.trim();
  const others = st.rooms.filter((r) => !r.id.startsWith("pending-"));
  $("recenthead").hidden = !others.length;
  if (!changed("rooms", [others, lang])) return;
  $("roomlist").replaceChildren(...others.map((r) => {
    const b = el("button", { className: "roomrow" }, el("span", { textContent: r.name }), el("span", { className: "muted", textContent: t("{n} members", { n: r.members }) }));
    b.onclick = () => run("switch_room", { id: r.id });
    b.oncontextmenu = (e) => { e.preventDefault(); openMenu([{ label: t("Leave room"), danger: true, run: () => run("leave_room", { id: r.id }) }], e); };
    return b;
  }));
}

function playSet() { return (st && st.sets.find((s) => s.id === selected)) || null; }
function players() {
  const set = playSet();
  if (!set) return [];
  return set.players.map((p) => ({ key: p.owner, start: p.start_us, off: p.offset_us || 0, file: p, recording: p.recording }));
}
function base(list = players()) {
  return list.length ? Math.min(...list.map((p) => p.start - p.off)) : nowUs();
}
function total(list = players()) {
  const b = base(list);
  const ends = [];
  for (const p of list) {
    if (p.file.end_us) ends.push((p.file.end_us - p.off - b) / 1e6);
    const tl = play.tiles[p.key];
    if (tl && tl.media.part && isFinite(tl.media.video.duration)) ends.push(tl.media.video.duration - shiftOf(p, tl.media.part, b));
  }
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
const recv = (key) => ({ app: { volume: 1, muted: false }, voice: { volume: 1, muted: false }, ...(receive[key] || {}) });
function applyAudio() {
  const n = play.keys.length;
  play.keys.forEach((key, i) => {
    const tl = play.tiles[key];
    if (!tl) return;
    const heard = play.view === i + 1 || (play.view === 0 && n === 1);
    const mine = recv(key);
    for (const [s, on, mix] of [[tl.media, heard, mine.app], [tl.mic, true, mine.voice]]) if (s) for (const v of [s.video, s.pending && s.pending.video]) if (v) {
      v.muted = muted || !on || mix.muted;
      v.volume = volume * mix.volume;
      v.playbackRate = rate;
    }
  });
  $("mute").classList.toggle("alt", muted || volume === 0);
  if (document.activeElement !== $("volume")) $("volume").value = volume;
  $("speed").value = rate;
}
function openReceive(key, anchor) {
  const row = (kind, label) => {
    const mix = recv(key)[kind];
    const range = el("input", { type: "range", min: 0, max: 1, step: VOLUME_STEP, value: mix.volume, ariaLabel: label });
    const mute = el("button", { className: "icon" + (mix.muted ? " alt" : ""), ariaLabel: t("Mute") }, speakerIcon());
    const save = () => { receive[key] = { ...recv(key), [kind]: { volume: Number(range.value), muted: mute.classList.contains("alt") } }; store("receive", receive); applyAudio(); };
    range.oninput = () => { mute.classList.remove("alt"); save(); };
    mute.onclick = () => { mute.classList.toggle("alt"); save(); };
    return el("div", { className: "poprow" }, el("span", { textContent: label }), range, mute);
  };
  const pop = $("pop");
  pop.replaceChildren(el("div", { className: "pophead", textContent: nameOf(key) }), row("app", t("App sound")), row("voice", t("Voice")));
  pop.hidden = false;
  const r = anchor.getBoundingClientRect();
  pop.style.left = Math.max(4, Math.min(r.left, innerWidth - pop.offsetWidth - 4)) + "px";
  pop.style.top = Math.max(4, r.top - pop.offsetHeight - 6) + "px";
}
function speakerIcon() {
  return svg("svg", { viewBox: "0 0 16 16" }, svg("path", { d: "M2 6h3l4-3v10l-4-3H2z" }), svg("path", { class: "on", d: "M11 5.5a3.5 3.5 0 0 1 0 5" }), svg("path", { class: "off", d: "M11 6l4 4M15 6l-4 4" }));
}

function renderPlayer() {
  const set = playSet();
  const list = players();
  const shared = watchParty && set && partyLayout && partyLayout.set === set.id;
  const lay = set ? (shared ? { mode: partyLayout.mode || "grid", slots: partyLayout.slots || 0, order: partyLayout.order || [] } : playLayout(set)) : fromSaved(null);
  play.keys = ordered(list.map((p) => p.key), lay.order);
  play.mode = lay.mode;
  play.slots = lay.slots;
  if (shared && partyLayout.focus !== undefined) { play.view = partyLayout.focus ? play.keys.indexOf(partyLayout.focus) + 1 : 0; delete partyLayout.focus; }
  prune(play, play.keys);
  for (const p of list) {
    const tl = tile(play, p.key);
    tl.name.textContent = nameOf(p.key);
    tl.dot.className = p.recording ? "dot rec" : "dot";
    tl.dot.hidden = !p.recording;
    tl.who.textContent = nameOf(p.key);
    feedParts(tl, p, nowM(), base(list), clock.playing);
  }
  place(play);
  applyAudio();
  renderViews();
  $("empty").hidden = !!set;
  $("playtiles").hidden = !set;
  $("transport").hidden = !set;
  $("toolbar").hidden = !set;
  $("live").disabled = !set || !set.live;
  if (set && !set.live) live = false;
  press($("live"), live);
  press($("cc"), subtitles);
  if (live && total() - nowM() > 2 * LIVE_LAG) setClock(true, total() - LIVE_LAG);
}
function renderViews() {
  const names = play.keys.map(nameOf);
  if (!changed("views", [play.view, names])) return;
  const many = names.length > MAX_KEYED;
  const button = (i, text) => {
    const b = el("button", { className: play.view === i ? "on" : "", title: text, ariaPressed: play.view === i }, i <= MAX_KEYED && !(many && i) ? el("kbd", { textContent: i }) : null, el("span", { textContent: many && i ? String(i) : text }));
    b.onclick = () => setView(play, i);
    return b;
  };
  $("views").replaceChildren(button(0, t("All")), ...names.map((n, i) => button(i + 1, n)));
}
function renderTools() {
  for (const b of $("tools").querySelectorAll("[data-tool]")) press(b, ink.tool === b.dataset.tool);
}

function setMenu(r, e) {
  const set = st.sets.find((s) => s.id === r.id);
  openMenu([
    { label: t("Export as one video..."), disabled: set.live, run: () => exportSet(set) },
    { label: t("Open folder"), run: () => run("open_set_folder", { room: set.room, set: set.id }, null) },
    { label: t("Delete from this PC"), danger: true, disabled: set.live, run: () => { if (confirm(t("Delete the set {set} from this PC?", { set: setTime(set.id) }))) run("delete_set", { room: set.room, set: set.id }); } },
  ], e);
}
function lackingText(s) {
  if (s.live || !s.missing) return "";
  return s.lacking.map((l) => t("{name}: {n} files left", { name: nameOf(l.id), n: l.n })).join(" · ");
}
function renderSets() {
  if (!st) {
    $("setlist").replaceChildren(...[...Array(SKELETON_ROWS)].map(() => el("div", { className: "skel" })));
    return;
  }
  const rows = st.sets.map((s) => ({ id: s.id, live: s.live, sel: s.id === selected, sub: [s.live ? setTime(s.id) : "", setSummary(s)].filter(Boolean).join(" · "), progress: s.progress.total && s.progress.have < s.progress.total ? s.progress.have / s.progress.total : null, ok: s.all_verified, left: lackingText(s) }));
  if (!changed("sets", [rows, lang])) return;
  if (!rows.length) { $("setlist").replaceChildren(el("div", { className: "none", textContent: t("No sets yet") })); return; }
  $("setlist").replaceChildren(...rows.map((r) => {
    const top = r.live ? el("span", { className: "t" }, el("i", { className: "dot rec" }), t("Live")) : el("span", { className: "t" }, setTime(r.id), r.ok ? el("span", { className: "ok", textContent: "✓", title: t("Everyone has it") }) : null);
    const b = el("button", { className: "setrow" + (r.sel ? " sel" : ""), role: "option", ariaSelected: r.sel },
      top, el("small", { textContent: r.sub }),
      r.progress !== null ? el("span", { className: "bar" }, el("i", { style: `width:${(r.progress * 100).toFixed(1)}%` })) : null,
      r.left ? el("small", { className: "left-files", textContent: r.left, title: r.left }) : null);
    b.onclick = () => { openSet(r.id); $("sets").classList.remove("open"); };
    b.oncontextmenu = (e) => { e.preventDefault(); setMenu(r, e); };
    return b;
  }));
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
  invoke("focus_set", { set: id }).catch(() => {});
}
function stepSet(dir) {
  const i = st.sets.findIndex((s) => s.id === selected);
  const next = st.sets[i - dir];
  if (i >= 0 && next) { openSet(next.id); osd(setTime(next.id)); }
}

function renderWatchParty() {
  const p = st && st.party;
  press($("wp"), watchParty);
  $("wp").textContent = watchParty && p && p.on ? `${t("Watch Party")} · ${p.viewers}` : t("Watch Party");
}

function tick() {
  if (!st) return;
  const feeds = [...Object.values(rec.tiles).map((tl) => tl.media), preview.slot].filter(Boolean);
  if (tab === "record") {
    for (const s of feeds) follow(s);
    for (const kind of ["app", "mic"]) if (!meters[kind].v) drawMeter(kind, 0);
  } else for (const s of feeds) if (!s.video.paused) s.video.pause();
  const list = players();
  const tot = total(list);
  let m = nowM();
  if (clock.playing && tot && m >= tot) { clock.playing = false; clock.m = tot; m = tot; }
  const b = base(list);
  for (const p of list) {
    const tl = play.tiles[p.key];
    if (!tl) continue;
    const wall = b + m * 1e6;
    if (pathOf(tl.media.part) !== pathOf(partFor(p, "v", wall)) || pathOf(tl.mic && tl.mic.part) !== pathOf(partFor(p, "m", wall))) feedParts(tl, p, m, b, clock.playing);
    const v = tl.media.video;
    const target = tl.media.part ? m + shiftOf(p, tl.media.part, b) : 0;
    const have = tl.media.part && v.readyState > 0 && isFinite(v.duration) ? v.duration : 0;
    const short = !tl.media.part || (!!p.file.total && p.file.have < p.file.total && target > have);
    tl.box.classList.toggle("wait", short);
    if (short && p.file.total) tl.what.textContent = t(p.file.stuck ? "Incomplete {n}%" : "Receiving {n}%", { n: Math.floor((p.file.have / p.file.total) * 100) });
    for (const s of [tl.media, tl.mic]) if (s && s.part && !switching(s)) sync(s.video, m + shiftOf(p, s.part, b), clock.playing);
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
  $("play").setAttribute("aria-label", clock.playing ? t("Pause") : t("Play"));
  if (st.active_set) $("timer").textContent = fmt((nowUs() - st.active_set.start_us) / 1e6);
  renderMoment(m);
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
function apply(cmd, m, who, prev = { playing: clock.playing, m: nowM() }) {
  live = false;
  if (cmd === "play") setClock(true, m);
  else if (cmd === "pause") setClock(false, m);
  else if (cmd === "seek") setClock(clock.playing, m);
  else return;
  if (cmd !== "seek") bezel(cmd === "play");
  const text = cmd === "play" ? t("{name} played", { name: who }) : cmd === "pause" ? t("{name} paused", { name: who }) : t("{name} jumped to {time}", { name: who, time: fmt(m) });
  osd(text, () => act(prev.playing ? "play" : "pause", prev.m));
  renderInk();
}
function view() {
  const set = playSet();
  return { set: set ? set.id : null, playing: clock.playing, wall_us: wallOf(nowM()), at_us: nowUs(), rate, focus: play.view ? play.keys[play.view - 1] : null, mode: play.mode, slots: play.slots, order: play.keys };
}
function share() {
  if (watchParty && st && st.party.on) invoke("party_act", { view: view() }).catch(() => {});
}
function joinParty() {
  partyLast = null;
  partyLayout = null;
  return invoke("party_on", { on: watchParty, view: view() }).catch(() => {});
}
const newer = (a, b) => !b || (a.seq !== b.seq ? a.seq > b.seq : a.stamp !== b.stamp ? a.stamp > b.stamp : a.origin > b.origin);
function applyParty(p) {
  if (p.set && p.set !== selected && !st.sets.some((s) => s.id === p.set)) return;
  partyLast = p;
  const mine = p.origin === st.id;
  if (quick && !mine) quick.moved = true;
  const who = nameOf(p.origin);
  if (p.set && p.set !== selected) { selectSet(p.set); if (!mine) osd(t("{name} opened {set}", { name: who, set: setTime(p.set) })); }
  partyLayout = { set: p.set, mode: p.mode, slots: p.slots, order: p.order, focus: p.focus };
  if (p.rate && p.rate !== rate) setRate(p.rate);
  const set = playSet();
  if (!set || !p.set || typeof p.wall_us !== "number") return;
  const m = Math.max(0, (p.wall_us - base()) / 1e6 + (p.playing ? ((nowUs() - p.at_us) / 1e6) * rate : 0));
  if (mine) { if (p.playing !== clock.playing || Math.abs(m - nowM()) > JUMP_MIN) setClock(p.playing, m); }
  else if (p.playing !== clock.playing) apply(p.playing ? "play" : "pause", m, who);
  else if (Math.abs(m - nowM()) > JUMP_MIN) apply("seek", m, who);
  else setClock(p.playing, m);
}
function act(cmd, m, prev) {
  if (!playSet()) return;
  apply(cmd, m, t("You"), prev);
  share();
}
window.__TAURI__.event.listen("party", () => refresh());
const toggle = () => act(clock.playing ? "pause" : "play", nowM());
const seekTo = (m) => act("seek", Math.max(0, m));
const jump = (delta) => seekTo(Math.min(total(), nowM() + delta));

function indexOf(state) {
  const byKey = new Map(state.events.map((e) => [e.key, e]));
  const setOfKey = new Map();
  for (const s of state.sets) for (const k of s.events) if (!setOfKey.has(k)) setOfKey.set(k, s);
  return { byKey, setOfKey, items: new Map(), all: null };
}
const setOf = (e) => idx.setOfKey.get(e.key) || null;
const setLength = (s) => fmt(((s.end_us || nowUs()) - s.start_us) / 1e6);
function setSummary(s) {
  const notes = s.events.filter((k) => { const e = idx.byKey.get(k); return e && e.kind === "note"; }).length;
  return [setLength(s), notes ? t(notes === 1 ? "{n} note" : "{n} notes", { n: notes }) : ""].filter(Boolean).join(" · ");
}
function itemOf(e, set) {
  const r = role(e);
  const at = (us) => (us === null || us === undefined || !set ? null : (us - set.start_us) / 1e6);
  return { ...e, role: r, setId: set ? set.id : null, m: at(e.local_us), end: at(e.end_us), who: e.plugin ? `${e.plugin.name || e.plugin.id} ${e.plugin.version || ""}`.trim() : nameOf(e.author), body: eventText(e), speaker: e.file ? nameOf(e.file) : nameOf(e.author) };
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
function renderEvents() {
  items = setItems(playSet());
  const words = $("filter").value.toLowerCase().split(/\s+/).filter(Boolean);
  const shown = (words.length ? allItems() : items).filter((n) => listed(n.role) && matches(n, words)).sort((x, y) => (x.setId === y.setId ? (x.m ?? -1) - (y.m ?? -1) : x.setId < y.setId ? 1 : -1));
  const speech = items.filter((n) => n.role === "speech").sort((x, y) => x.m - y.m);
  renderSide(shown.length, speech.length);
  renderTimeline(Math.round(total()));
  renderEventList(shown, words);
  renderConvo(speech);
}
function renderSide(shown, speech) {
  $("sideevents").textContent = `${t("Events")} ${shown}`;
  $("sideconvo").hidden = !speech;
  if (!speech) side = "events";
  press($("sideevents"), side === "events");
  press($("sideconvo"), side === "convo");
  $("notelist").hidden = side !== "events";
  $("convolist").hidden = side !== "convo";
  $("filter").hidden = side !== "events";
  $("notestoggle").textContent = `${t("Events")} ${shown}`;
}
function renderTimeline(tot) {
  if (changed("chapters", [items.filter((n) => n.role === "chapter").map((n) => [n.key, n.m, n.end, n.body]), tot])) renderChapters(tot);
  if (changed("lane", [items.filter((n) => n.role === "moment").map((n) => [n.key, n.m, n.body, n.author]), tot, st.colors])) renderLane(tot);
}
function renderEventList(shown, words) {
  const set = playSet();
  if ($("notelist").querySelector("input") || !changed("events", [shown, lang, selected, st.colors])) return;
  $("notelist").replaceChildren(...(shown.length ? shown.map((n) => eventRow(n, set)) : [el("div", { className: "none", textContent: words.length ? t("Nothing found") : t("No events yet") })]));
}
function renderConvo(speech) {
  if (!changed("convo", [speech.map((n) => [n.key, n.m, n.speaker, n.body]), lang])) return;
  $("convolist").replaceChildren(...speech.map((n) => {
    const row = el("button", { className: "line" }, el("span", { className: "when", textContent: fmt(n.m) }), el("span", { className: "body" }, el("b", { textContent: n.speaker }), " ", n.body));
    row.dataset.key = n.key;
    row.onclick = () => seekTo(n.m);
    return row;
  }));
}
function timeText(n) {
  if (n.m === null) return t("Set");
  return n.end !== null ? `${fmt(n.m)}–${fmt(n.end)}` : fmt(n.m);
}
function goTo(n) {
  if (n.m === null) return;
  if (n.setId !== selected) openSet(n.setId, n.m);
  else seekTo(n.m);
}
function eventRow(n, set) {
  const when = el("button", { className: "when", textContent: timeText(n), disabled: !n.seekable || n.m === null });
  when.onclick = () => goTo(n);
  const meta = [n.who, n.role !== "drawing" && n.kind !== "note" && n.text ? kindLabel(n.kind) : "", set && n.setId !== set.id ? setTime(n.setId) : ""].filter(Boolean).join(" · ");
  const body = el("div", { className: "body" }, el("span", {}, ...inline(n.body)), el("span", { className: "who", textContent: meta }));
  const row = el("div", { className: "note" + (n.role === "chapter" ? " chapter" : "") }, el("i", { className: "swatch", style: `background:${n.plugin ? "var(--muted)" : colorOf(colorIndex(n.author))}` }), when, body);
  if (n.mine) {
    const acts = el("span", { className: "acts" });
    if (n.role !== "drawing") acts.append(el("button", { textContent: t("Edit"), onclick: () => editNote(n, row) }));
    acts.append(el("button", { textContent: t("Delete"), onclick: () => {
      run("event_action", { kind: "delete", id: n.id, text: "" });
      osd(t("Deleted"), () => run("event_action", { kind: "restore", id: n.id, text: "" }));
    } }));
    row.append(acts);
  }
  return row;
}
function renderChapters(tot) {
  const list = chaptersOf(items);
  $("chapters").hidden = !list.length;
  $("chapters").replaceChildren(...(tot ? list : []).map((n) => {
    const c = el("button", { className: "chap", title: `${timeText(n)} ${n.body}`, textContent: n.body, tabIndex: -1 });
    c.style.left = Math.max(0, (n.m / tot) * 100) + "%";
    c.style.width = Math.max(0, ((Math.min(n.end, tot) - n.m) / tot) * 100) + "%";
    c.onclick = () => seekTo(n.m);
    return c;
  }));
}
function renderLane(tot) {
  $("lane").replaceChildren(...(tot ? items.filter((n) => n.role === "moment" && n.m !== null) : []).map((n) => {
    const mark = el("button", { className: "mark" + (n.plugin ? " plugin" : ""), title: `${fmt(n.m)} ${n.body}`, ariaLabel: `${fmt(n.m)} ${n.body}`, tabIndex: -1 });
    mark.style.left = Math.max(0, Math.min(100, (n.m / tot) * 100)) + "%";
    if (!n.plugin) mark.style.background = colorOf(colorIndex(n.author));
    mark.onclick = () => seekTo(n.m);
    return mark;
  }));
}
const chapterAt = (m) => chaptersOf(items).filter((n) => n.m <= m && m <= n.end).pop();
function renderMoment(m) {
  if (tab !== "player" || !playSet()) return;
  const chapter = chapterAt(m);
  $("chapter").textContent = chapter ? chapter.body : "";
  const notes = activeAt(items.filter((n) => n.role === "moment" && n.text), m).slice(-OVERLAY_MAX);
  const typists = (watchParty && st.party.on ? st.party.typing || [] : []).filter((id) => id !== st.id);
  if (changed("overlay", [notes.map((n) => [n.key, n.m, n.body, n.author]), typists.map(nameOf), st.colors, lang])) $("overlay").replaceChildren(
    ...notes.map((n) => el("p", {}, el("b", { textContent: fmt(n.m) }), el("i", { className: "swatch", style: `background:${colorOf(colorIndex(n.author))}` }), ...inline(n.body))),
    ...typists.map((id) => el("p", { className: "typing", textContent: t("{name} is typing...", { name: nameOf(id) }) })));
  const speech = subtitles ? activeAt(items.filter((n) => n.role === "speech"), m) : [];
  for (const key of play.keys) {
    const tl = play.tiles[key];
    if (!tl) continue;
    const lines = speech.filter((n) => (n.file || n.author) === key).map((n) => n.body);
    tl.sub.hidden = !lines.length;
    if (tl.sub.textContent !== lines.join("\n")) tl.sub.textContent = lines.join("\n");
  }
  const current = speech.length ? speech[speech.length - 1].key : null;
  for (const row of $("convolist").children) {
    const on = row.dataset.key === current;
    if (on && !row.classList.contains("on") && clock.playing) row.scrollIntoView({ block: "nearest" });
    row.classList.toggle("on", on);
  }
}
function filterTag(tag) {
  if (tab !== "player") showTab("player");
  $("filter").value = "#" + tag;
  side = "events";
  $("notes").classList.add("open");
  renderEvents();
}
function openLink(link) {
  if (!st.sets.some((s) => s.id === link.set)) { flash(t("That set is not on this PC")); return; }
  showTab("player");
  openSet(link.set, link.m);
  if (link.view) setTimeout(() => setView(play, play.keys.indexOf(link.view) + 1), TICK_MS);
}
function editNote(n, row) {
  const input = el("input", { value: n.text, ariaLabel: t("Edit") });
  const done = () => { delete sigs.events; renderEvents(); };
  input.onkeydown = (e) => {
    e.stopPropagation();
    if (composing(e)) return;
    if (e.key === "Enter") { run("event_action", { kind: "edit", id: n.id, text: input.value }); input.remove(); done(); }
    if (e.key === "Escape") { e.preventDefault(); input.remove(); done(); }
  };
  input.onblur = () => { if (input.isConnected) { input.remove(); done(); } };
  row.replaceChildren(input);
  input.focus();
}
function stepItems(list, dir) {
  const m = nowM();
  const times = list.map((n) => n.m).filter((x) => x !== null).sort((a, b) => a - b);
  const next = dir > 0 ? times.find((x) => x > m + NOTE_STEP_GAP) : [...times].reverse().find((x) => x < m - NOTE_STEP_GAP);
  if (next !== undefined) seekTo(next);
  else osd(dir > 0 ? t("No next one") : t("No previous one"));
}
const stepNote = (dir) => stepItems(items.filter((n) => n.kind === "note" && n.role === "moment"), dir);
const stepEvent = (dir) => stepItems(items.filter((n) => (n.role === "moment" && n.kind !== "note") || n.role === "chapter"), dir);

function openQuick() {
  const set = playSet();
  if (!set) return;
  if (!quick) {
    quick = { resume: clock.playing, m: nowM(), wall: wallOf(nowM()), set: set.id, timed: true, moved: false };
    if (clock.playing) act("pause", nowM());
  }
  $("quick").hidden = false;
  renderQuick();
  $("quicktext").focus();
  invoke("party_typing", { on: true }).catch(() => {});
}
function renderQuick() {
  $("quicktime").textContent = quick.timed ? fmt(quick.m) : t("Whole set");
  const word = ($("quicktext").value.slice(0, $("quicktext").selectionStart).match(/#([\p{L}\p{N}_-]*)$/u) || [])[1];
  const known = [...new Set(st.events.flatMap((e) => e.tags || []))];
  const hints = word === undefined ? [] : known.filter((tg) => tg.toLowerCase().startsWith(word.toLowerCase()) && tg.toLowerCase() !== word.toLowerCase()).slice(0, TAG_HINTS);
  $("taghints").replaceChildren(...hints.map((tg) => el("button", { type: "button", className: "tag", textContent: "#" + tg, onmousedown: (e) => e.preventDefault(), onclick: () => pickTag(tg) })));
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
  invoke("party_typing", { on: false }).catch(() => {});
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
  const r = rects(count, id, 22, 14, 1.5);
  return svg("svg", { viewBox: "0 0 22 14", class: "lic" }, ...r.tiles.map((b, i) => svg("rect", { x: b.x, y: b.y, width: b.w, height: b.h, rx: 1, class: i === 0 && id !== "grid" && id !== "row" ? "big" : "" })));
}
function renderLayoutButtons() {
  for (const [id, s] of [["layoutrec", rec], ["layoutplay", play]]) {
    const n = Math.max(1, s.keys.length, s.slots || 0);
    const mode = effectiveMode(s);
    if (!changed(id, [mode, n, lang])) continue;
    $(id).replaceChildren(layoutIcon(mode, n));
    $(id).title = t(LAYOUTS.find((l) => l.id === mode).name);
  }
}
function pickLayout(s, anchor) {
  const present = Math.max(1, s.keys.length);
  const show = (count) => {
    const box = el("div", { className: "layoutpick" });
    const counts = el("div", { className: "seg counts" }, ...[...Array(MAX_SLOTS).keys()].map((i) => {
      const n = i + 1;
      const b = el("button", { textContent: n, disabled: n < present, className: n === count ? "on" : "", title: t("{n} players", { n }) });
      b.onclick = () => show(n);
      return b;
    }));
    const kinds = el("div", { className: "kinds" }, ...LAYOUTS.filter((l) => layoutFits(l.id, count)).map((l) => {
      const b = el("button", { className: "kind" + (effectiveMode(s) === l.id && Math.max(present, s.slots || 0) === count ? " on" : ""), title: t(l.name) }, layoutIcon(l.id, count), el("span", { textContent: t(l.name) }));
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
  if (!keepFocus || m.hidden) menuReturn = document.activeElement;
  m.replaceChildren(...list.map((it) => {
    if (it.separator) return el("hr");
    if (it.heading) return el("div", { className: "heading", textContent: it.heading });
    if (it.element) return it.element;
    const b = el("button", { role: "menuitem", disabled: (!it.run && !it.keep) || !!it.disabled, className: [it.on ? "on" : "", it.danger ? "danger" : "", it.keep ? "static" : ""].join(" ").trim() }, it.lead || it.icon || null, el("span", { className: "label", textContent: it.label }), it.note ? el("span", { className: "note" + (it.warn ? " warn" : ""), textContent: it.note }) : null, it.right ? el("span", { className: "right", textContent: it.right }) : null);
    b.onclick = () => { if (!it.run) return; closeMenu(); it.run(); };
    return b;
  }));
  m.hidden = false;
  const r = at instanceof Event ? { left: at.clientX || at.target.getBoundingClientRect().left, bottom: at.clientY || at.target.getBoundingClientRect().bottom } : at.getBoundingClientRect();
  m.style.left = Math.max(4, Math.min(r.left, innerWidth - m.offsetWidth - 4)) + "px";
  m.style.top = Math.max(4, Math.min(r.bottom + 4, innerHeight - m.offsetHeight - 4)) + "px";
  const first = m.querySelector("button:not(:disabled)");
  if (first && !keepFocus) first.focus();
}
function closeMenu() {
  if ($("menu").hidden) return;
  $("menu").hidden = true;
  if (menuReturn && menuReturn.isConnected) menuReturn.focus();
}
document.addEventListener("mousedown", (e) => {
  if (!$("menu").contains(e.target)) closeMenu();
  if (!$("pop").contains(e.target)) $("pop").hidden = true;
  if (!$("extra").contains(e.target) && e.target.closest("#more") === null) $("extra").classList.remove("open");
});
function stageMenu(e) {
  const set = playSet();
  if (!set) return;
  openMenu([
    { label: t("Add note"), run: openQuick },
    { label: t("Copy link to this moment"), run: () => navigator.clipboard.writeText(momentLink(set.id, nowM(), play.view ? play.keys[play.view - 1] : null)).then(() => osd(t("Link copied"))) },
    { label: t("Clear drawing"), run: () => clearInk(false) },
  ], e);
}

function flash(text, info) {
  flashText = t(String(text));
  flashInfo = !!info;
  clearTimeout(flashTimer);
  flashTimer = setTimeout(() => { flashText = ""; renderNotice(); }, NOTICE_MS);
  renderNotice();
}
function renderNotice() {
  const problems = st ? [st.notice, st.ffmpeg, st.capture.status, st.disk.low ? t("Low disk space: {free} free", { free: bytes(st.disk.free) }) : "", ...st.peers.filter((p) => p.message).map((p) => `${nameOf(p.id)}: ${p.message}`)].map((x) => (x ? t(x) : "")) : [];
  const update = pending ? t("Version {version} is available", { version: pending.version }) : updateText;
  const progress = exporting ? t("Exporting {n}%", { n: Math.floor(exporting.percent) }) + (exporting.skipped ? " " + t("without {names} (not on this PC yet)", { names: exporting.skipped }) : "") : "";
  const list = [flashText, ...problems, progress, update].filter(Boolean);
  const text = list.join("  ·  ");
  $("noticetext").textContent = text;
  $("notice").hidden = !text || (text === dismissed && !exporting);
  $("notice").classList.toggle("info", list.every((x) => (x === flashText && flashInfo) || x === update || x === progress));
  $("updatebtn").hidden = !pending;
  $("exportcancel").hidden = !exporting;
}
$("notice").onclick = (e) => { if (e.target.tagName !== "BUTTON") { dismissed = $("noticetext").textContent; renderNotice(); } };
$("exportcancel").onclick = () => run("cancel_export", null, null);
function exportSet(set) {
  if (!set) { flash(t("Choose a set in the Player first")); return; }
  invoke("export_set", { room: set.room, set: set.id }).then((r) => {
    if (r) { exporting = { percent: 0, ...exporting, skipped: r.skipped.map(nameOf).join(", ") }; renderNotice(); }
  }).catch(flash);
}
window.__TAURI__.event.listen("export", (e) => {
  const x = e.payload;
  if (x.done) { exporting = null; flash(x.error || t("Export finished"), !x.error); return; }
  exporting = { ...exporting, percent: x.percent };
  renderNotice();
});
async function checkUpdate(manual) {
  try {
    pending = await window.__TAURI__.updater.check();
  } catch {
    if (manual) flash(t("Update check failed"));
    return;
  }
  if (!pending && manual) flash(t("Up to date"), true);
  renderNotice();
}
$("updatebtn").onclick = async (e) => {
  e.stopPropagation();
  const update = pending;
  pending = null;
  updateText = t("Installing {version}; the app will restart", { version: update.version });
  renderNotice();
  await update.downloadAndInstall().catch(() => { updateText = t("Update failed"); renderNotice(); });
};

function renderCapture() {
  const c = st.capture;
  const value = c.source ? JSON.stringify(c.source) : "";
  if (document.activeElement !== $("source")) {
    if (![...$("source").options].some((o) => o.value === value)) $("source").append(el("option", { value, textContent: c.source.kind === "window" ? c.source.title : t("Display {n}", { n: c.source.index }) }));
    $("source").value = value;
  }
  $("source").classList.toggle("lost-src", !!c.source_lost);
  $("lost").hidden = !c.source_lost;
  if (document.activeElement !== $("mic")) $("mic").value = [...$("mic").options].some((o) => o.value === c.mic) ? c.mic : "";
  for (const kind of ["app", "mic"]) {
    if (document.activeElement !== $(kind + "vol")) $(kind + "vol").value = c.mix[kind].volume;
    $(kind + "mute").classList.toggle("alt", c.mix[kind].muted);
    $(kind + "mute").setAttribute("aria-pressed", c.mix[kind].muted);
    $(kind + "meter").classList.toggle("muted", c.mix[kind].muted);
  }
  const q = c.quality;
  if (document.activeElement !== $("quality")) $("quality").value = q.id;
  $("qinfo").textContent = [q.width ? `${q.width}×${q.height}` : "", `${q.fps} fps`, `${(q.kbps / 1000).toFixed(1)} Mbit/s`, q.codec.toUpperCase()].filter(Boolean).join(" · ");
}
async function fillSources() {
  const list = await invoke("list_sources");
  if (!changed("sources", list)) return;
  $("source").replaceChildren(el("option", { value: "", textContent: t("No source") }), ...list.map((s) => el("option", { value: JSON.stringify(s.id), textContent: s.id.kind === "display" ? t("Display {n}", { n: s.id.index }) : s.id.title || s.name })));
  if (st) renderCapture();
}
async function fillMics() {
  const list = await invoke("list_mics");
  if (!changed("mics", list)) return;
  $("mic").replaceChildren(el("option", { value: "", textContent: t("Default") }), ...list.map((m) => el("option", { value: m.id, textContent: m.name })));
  if (st) renderCapture();
}
async function fillProcesses() {
  const names = await invoke("list_processes");
  const current = st ? st.audio_process : $("proc").value;
  $("proc").replaceChildren(el("option", { value: "", textContent: t("Automatic") }), ...names.map((n) => el("option", { textContent: n })));
  $("proc").value = current;
}
const setMix = (kind, mute) => run("set_mix", { kind, volume: Number($(kind + "vol").value), muted: mute });
for (const kind of ["app", "mic"]) {
  $(kind + "vol").oninput = () => setMix(kind, st.capture.mix[kind].muted);
  $(kind + "mute").onclick = () => setMix(kind, !st.capture.mix[kind].muted);
  $(kind + "scale").replaceChildren(...METER_TICKS.map((db) => el("span", { textContent: db, style: `left:${meterPos(db) * 100}%` })));
  $(kind + "meter").style.setProperty("--yellow", meterPos(METER_YELLOW_DB) * 100 + "%");
  $(kind + "meter").style.setProperty("--red", meterPos(METER_RED_DB) * 100 + "%");
}
function meterPos(db) {
  return Math.max(0, Math.min(1, (db - METER_MIN_DB) / -METER_MIN_DB));
}
const toDb = (v) => (v > 0 ? 20 * Math.log10(v) : -Infinity);
$("source").onfocus = fillSources;
$("source").onchange = () => run("set_source", { source: $("source").value ? JSON.parse($("source").value) : null }, null);
$("mic").onfocus = fillMics;
$("mic").onchange = () => run("set_mic", { id: $("mic").value }, null);
$("quality").onchange = () => run("set_quality", { id: $("quality").value });
$("usedisplay").onclick = () => run("use_display", null, fillSources);
function drawMeter(kind, v) {
  const now = performance.now();
  const m = meters[kind];
  m.v = v;
  if (v >= m.peak || now - m.at > PEAK_HOLD_MS) { m.peak = v; m.at = now; }
  if (v >= 1) m.clip = now;
  const box = $(kind + "meter");
  box.style.setProperty("--v", meterPos(toDb(v)) * 100 + "%");
  box.style.setProperty("--p", meterPos(toDb(m.peak)) * 100 + "%");
  box.classList.toggle("clip", now - m.clip < CLIP_HOLD_MS);
  box.classList.toggle("quiet", !m.peak);
  const db = toDb(m.peak);
  $(kind + "db").textContent = db > METER_MIN_DB ? `${Math.round(db)} dB` : "";
}
window.__TAURI__.event.listen("levels", (e) => { for (const kind of ["app", "mic"]) drawMeter(kind, Math.max(0, e.payload[kind] || 0)); });

function renderAsks() {
  if (!changed("asks", [st.asks, lang])) return;
  $("asks").hidden = !st.asks.length;
  $("asks").replaceChildren(...st.asks.map((a) => {
    const answer = (approve) => () => run("answer_ask", { id: a.id, approve });
    return el("div", { className: "notice info" }, el("span", { textContent: t("{name} wants to join", { name: a.name || a.id.slice(0, 10) }) }),
      el("button", { className: "primary", textContent: t("Approve"), onclick: answer(true) }), el("button", { textContent: t("Decline"), onclick: answer(false) }));
  }));
}

function refresh() {
  if (refreshing) { refreshAgain = true; return refreshing; }
  refreshing = (async () => {
    try { do { refreshAgain = false; await update(); } while (refreshAgain); }
    finally { refreshing = null; }
  })();
  return refreshing;
}
const STEPS = [renderTop, renderAsks, renderNotice, renderStart, renderRecord, renderCapture, renderPlayer, renderSets, renderEvents, renderWatchParty, renderTools, renderHome, renderLayoutButtons, renderRecovery];
async function update() {
  st = await invoke("state");
  idx = indexOf(st);
  if (lang !== st.lang) { lang = st.lang; setLanguage(lang); for (const k of Object.keys(sigs)) delete sigs[k]; fillSources().catch(() => {}); fillMics().catch(() => {}); }
  if (!ready) { ready = true; invoke("ui_ready"); pollPlugins(); }
  if (!selected || !st.sets.some((s) => s.id === selected)) {
    const first = st.sets[0];
    if (first) selectSet(first.id);
    else selected = null;
  }
  const roomId = st.room ? st.room.id : "";
  if (partyRoom !== roomId) { partyRoom = roomId; await joinParty(); st = await invoke("state"); idx = indexOf(st); }
  if (!drag && watchParty && st.party.on && newer(st.party.shared, partyLast)) applyParty(st.party.shared);
  if (watchParty && st.party.on && !st.party.shared.set && playSet()) share();
  for (const step of STEPS) try { step(); } catch (e) { console.error(e); }
}
function renderRecovery() {
  $("recovery").hidden = !st.recovery;
  $("recoverymsg").textContent = st.recovery ? t(st.recovery) : "";
  $("rinfo").textContent = st.previous_version ? t("Version {version}, previous run {previous}", { version: st.version, previous: st.previous_version }) : t("Version {version}", { version: st.version });
}

let full = null;
function toggleFull() {
  if (full) full.classList.remove("full");
  full = full || tab === "room" ? null : tab === "player" ? $("stage") : $("rectiles");
  if (full) full.classList.add("full");
  window.__TAURI__.window.getCurrentWindow().setFullscreen(!!full).catch(() => {});
  wake();
}
const toggleFill = () => { document.body.classList.toggle("fill"); wake(); };
function wake() {
  $("stage").classList.remove("idle");
  clearTimeout(idleTimer);
  if (full || document.body.classList.contains("fill")) idleTimer = setTimeout(() => { if (clock.playing) $("stage").classList.add("idle"); }, IDLE_MS);
}
document.addEventListener("mousemove", wake);

const copyInvite = () => invoke("create_invite", { reuse: true }).then((code) => navigator.clipboard.writeText(code)).then(() => flash(t("Invite code copied"), true)).catch(flash);
const cancelInvite = () => run("cancel_invites", null, () => { flash(t("Invite cancelled"), true); refresh(); });
const createRoom = () => invoke("create_room", { name: "" }).then(() => { showTab("room"); return refresh().then(copyInvite); }).catch(flash);
const leaveRoom = () => { if (st.room && confirm(t("Leave {room}? Its recordings stay until you delete them.", { room: st.room.name }))) run("leave_room", { id: st.room.id }); };
let joining = false;
function joinWith(input) {
  if (joining) return;
  joining = true;
  run("join_room", { code: input.value }, () => { input.value = ""; if ($("joindlg").open) $("joindlg").close(); refresh(); }).finally(() => { joining = false; });
}
function showTab(name) {
  if (name !== tab) closeQuick(false);
  if (full) toggleFull();
  tab = name;
  if (st) { renderTop(); place(rec); place(play); }
  refresh();
}
function openJoin() {
  if (!st.room || st.room.pending) { $("joincode").focus(); return; }
  $("joindlg").showModal();
  $("joincode2").focus();
}
function openRename() {
  if (!st.room) return;
  $("renameinput").value = st.room.name;
  $("renamedlg").showModal();
  $("renameinput").select();
}
async function openCheck() {
  if (!st.room) return;
  $("checkdlg").showModal();
  $("checkbody").replaceChildren(el("div", { className: "muted", textContent: t("Checking...") }));
  const results = await invoke("check_room").catch((e) => { flash(e); return []; });
  const icon = { ok: "✓", warn: "!", fail: "✕", info: "·" };
  $("checkbody").replaceChildren(...results.map((r) => {
    const m = memberState(r.id);
    const lines = r.lines ? r.lines.map((l) => el("div", { className: "line " + l.level }, el("span", { className: "icon", textContent: icon[l.level] || "·" }), el("span", { textContent: l.text }))) : [el("div", { className: "line fail" }, el("span", { className: "icon", textContent: "✕" }), el("span", { textContent: t("No answer") }))];
    return el("section", { className: "checkpart" }, el("h4", {}, avatar(r.id, true), nameOf(r.id), el("span", { className: "muted", textContent: [m.state, m.path].filter(Boolean).join(" · ") })), ...lines);
  }));
}
function boardCards() {
  const at = (e) => e.id - ((peerOf(e.author) || {}).offset_us || 0);
  const out = {};
  for (const e of st.events) if (e.kind === "board" && e.values && (!out[e.values.card] || at(e) >= at(out[e.values.card]))) out[e.values.card] = e;
  return out;
}
function renderHome() {
  if (tab !== "room" || !st.room || st.room.pending) return;
  const all = people();
  const online = 1 + st.peers.filter((p) => p.connected).length;
  $("homename").textContent = st.room.name;
  $("homesub").textContent = t("{n} members · {online} online", { n: all.length, online });
  if (changed("people", [all.map((id) => [id, memberState(id), nameOf(id), colorIndex(id), recOf(id)]), lang])) $("people").replaceChildren(...all.map((id) => {
    const m = memberState(id);
    const card = el("div", { className: "person" + (m.relay ? " thin" : "") }, avatar(id), el("div", { className: "pinfo" }, el("b", { textContent: nameOf(id) }), el("span", { className: "muted", textContent: [m.state, m.path].filter(Boolean).join(" · ") })));
    if (id !== st.id) card.oncontextmenu = (e) => { e.preventDefault(); memberMenu(id, e); };
    return card;
  }));
  if (!$("boardcards").querySelector("textarea") && changed("board", [st.events.filter((e) => e.kind === "board").map((e) => e.key), all, lang, st.names, st.colors])) renderBoard();
  const sets = st.sets.slice(0, HOME_SETS).map((s) => ({ s, summary: setSummary(s) }));
  if (changed("homesets", [sets.map(({ s, summary }) => [s.id, s.live, summary, s.players.map((p) => [p.owner, nameOf(p.owner), colorIndex(p.owner), recOf(p.owner), (peerOf(p.owner) || {}).connected])]), lang])) $("homesets").replaceChildren(...(sets.length ? sets.map(({ s, summary }) => {
    const row = el("button", { className: "hrow" }, el("span", { className: "t" }, s.live ? el("i", { className: "dot rec" }) : null, s.live ? t("Live") : setTime(s.id)), el("span", { className: "avs" }, ...s.players.map((p) => avatar(p.owner, true))), el("small", { className: "muted", textContent: summary }));
    row.onclick = () => { showTab("player"); openSet(s.id); };
    return row;
  }) : [el("div", { className: "none", textContent: t("No sets yet") })]));
  const notes = st.events.filter((e) => e.kind === "note" && !e.plugin).sort((a, b) => b.id - a.id).slice(0, HOME_NOTES).map((e) => itemOf(e, setOf(e)));
  if (changed("homenotes", [notes.map((n) => [n.key, n.body, n.who, n.setId, n.m, n.author]), st.colors, lang])) $("homenotes").replaceChildren(...(notes.length ? notes.map((n) => {
    const row = el("button", { className: "hrow note" }, el("i", { className: "swatch", style: `background:${colorOf(colorIndex(n.author))}` }), el("span", { className: "body" }, ...inline(n.body)), el("small", { className: "muted", textContent: [n.who, n.setId ? `${setTime(n.setId)} ${n.m !== null ? fmt(n.m) : ""}` : ""].filter(Boolean).join(" · ") }));
    row.onclick = () => { if (n.setId) { showTab("player"); goTo(n); } };
    return row;
  }) : [el("div", { className: "none", textContent: t("No notes yet") })]));
}
function renderBoard() {
  const cards = [{ card: "team", title: t("Team") }, ...people().map((id) => ({ card: id, title: nameOf(id), color: colorIndex(id) }))];
  const newest = boardCards();
  $("boardcards").replaceChildren(...cards.map((c) => {
    const e = newest[c.card] || null;
    const body = el("div", { className: "cardbody" }, ...(e && e.text ? markdown(e.text) : [el("p", { className: "muted", textContent: t("Empty") })]));
    const edit = el("button", { className: "icon text", textContent: t("Edit") });
    const box = el("article", { className: "card" + (c.card === "team" ? " team" : "") }, el("header", {}, c.color !== undefined ? el("i", { className: "swatch", style: `background:${colorOf(c.color)}` }) : null, el("h3", { textContent: c.title }), el("span", { className: "spacer" }), edit), body);
    edit.onclick = () => {
      const area = el("textarea", { value: e ? e.text : "", rows: 10 });
      const save = el("button", { className: "primary", textContent: t("Save") });
      const cancel = el("button", { textContent: t("Cancel") });
      const done = () => { delete sigs.board; renderBoard(); };
      save.onclick = () => run("add_event", { kind: "board", text: area.value, target: {}, values: { card: c.card } }, () => refresh().then(done));
      cancel.onclick = done;
      area.onkeydown = (k) => k.stopPropagation();
      body.replaceChildren(area, el("div", { className: "row end" }, cancel, save));
      area.focus();
    };
    return box;
  }));
}
function openPlugins() {
  if (!st.room) return;
  delete sigs.plugins;
  renderPlugins().catch(flash);
  if (!$("pluginsdlg").open) $("pluginsdlg").showModal();
}
function pollPlugins() {
  if (st && st.room && !document.hidden) renderPlugins().catch(() => {});
}
async function renderPlugins() {
  const gen = ++pluginsGen;
  const [list, info] = await Promise.all([invoke("room_plugins"), invoke("jobs")]);
  if (gen !== pluginsGen) return;
  jobInfo = info;
  for (const p of list) pluginNames[p.id] = p.name;
  if (changed("tags", info.tags)) for (const k of ["events", "overlay", "homenotes", "board"]) delete sigs[k];
  if ($("pluginsdlg").open && changed("plugins", [list, lang])) $("pluginlist").replaceChildren(...list.map((p) => {
    const settings = { ...p.settings };
    const save = (on) => invoke("set_room_plugin", { id: p.id, on, settings }).then(renderPlugins).catch(flash);
    const on = el("input", { type: "checkbox", checked: p.on, onchange: (e) => save(e.target.checked) });
    const options = p.options.map((o) => el("label", { className: "opt" }, el("span", { textContent: t(o.key.charAt(0).toUpperCase() + o.key.slice(1)) }), el("select", { onchange: (e) => { settings[o.key] = e.target.value; save(p.on); } }, ...o.choices.map((c) => el("option", { value: c, textContent: c, selected: settings[o.key] === c })))));
    const get = p.ready ? null : el("button", { textContent: t("Download {size}", { size: bytes(p.download_bytes) }), onclick: (e) => { e.target.disabled = true; e.target.textContent = t("Downloading..."); invoke("prepare_plugin", { id: p.id }).then(renderPlugins).catch((err) => { flash(err); delete sigs.plugins; renderPlugins().catch(() => {}); }); } });
    return el("div", { className: "plugin" }, el("label", { className: "check" }, on, el("b", { textContent: p.name }), el("span", { className: "muted", textContent: p.version })), el("div", { className: "row wrap" }, ...options, get));
  }));
  renderJobs();
}
function renderJobs() {
  if (!jobInfo || !$("pluginsdlg").open) return;
  const running = jobInfo.running;
  const label = { waiting: t("Waiting"), claimed: t("Running"), done: t("Done"), failed: t("Failed") };
  const rows = jobInfo.jobs.slice().sort((a, b) => (a.set < b.set ? 1 : a.set > b.set ? -1 : 0));
  $("joblist").replaceChildren(...(rows.length ? rows.map((j) => {
    const mine = running && running.job === j.job;
    const state = mine ? `${t("Running")} ${Math.floor((running.progress || 0) * 100)}%` : [label[j.state] || j.state, j.by ? nameOf(j.by) : "", j.error || ""].filter(Boolean).join(" · ");
    const action = mine ? el("button", { textContent: t("Cancel"), onclick: () => run("cancel_job", null, null) }) : j.state === "failed" ? el("button", { textContent: t("Retry"), onclick: () => run("retry_job", { job: j.job }, renderPlugins) }) : null;
    return el("div", { className: "jrow" + (j.state === "failed" ? " failed" : "") }, el("span", { textContent: pluginNames[j.plugin] || j.plugin }), el("span", { textContent: setTime(j.set) }), el("span", { textContent: nameOf(j.owner) }), el("span", { className: "muted", textContent: state }), action || el("span"));
  }) : [el("div", { className: "none", textContent: t("No jobs") })]));
}
window.__TAURI__.event.listen("job", (e) => {
  if (jobInfo) jobInfo.running = e.payload;
  renderJobs();
  renderSync();
});
async function openStorage() {
  $("storagedlg").showModal();
  const info = await invoke("data_info").catch(() => null);
  $("storagehead").textContent = info ? t("{used} used · {free} free", { used: bytes(info.used_bytes), free: bytes(info.free_bytes) }) : "";
  $("storagelist").replaceChildren(...st.sets.map((s) => {
    const row = el("div", { className: "srow" }, el("span", { textContent: setTime(s.id) }), el("span", { className: "muted", textContent: setLength(s) }), el("span", { className: "size", textContent: bytes(s.progress.have) }));
    row.oncontextmenu = (e) => { e.preventDefault(); setMenu(s, e); };
    return row;
  }));
}
function openSettings() {
  if (st) {
    $("name").value = st.name;
    $("share").checked = st.share;
    $("limit").value = st.limit_mbps || "";
    $("keepdays").value = st.keep_days || "";
    $("language").value = st.language;
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
const changeRate = (dir) => { setRate(SPEEDS[Math.max(0, Math.min(SPEEDS.length - 1, SPEEDS.indexOf(rate) + dir))]); osd(t("Speed {rate}x", { rate })); share(); };
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

$("rec").onclick = () => run(st && st.recording ? "stop_recording" : "start_recording");
$("tabroom").onclick = () => showTab("room");
$("tabrecord").onclick = () => showTab("record");
$("homeinvite").onclick = copyInvite;
$("homemore").onclick = (e) => homeMenu(e.currentTarget);
$("tabplayer").onclick = () => showTab("player");
$("roombtn").onclick = (e) => roomMenu(e.currentTarget);
$("play").onclick = toggle;
$("back").onclick = () => jump(-STEP.normal);
$("fwd").onclick = () => jump(STEP.normal);
$("addnote").onclick = openQuick;
$("wp").onclick = () => { watchParty = !watchParty; joinParty().then(refresh); renderWatchParty(); place(play); };
$("osdundo").onclick = () => { const u = osdUndo; $("osd").hidden = true; osdUndo = null; if (u) u(); };
$("live").onclick = () => { live = !live; if (live) setClock(true, Math.max(0, total() - LIVE_LAG)); renderPlayer(); };
$("speed").onchange = (e) => { setRate(Number(e.target.value)); share(); };
$("volume").oninput = () => setVolume(Number($("volume").value));
$("mute").onclick = toggleMute;
$("cc").onclick = () => { subtitles = !subtitles; store("subtitles", subtitles); renderPlayer(); };
$("more").onclick = () => $("extra").classList.toggle("open");
$("fill").onclick = toggleFill;
$("full").onclick = toggleFull;
$("layoutrec").onclick = (e) => pickLayout(rec, e.currentTarget);
$("layoutplay").onclick = (e) => pickLayout(play, e.currentTarget);
$("setsrail").onclick = () => $("setsrail").setAttribute("aria-expanded", $("sets").classList.toggle("open"));
$("notestoggle").onclick = () => $("notestoggle").setAttribute("aria-expanded", $("notes").classList.toggle("open"));
$("sideevents").onclick = () => { side = "events"; renderEvents(); };
$("sideconvo").onclick = () => { side = "convo"; renderEvents(); };
for (const b of $("tools").querySelectorAll("[data-tool]")) b.onclick = () => { ink.tool = b.dataset.tool; renderTools(); };
$("clearink").onclick = () => clearInk(false);
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
  };
};
const peek = { video: el("video", { muted: true, preload: "auto" }), want: null };
peek.video.onseeked = () => {
  const c = $("peek").querySelector("canvas");
  c.getContext("2d").drawImage(peek.video, 0, 0, c.width, c.height);
  if (peek.want !== null && Math.abs(peek.want - peek.video.currentTime) > SEEK_TOLERANCE) peek.video.currentTime = peek.want;
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
$("startname").oninput = () => renderStart();
$("startname").onchange = () => run("set_name", { name: $("startname").value });
$("newroom").onclick = () => run("set_name", { name: $("startname").value || st.name }, createRoom);
$("joincode").oninput = () => renderStart();
$("join").onclick = () => run("set_name", { name: $("startname").value || st.name }, () => joinWith($("joincode")));
$("joincode").onkeydown = (e) => { if (e.key === "Enter" && !composing(e) && !$("join").disabled) $("join").click(); };
$("join2").onclick = () => joinWith($("joincode2"));
$("joincode2").onkeydown = (e) => { if (e.key === "Enter" && !composing(e)) joinWith($("joincode2")); };
$("pendingcancel").onclick = () => st.room && run("leave_room", { id: st.room.id });
$("renamesave").onclick = () => run("rename_room", { name: $("renameinput").value }, () => { $("renamedlg").close(); refresh(); });
$("renameinput").onkeydown = (e) => { if (e.key === "Enter" && !composing(e)) $("renamesave").click(); };
$("recheck").onclick = openCheck;
$("proc").onchange = () => run("set_audio_process", { name: $("proc").value }, null);
$("keepdays").onchange = () => run("set_keep_days", { days: $("keepdays").value ? Number($("keepdays").value) : null }, null);
$("name").onchange = () => run("set_name", { name: $("name").value });
$("language").onchange = () => run("set_language", { language: $("language").value });
$("share").onchange = () => run("set_share", { on: $("share").checked }, null);
$("limit").onchange = () => run("set_limit", { mbps: $("limit").value ? Number($("limit").value) : null }, null);
$("rfolder").onclick = () => run("open_data_folder", null, null);
$("rreport").onclick = () => run("save_report", { includeAddresses: false, audioProcess: "" }, (p) => { $("rinfo").textContent = p; });
$("rreset").onclick = () => run("reset", { everything: false, confirm: "" }, null);
$("rcontinue").onclick = () => run("dismiss_recovery");

const menuActions = {
  "settings": openSettings,
  "storage": openStorage,
  "create-room": createRoom,
  "join-room": openJoin,
  "copy-invite": copyInvite,
  "cancel-invite": cancelInvite,
  "rename-room": openRename,
  "check-room": openCheck,
  "plugins": openPlugins,
  "leave-room": leaveRoom,
  "export-set": () => exportSet(playSet()),
  "tab-room": () => showTab("room"),
  "tab-record": () => showTab("record"),
  "tab-player": () => showTab("player"),
  "fullscreen": toggleFull,
  "fill": toggleFill,
  "all-players": () => setView(current(), 0),
  "next-player": () => cycle(current(), 1),
  "prev-player": () => cycle(current(), -1),
  "shortcuts": () => toggleDialog($("shortcuts")),
  "update": () => checkUpdate(true),
};
window.__TAURI__.event.listen("menu", (e) => { if (menuActions[e.payload]) menuActions[e.payload](); });
window.__TAURI__.event.listen("joined", (e) => flash(t("{name} joined", { name: e.payload }), true));
window.__TAURI__.event.listen("flash", (e) => flash(e.payload, true));
$("filter").oninput = () => renderEvents();

const once = (run) => (e) => { if (!e.repeat) run(); };
const PLAYER_KEYS = {
  " ": once(toggle),
  k: once(toggle),
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
  a: () => stepEvent(-1),
  d: () => stepEvent(1),
  t: toggleFill,
  x: () => clearInk(false),
  u: () => $("osdundo").click(),
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
  $("pop").hidden = true;
  if (typing()) document.activeElement.blur();
  else if (full) toggleFull();
  else if (document.body.classList.contains("fill")) toggleFill();
  else { $("sets").classList.remove("open"); $("notes").classList.remove("open"); $("extra").classList.remove("open"); }
}
document.addEventListener("keydown", (e) => {
  if (composing(e)) return;
  if (!$("menu").hidden) { menuKey(e); return; }
  const ctrl = e.ctrlKey && !e.altKey && !e.metaKey;
  if (ctrl && e.shiftKey && e.key.toLowerCase() === "i") { e.preventDefault(); invoke("open_debug"); return; }
  if (document.querySelector("dialog[open]")) {
    if (e.key === "?" && $("shortcuts").open) { $("shortcuts").close(); e.preventDefault(); }
    return;
  }
  if (ctrl && e.key === "Tab") { e.preventDefault(); showTab(TABS[(TABS.indexOf(tab) + (e.shiftKey ? TABS.length - 1 : 1)) % TABS.length]); return; }
  if (ctrl && ["1", "2", "3"].includes(e.key)) { e.preventDefault(); showTab(TABS[Number(e.key) - 1]); return; }
  if (e.key === "Escape") { escapeKey(); return; }
  if (typing() || nativeKey(e) || e.metaKey || e.altKey || !st) return;
  const player = tab === "player";
  const key = e.key.normalize("NFKC");
  const k = key.length === 1 ? key.toLowerCase() : key;
  let run = null;
  if (e.ctrlKey) run = player && (key === "ArrowLeft" || key === "ArrowRight") ? () => jump(key === "ArrowLeft" ? -STEP.huge : STEP.huge) : null;
  else if (key === "?") run = () => toggleDialog($("shortcuts"));
  else if (tab !== "room") run = stageKey(e, key) || (player && (PLAYER_KEYS[(e.shiftKey ? "Shift+" : "") + k] || PLAYER_KEYS[k])) || null;
  if (!run) return;
  e.preventDefault();
  run(e);
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
setInterval(pollPlugins, JOBS_MS);
refresh();
setTimeout(() => checkUpdate(false), UPDATE_CHECK_DELAY_MS);

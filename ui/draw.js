const DRAW_START_PX = 4;
const DRAW_STEP_PX = 2;
const DRAW_SEND_MS = 50;
const LASER_MS = 1500;
const ARROW_HEAD = 0.035;
const ARROW_ANGLE = Math.PI / 7;
const COORD_STEP = 10000;

const ink = { tool: "pen", live: new Map(), cleared: null, press: null, sentAt: 0, sendTimer: null, frame: 0 };

function contentRect(tl) {
  const box = tl.pic.getBoundingClientRect();
  const v = tl.media.video;
  const ratio = v.videoWidth && v.videoHeight ? v.videoWidth / v.videoHeight : 1 / ASPECT;
  const w = Math.min(box.width, box.height * ratio);
  const h = w / ratio;
  return { x: (box.width - w) / 2, y: (box.height - h) / 2, w, h, left: box.left, top: box.top };
}

function inkLayer(tl) {
  if (!tl.ink) tl.ink = tl.pic.appendChild(svg("svg", { class: "ink", viewBox: "0 0 1 1", preserveAspectRatio: "none" }));
  const r = contentRect(tl);
  Object.assign(tl.ink.style, { left: r.x + "px", top: r.y + "px", width: r.w + "px", height: r.h + "px" });
  return r;
}

function strokeShapes(s, aspect) {
  const p = s.points;
  if (p.length < 2) return [];
  const attrs = { class: `stroke ${s.tool}`, stroke: colorOf(s.color) };
  if (s.tool !== "arrow") return [svg("polyline", { ...attrs, points: pairs(p) })];
  const [x0, y0, x1, y1] = [p[0], p[1], p[p.length - 2], p[p.length - 1]];
  const angle = Math.atan2(y1 - y0, (x1 - x0) * aspect);
  const head = (a) => [x1 - (Math.cos(a) * ARROW_HEAD) / aspect, y1 - Math.sin(a) * ARROW_HEAD];
  return [svg("polyline", { ...attrs, points: pairs([x0, y0, x1, y1]) }), svg("polyline", { ...attrs, points: pairs([...head(angle + ARROW_ANGLE), x1, y1, ...head(angle - ARROW_ANGLE)]) })];
}

function pairs(p) {
  const out = [];
  for (let i = 0; i + 1 < p.length; i += 2) out.push(`${p[i]},${p[i + 1]}`);
  return out.join(" ");
}

function storedStrokes(set, m) {
  if (!set || clock.playing) return [];
  if (ink.cleared && ink.cleared.set === set.id && Math.abs(ink.cleared.m - m) <= JUMP_MIN) return [];
  const sids = new Set();
  const out = [];
  for (const e of st.events) {
    if (e.kind !== "drawing" || e.set !== set.id || e.local_us === null || !e.values) continue;
    const at = (e.local_us - set.start_us) / 1e6;
    if (Math.abs(at - m) > JUMP_MIN) continue;
    sids.add(e.values.sid);
    out.push({ member: e.values.member, tool: e.values.tool, points: e.values.points || [], color: colorIndex(e.author) });
  }
  for (const [key, s] of ink.live) if (s.done && sids.has(s.id)) ink.live.delete(key);
  return out;
}

function colorIndex(id) {
  return (st.colors || {})[id] || 0;
}

function renderInk() {
  if (!st || page !== "player") return;
  const set = playSet();
  const now = performance.now();
  const shown = [...storedStrokes(set, nowM())];
  let fading = false;
  if (clock.playing) ink.cleared = null;
  const drawing = ink.press && ink.press.stroke;
  for (const [key, s] of ink.live) {
    if ((s.tool === "laser" && s !== drawing && now - s.at > LASER_MS) || (s.tool !== "laser" && clock.playing)) { ink.live.delete(key); continue; }
    if (s.tool === "laser") fading = true;
    shown.push(s);
  }
  for (const key of play.keys) {
    const tl = play.tiles[key];
    if (!tl) continue;
    const r = inkLayer(tl);
    const mine = shown.filter((s) => s.member === key);
    tl.ink.replaceChildren(...mine.flatMap((s) => {
      const shapes = strokeShapes(s, r.w / Math.max(1, r.h));
      if (s.tool === "laser" && s !== drawing) for (const shape of shapes) shape.style.opacity = Math.max(0, 1 - (now - s.at) / LASER_MS);
      return shapes;
    }));
  }
  cancelAnimationFrame(ink.frame);
  if (fading) ink.frame = requestAnimationFrame(renderInk);
}

function sendStroke(s) {
  const out = { id: s.id, set: selected, member: s.member, tool: s.tool, color: s.color, points: s.points, done: !!s.done };
  clearTimeout(ink.sendTimer);
  const wait = DRAW_SEND_MS - (performance.now() - ink.sentAt);
  const go = () => { ink.sentAt = performance.now(); invoke("draw", { stroke: out }).catch(warn); };
  if (s.done || wait <= 0) go();
  else ink.sendTimer = setTimeout(go, wait);
}

function clearInk(remote) {
  const set = playSet();
  ink.cleared = set ? { set: set.id, m: nowM() } : null;
  ink.live.clear();
  if (!remote) invoke("draw", { stroke: { clear: true, set: selected } }).catch(warn);
  renderInk();
}

function inkDown(tl, e) {
  if (!ink.tool || e.button !== 0 || e.target.closest(".nl")) return;
  ink.press = { tl, x: e.clientX, y: e.clientY, id: e.pointerId, stroke: null, last: null };
}

function inkMove(e) {
  const p = ink.press;
  if (!p || p.id !== e.pointerId) return;
  if (!(e.buttons & 1)) { ink.press = null; return; }
  if (!p.stroke) {
    if (Math.hypot(e.clientX - p.x, e.clientY - p.y) < DRAW_START_PX) return;
    if (ink.tool !== "laser" && clock.playing) { ink.press = null; return; }
    p.tl.box.setPointerCapture(e.pointerId);
    p.stroke = { id: Math.random().toString(36).slice(2), member: p.tl.key, tool: ink.tool, color: colorIndex(st.id), points: [], at: performance.now() };
    ink.live.set("me/" + p.stroke.id, p.stroke);
    addPoint(p, p.x, p.y);
  }
  addPoint(p, e.clientX, e.clientY);
}

function addPoint(p, x, y) {
  if (p.last && Math.hypot(x - p.last[0], y - p.last[1]) < DRAW_STEP_PX) return;
  p.last = [x, y];
  const r = contentRect(p.tl);
  const n = (v) => Math.round(Math.min(1, Math.max(0, v)) * COORD_STEP) / COORD_STEP;
  const point = [n((x - r.left - r.x) / r.w), n((y - r.top - r.y) / r.h)];
  p.stroke.points.push(...point);
  p.stroke.at = performance.now();
  sendStroke(p.stroke);
  renderInk();
}

function inkUp(e) {
  const p = ink.press;
  ink.press = null;
  if (!p || !p.stroke || p.id !== e.pointerId) return false;
  const s = p.stroke;
  s.done = true;
  s.at = performance.now();
  sendStroke(s);
  const set = playSet();
  if (s.tool !== "laser" && set && s.points.length >= 4) {
    run("add_event", { kind: "drawing", text: "", target: { t_us: wallOf(nowM()), set: set.id }, values: { member: s.member, tool: s.tool, points: s.points, sid: s.id } });
  }
  renderInk();
  return true;
}

listenMain("draw", (e) => {
  const { from, stroke } = e.payload;
  if (!watchParty || !stroke || stroke.set !== selected) return;
  if (stroke.clear) { clearInk(true); return; }
  ink.live.set(from + "/" + stroke.id, { ...stroke, color: colorIndex(from), at: performance.now() });
  renderInk();
});

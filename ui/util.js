const { invoke: tauriInvoke, convertFileSrc } = window.__TAURI__.core;
const invoke = (name, args) => tauriInvoke("call", { name, args: args || {} });
const dialog = (name, args) => tauriInvoke(name, args || {});
const LIMITS = { name: 24, room: 40, card: 24, note: 500, board: 4000, code: 1024, relay: 256 };
const UNWANTED = /[\u0000-\u001F\u007F-\u009F\u00AD\u061C\u115F\u1160\u200B\u200E\u200F\u202A-\u202E\u2060\u2066-\u2069\u3164\uFEFF\uFFA0]/g;
const STACKED = /[\u0300-\u036F\u1AB0-\u1AFF\u1DC0-\u1DFF\u20D0-\u20FF\uFE20-\uFE2F]{4,}/g;
const cleanText = (raw, max) => [...String(raw).replace(/[\n\r\t]/g, " ").replace(STACKED, (run) => run.slice(0, 3)).replace(UNWANTED, "").trim()].slice(0, max).join("").trim();
const single = (fn) => {
  let busy = false;
  return (...args) => (busy ? undefined : ((busy = true), Promise.resolve(fn(...args)).finally(() => { busy = false; })));
};
const $ = (id) => document.getElementById(id);
const boardWindow = location.search === "?board";
const listenMain = (name, fn) => { if (!boardWindow) window.__TAURI__.event.listen(name, fn); };
const setFull = (on) => invoke("set_fullscreen", { on, window: boardWindow ? "board" : "main" }).catch(warn);

function el(tag, props = {}, ...children) {
  const e = Object.assign(document.createElement(tag), props);
  e.append(...children.flat().filter((c) => c !== null && c !== undefined && c !== false));
  return e;
}
const withClass = (e, className) => { e.classList.add(className); return e; };
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
function bytes(n) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1000 && i < units.length - 1) { n /= 1000; i++; }
  return `${n.toFixed(i && n < 10 ? 1 : 0)} ${units[i]}`;
}
let clockShift = 0;
const nowUs = () => Date.now() * 1000 + clockShift;
const MEMBER_COLORS = ["orange", "sky", "lime", "magenta", "yellow", "violet", "pink", "blue"];
const colorOf = (index) => `var(--member-${MEMBER_COLORS[(index || 0) % MEMBER_COLORS.length]})`;
const sigs = {};
function changed(name, value, store = sigs) {
  const sig = JSON.stringify(value);
  if (store[name] === sig) return false;
  store[name] = sig;
  return true;
}
const rowHit = (target, selected, moved) => !moved && !selected && !target.closest("button, a, input, textarea");
function clickable(node, fn) {
  let from = null;
  node.tabIndex = 0;
  node.setAttribute("role", "button");
  node.classList.add("clickable");
  node.addEventListener("pointerdown", (e) => { from = [e.clientX, e.clientY]; });
  node.addEventListener("click", (e) => {
    const moved = from && Math.hypot(e.clientX - from[0], e.clientY - from[1]) > 4;
    from = null;
    if (rowHit(e.target, String(window.getSelection()).length > 0, moved)) fn();
  });
  node.addEventListener("keydown", (e) => {
    if (e.target === node && (e.key === "Enter" || e.key === " ")) { e.preventDefault(); fn(); }
  });
  return node;
}
function press(e, on) {
  e.classList.toggle("on", on);
  e.setAttribute("aria-pressed", on);
}
function pick(box, value, onPick) {
  for (const b of box.children) {
    press(b, b.dataset.v === value);
    b.onclick = () => onPick(b.dataset.v);
  }
}
const composing = (e) => e.isComposing || e.keyCode === 229;
function editKey(e, multiline) {
  if (composing(e)) return null;
  if (e.key === "Escape") return "cancel";
  if (e.key === "Enter" && (!multiline || e.ctrlKey)) return "save";
  return null;
}
function cancelButton(onCancel) {
  const b = el("button", { type: "button", className: "icon sm cancel", onmousedown: (e) => e.preventDefault(), onclick: onCancel }, icon("x", "sm"));
  b.dataset.tTitle = "Cancel (Esc)";
  b.dataset.tLabel = "Cancel (Esc)";
  b.title = t("Cancel (Esc)");
  b.setAttribute("aria-label", t("Cancel (Esc)"));
  return b;
}
function nativeKey(e) {
  const a = document.activeElement;
  if (!a) return false;
  if (a.type === "range" || a.getAttribute("role") === "separator") return /^(Arrow|Home$|End$|Page)/.test(e.key);
  return (e.key === " " || e.key === "Enter") && (a.tagName === "BUTTON" || a.type === "checkbox" || a.tagName === "SUMMARY");
}
function listMove(key, i, n, wrap) {
  if (!n) return null;
  if (key === "Home") return 0;
  if (key === "End") return n - 1;
  const d = key === "ArrowDown" ? 1 : key === "ArrowUp" ? -1 : 0;
  if (!d) return null;
  if (i < 0) return d > 0 ? 0 : n - 1;
  if (wrap) return (i + d + n) % n;
  return Math.max(0, Math.min(n - 1, i + d));
}
function arrowList(box, selector) {
  box.addEventListener("keydown", (e) => {
    if (e.altKey || e.ctrlKey || e.metaKey || e.shiftKey) return;
    const rows = [...box.querySelectorAll(selector)];
    const i = rows.indexOf(document.activeElement);
    const to = i < 0 ? null : listMove(e.key, i, rows.length, false);
    if (to === null) return;
    e.preventDefault();
    e.stopPropagation();
    rows[to].focus();
  });
}
function escapeStep(s) {
  if (s.menu || s.dialog) return "none";
  if (s.pop) return "pop";
  if (s.typing) return "blur";
  if (s.full) return "full";
  if (s.window) return "close";
  return s.panel ? "panel" : "none";
}
const UI_REPORT_MS = 1000;
const uiValue = (v) => (typeof v === "function" ? v() : v);
const uiCommands = (registry) => Object.fromEntries(Object.entries(registry).map(([cmd, c]) => [cmd, { about: c.about, args: Object.fromEntries(Object.entries(c.args || {}).map(([k, v]) => [k, uiValue(v)])) }]));
const uiState = (registry, readers) => Object.fromEntries(Object.entries(Object.assign({}, ...Object.values(registry).map((c) => c.reads || {}), readers)).map(([k, read]) => [k, read()]));
function uiRun(registry, cmd, args) {
  if (!Object.hasOwn(registry, cmd)) return false;
  registry[cmd].run(args || {});
  return true;
}
const typing = () => { const a = document.activeElement; return a && (a.tagName === "SELECT" || a.tagName === "TEXTAREA" || (a.tagName === "INPUT" && a.type !== "range" && a.type !== "checkbox") || a.isContentEditable); };
function stored(key, fallback) {
  try { const v = JSON.parse(localStorage.getItem(key)); return v === null ? fallback : v; } catch { return fallback; }
}
function store(key, value) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch (e) { console.warn(e); }
}
const longId = (id) => (String(id).match(/.{1,4}/g) || []).slice(0, 3).join("·");
const idText = (id, tag = "span") => el(tag, { className: "mono", textContent: longId(id), title: id });
const warn = (e) => console.warn(e);
const playFailed = (e) => { if (e.name !== "AbortError" && e.name !== "NotAllowedError") warn(e); };
const sessionDay = (id) => id.slice(0, 10);
const clockOf = (date) => `${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
function formatWhen(date, now = new Date(), lang = language) {
  const sameYear = date.getFullYear() === now.getFullYear();
  const day = lang === "ja"
    ? `${sameYear ? "" : `${date.getFullYear()}/`}${date.getMonth() + 1}/${date.getDate()}`
    : new Intl.DateTimeFormat("en-US", sameYear ? { month: "short", day: "numeric" } : { year: "numeric", month: "short", day: "numeric" }).format(date);
  return `${day} ${clockOf(date)}`;
}
const fullWhen = (date, lang = language) => new Intl.DateTimeFormat(lang === "ja" ? "ja-JP" : "en-US", { dateStyle: "full", timeStyle: "medium", hour12: false }).format(date);
const sessionStart = (id) => {
  const [y, m, d] = sessionDay(id).split("-").map(Number);
  const [h, mi, sec] = (id.split(" ")[1] || "").split("-").map(Number);
  return new Date(y, m - 1, d, h || 0, mi || 0, sec || 0);
};
const windowTitle = (app, room, recording) => (!room ? app : recording ? `${room} - ${recording}` : room);
let titles = {};
const sessionWhen = (id, now = new Date()) => formatWhen(sessionStart(id), now, language);
const sessionName = (id, now = new Date()) => titles[id] || sessionWhen(id, now);
const HOTKEY_LABELS = { mark: "Mark this moment" };
const hotkeyRows = (hotkeys) => hotkeys.map((h) => ({ ...h, label: HOTKEY_LABELS[h.action] || h.action, changed: h.keys !== h.default, warning: h.taken ? "In use by another app" : "" }));
function keysOf(e) {
  const named = { " ": "Space", ArrowLeft: "Left", ArrowRight: "Right", ArrowUp: "Up", ArrowDown: "Down" };
  const code = e.code || "";
  const key = /^Key[A-Z]$/.test(code) ? code.slice(3) : /^Digit[0-9]$/.test(code) ? code.slice(5) : /^F([1-9]|1[0-9]|2[0-4])$/.test(code) ? code : named[e.key] || null;
  if (!key || (!e.ctrlKey && !e.altKey)) return null;
  return [e.ctrlKey && "Ctrl", e.altKey && "Alt", e.shiftKey && "Shift", key].filter(Boolean).join("+");
}

function sessionNotice(action, by) {
  if (action === "start") return by ? t("{by} started recording", { by }) : t("Recording started");
  return by ? t("{by} stopped recording", { by }) : t("Recording stopped");
}

function raise(box) {
  if (box.matches(":popover-open")) box.hidePopover();
  if (!box.hidden) box.showPopover();
}
if (typeof MutationObserver !== "undefined") {
  const raiseObserver = new MutationObserver((list) => list.forEach((m) => raise(m.target)));
  document.querySelectorAll("[popover]").forEach((box) => raiseObserver.observe(box, { attributes: true, attributeFilter: ["hidden"] }));
  const showModalDialog = HTMLDialogElement.prototype.showModal;
  HTMLDialogElement.prototype.showModal = function () { showModalDialog.call(this); document.querySelectorAll("[popover]:not([hidden])").forEach(raise); };
}

const INK_TOOLS = ["pen", "laser", null];
const INK_TOOL_TITLE = {
  pen: "Pen (P): draw a line that stays at this moment; press again for Laser",
  laser: "Laser (P): a pointer that fades and works while playing; press again to stop drawing",
  off: "Drawing off (P): press to draw with the Pen",
};
const INK_TOOL_NAME = { pen: "Pen", laser: "Laser", off: "Drawing off" };
const nextInkTool = (tool) => INK_TOOLS[(INK_TOOLS.indexOf(tool) + 1) % INK_TOOLS.length];
const POP_GAP = 4;
const POP_EDGE = 8;
function popupSpot(at, w, h, vw, vh, above) {
  const room = { up: at.top - POP_GAP - POP_EDGE, down: vh - POP_EDGE - POP_GAP - at.bottom };
  const [want, other] = above ? ["up", "down"] : ["down", "up"];
  const up = (room[want] >= h || room[want] >= room[other]) === above;
  const fit = (v, size, max) => Math.max(POP_EDGE, Math.min(v, max - POP_EDGE - size));
  return { left: fit(at.left + w <= vw - POP_EDGE ? at.left : at.right - w, w, vw), top: fit(up ? at.top - POP_GAP - h : at.bottom + POP_GAP, h, vh) };
}
function openPopup(box, at, above) {
  box.anchor = { at, above };
  box.hidden = at instanceof Element && !at.getClientRects().length;
  placePopup(box);
}
function placePopup(box) {
  const { at, above } = box.anchor;
  if (box.hidden || (at instanceof Element && !at.getClientRects().length)) return;
  const p = popupSpot(at instanceof Element ? at.getBoundingClientRect() : at, box.offsetWidth, box.offsetHeight, innerWidth, innerHeight, above);
  box.style.left = p.left + "px";
  box.style.top = p.top + "px";
}
if (typeof addEventListener !== "undefined") {
  const placeOpen = () => document.querySelectorAll("[popover]:not([hidden])").forEach((box) => box.anchor && placePopup(box));
  addEventListener("resize", placeOpen);
  addEventListener("scroll", placeOpen, true);
}

const STROKE_PLAY_S = 2;
const STROKE_FADE_S = 0.5;
function strokeAlpha(at, m, playing, calm) {
  if (!playing) return Math.abs(at - m) <= JUMP_MIN ? 1 : 0;
  const age = m - at;
  if (age < 0 || age >= STROKE_PLAY_S) return 0;
  return calm ? 1 : Math.min(1, (STROKE_PLAY_S - age) / STROKE_FADE_S);
}

const { invoke, convertFileSrc } = window.__TAURI__.core;
const $ = (id) => document.getElementById(id);

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
function bytes(n) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1000 && i < units.length - 1) { n /= 1000; i++; }
  return `${n.toFixed(i && n < 10 ? 1 : 0)} ${units[i]}`;
}
const nowUs = () => Date.now() * 1000;
const setTime = (id) => { const [d, tm] = id.split(" "); return tm ? `${d.slice(5).replace("-", "/")} ${tm.replaceAll("-", ":").slice(0, 5)}` : id; };
const colorOf = (index) => { const palette = (st && st.palette) || []; return palette.length ? palette[(index || 0) % palette.length] : "var(--muted)"; };
const sigs = {};
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
const composing = (e) => e.isComposing || e.keyCode === 229;
function nativeKey(e) {
  const a = document.activeElement;
  if (!a) return false;
  if (a.type === "range" || a.getAttribute("role") === "separator") return /^(Arrow|Home$|End$|Page)/.test(e.key);
  return (e.key === " " || e.key === "Enter") && (a.tagName === "BUTTON" || a.type === "checkbox" || a.tagName === "SUMMARY");
}
const typing = () => { const a = document.activeElement; return a && (a.tagName === "SELECT" || a.tagName === "TEXTAREA" || (a.tagName === "INPUT" && a.type !== "range" && a.type !== "checkbox") || a.isContentEditable); };
function stored(key, fallback) {
  try { const v = JSON.parse(localStorage.getItem(key)); return v === null ? fallback : v; } catch { return fallback; }
}
function store(key, value) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch (e) { console.warn(e); }
}
function unstore(match) {
  try { for (const key of Object.keys(localStorage)) if (match(key)) localStorage.removeItem(key); } catch (e) { console.warn(e); }
}
const warn = (e) => console.warn(e);
const playFailed = (e) => { if (e.name !== "AbortError" && e.name !== "NotAllowedError") warn(e); };
const sessionDay = (id) => id.slice(0, 10);
const sessionClock = (id) => (id.split(" ")[1] || "").replaceAll("-", ":").slice(0, 5);
function dayOf(date) {
  const p = (n) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${p(date.getMonth() + 1)}-${p(date.getDate())}`;
}
function dayLabel(day, now, lang) {
  if (day === dayOf(now)) return t("Today");
  if (day === dayOf(new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1))) return t("Yesterday");
  const [y, m, d] = day.split("-").map(Number);
  const options = y === now.getFullYear() ? { month: "short", day: "numeric" } : { year: "numeric", month: "short", day: "numeric" };
  return new Intl.DateTimeFormat(lang === "ja" ? "ja-JP" : "en-US", options).format(new Date(y, m - 1, d));
}
let titles = {};
const sessionName = (id, now = new Date()) => titles[id] || `${dayLabel(sessionDay(id), now, language)} ${sessionClock(id)}`;
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

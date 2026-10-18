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
  if (a.type === "range") return /^(Arrow|Home$|End$|Page)/.test(e.key);
  return (e.key === " " || e.key === "Enter") && (a.tagName === "BUTTON" || a.type === "checkbox" || a.tagName === "SUMMARY");
}
const typing = () => { const a = document.activeElement; return a && (a.tagName === "SELECT" || a.tagName === "TEXTAREA" || (a.tagName === "INPUT" && a.type !== "range" && a.type !== "checkbox") || a.isContentEditable); };
function stored(key, fallback) {
  try { const v = JSON.parse(localStorage.getItem(key)); return v === null ? fallback : v; } catch { return fallback; }
}
function store(key, value) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch {}
}

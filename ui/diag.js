const CLOCK_WARN_US = 3000000;
const CLOCK_SAME_US = 50000;
const CLOCK_SYNC_OLD_S = 14 * 86400;
const DROP_WARN = 30;
const MB_PER_GB = 1024;
const LEVEL_ICONS = { ok: "ok", warn: "alert", fail: "fail" };

function agoText(seconds) {
  if (seconds < 60) return t("Just now");
  if (seconds < 3600) return t("{n} min ago", { n: Math.round(seconds / 60) });
  if (seconds < 86400) return t("{n} h ago", { n: Math.round(seconds / 3600) });
  return t("{n} days ago", { n: Math.round(seconds / 86400) });
}
function spanText(us) {
  const a = Math.abs(us);
  return a >= 1000000 ? `${(a / 1000000).toFixed(1)} s` : `${Math.round(a / 1000)} ms`;
}
function skewText(us) {
  if (Math.abs(us) < CLOCK_SAME_US) return t("In sync");
  return t(us > 0 ? "{x} ahead" : "{x} behind", { x: spanText(us) });
}
function timeRow(time, nowS) {
  if (!time) return null;
  const key = "Windows time";
  if (time.running === false) return { key, level: "warn", text: t("Sync is off") };
  if (time.running !== true) return { key, level: "info", text: t("Unknown") };
  if (time.last_sync === null || time.last_sync === undefined) return { key, level: "ok", text: t("Sync is on") };
  const age = Math.max(0, nowS - time.last_sync);
  return { key, level: age > CLOCK_SYNC_OLD_S ? "warn" : "ok", text: t("Sync is on · last sync {ago}", { ago: agoText(age) }) };
}
const measured = (p) => p.connected && p.offset_us !== null && p.offset_us !== undefined;
function clockRow(p) {
  if (!measured(p)) return null;
  return { key: "Clock", level: Math.abs(p.offset_us) > CLOCK_WARN_US ? "warn" : "ok", text: skewText(p.offset_us) };
}
function hardwareRows(hw) {
  if (!hw) return [{ key: "Hardware", level: "info", text: t("Not shared") }];
  const rows = [];
  const add = (key, text, level = "info") => { if (text) rows.push({ key, level, text }); };
  add("Graphics", [hw.gpu, hw.driver].filter(Boolean).join(" · "));
  add("Encoder", hw.encoder ? [hw.encoder === "Software" ? t("Software") : hw.encoder, String(hw.codec || "").toUpperCase()].filter(Boolean).join(" · ") : "");
  add("CPU", hw.cpu);
  add("Memory", hw.ram_mb ? `${Math.round(hw.ram_mb / MB_PER_GB)} GB` : "");
  add("Display", hw.display && hw.display[0] ? `${hw.display[0]} × ${hw.display[1]}` : "");
  add("Windows version", hw.os);
  if (typeof hw.fps === "number") add("Capture", t("{fps} fps · {n} dropped", { fps: hw.fps.toFixed(1), n: hw.dropped || 0 }), (hw.dropped || 0) >= DROP_WARN ? "warn" : "ok");
  return rows;
}
function pcCards(s, nowS) {
  const mine = { id: s.id, me: true, rows: [...hardwareRows(s.hw), timeRow(s.time_sync, nowS)].filter(Boolean) };
  return [mine, ...s.peers.map((p) => ({ id: p.id, me: false, dim: !p.connected, rows: [...hardwareRows(p.hw), clockRow(p)].filter(Boolean) }))];
}
function clockPairs(peers) {
  const known = peers.filter(measured);
  const pairs = [];
  for (let i = 0; i < known.length; i++) for (let j = i + 1; j < known.length; j++) pairs.push({ a: known[i].id, b: known[j].id, us: known[j].offset_us - known[i].offset_us });
  return pairs;
}
function clockSummary(s, nowS = Date.now() / 1000) {
  const time = timeRow(s.time_sync, nowS);
  if (time && time.level === "warn") return { level: "warn", text: `${t("Windows time")}: ${time.text}` };
  if (!s.peers.some((p) => p.connected)) return { level: "ok", text: t("No one else is online") };
  const offsets = [0, ...s.peers.filter(measured).map((p) => p.offset_us)];
  if (offsets.length === 1) return { level: "info", text: t("Measuring…") };
  const range = Math.max(...offsets) - Math.min(...offsets);
  return { level: range > CLOCK_WARN_US ? "warn" : "ok", text: t("Largest difference {x}", { x: spanText(range) }) };
}
function flowRows(s, flow, label) {
  const on = s.peers.filter((p) => p.connected);
  const relayed = viaRelay(on);
  const who = (p) => s.names[p.id] || nameOf(p.id);
  const route = relayed.length
    ? { key: "Members", level: "warn", text: t("Through a relay server: {names}", { names: relayed.map(who).join(", ") }) }
    : { key: "Members", level: on.length ? "ok" : "info", text: on.length ? t("All direct") : t("No one else is online") };
  const rows = [route];
  if (!flow) return rows;
  const relay = [...new Set([flow.relay, ...relayed.map((p) => p.relay)].filter(Boolean))].join(", ");
  rows.push({ key: "Address lookup", level: "info", text: [relay, t("Publishes only the relay address")].filter(Boolean).join(" · ") });
  rows.push({ key: "Update check", level: "info", text: t("github.com · at start and every 4 h · {channel} channel", { channel: flow.update.channel === "beta" ? t("Beta") : t("Stable") }) });
  const downloads = flow.downloads.map((d) => t("Downloading {what} from {host}", { what: label(d.key), host: d.hosts.join(", ") }));
  rows.push(downloads.length ? { key: "Downloads", level: "warn", text: downloads.join(" · ") } : { key: "Downloads", level: "info", text: t("Video tools, plugin tools and models, only when needed") });
  if (flow.api.on) rows.push({ key: "Local API", level: flow.api.lan ? "warn" : "info", text: flow.api.lan ? t("Other devices on your network can control the app") : t("This PC only") });
  return rows;
}
function okLine(row) {
  return el("div", { className: "ok-line lv-" + row.level }, icon(LEVEL_ICONS[row.level] || "info", "sm"), el("b", { textContent: t(row.key) }), el("span", { textContent: row.text }));
}
function pcsView(s) {
  const nowS = Date.now() / 1000;
  const cards = pcCards(s, nowS).map((c) => el("div", { className: "okbox" + (c.dim ? " dim" : "") }, el("div", { className: "h3 first", textContent: c.me ? `${nameOf(c.id)} (${t("This PC")})` : nameOf(c.id) }), ...c.rows.map(okLine)));
  const pairs = clockPairs(s.peers);
  if (pairs.length) cards.push(el("div", { className: "okbox" }, el("div", { className: "h3 first", textContent: t("Clocks between members") }), ...pairs.map((p) => okLine({ key: `${nameOf(p.a)} / ${nameOf(p.b)}`, level: Math.abs(p.us) > CLOCK_WARN_US ? "warn" : "ok", text: Math.abs(p.us) < CLOCK_SAME_US ? t("In sync") : spanText(p.us) }))));
  return cards;
}

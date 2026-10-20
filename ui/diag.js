const CLOCK_WARN_US = 3000000;
const CLOCK_SAME_US = 50000;
const DROP_WARN = 30;
const MB_PER_GB = 1024;
const LEVEL_ICONS = { ok: "ok", warn: "alert", fail: "fail" };

function spanText(us) {
  const a = Math.abs(us);
  return a >= 1000000 ? `${(a / 1000000).toFixed(1)} s` : `${Math.round(a / 1000)} ms`;
}
function skewText(us) {
  if (Math.abs(us) < CLOCK_SAME_US) return t("In sync");
  return t(us > 0 ? "{x} ahead" : "{x} behind", { x: spanText(us) });
}
function ownClockRow(clock) {
  const offset = clock && clock.offset_us;
  if (typeof offset !== "number") return { key: "Clock", level: "warn", text: t("Unreachable") };
  return { key: "Clock", level: Math.abs(offset) > CLOCK_WARN_US ? "warn" : "ok", text: skewText(-offset) };
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
function pcCards(s) {
  const mine = { id: s.id, me: true, rows: [...hardwareRows(s.hw), ownClockRow(s.clock)] };
  return [mine, ...s.peers.map((p) => ({ id: p.id, me: false, dim: !p.connected, rows: [...hardwareRows(p.hw), clockRow(p)].filter(Boolean) }))];
}
function clockSummary(s) {
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
  rows.push(flow.time.ok ? { key: "Time check", level: "info", text: t("{host} · at start and every hour", { host: flow.time.host }) } : { key: "Time check", level: "warn", text: `${flow.time.host} · ${t("Unreachable")}` });
  const downloads = flow.downloads.map((d) => t("Downloading {what} from {host}", { what: label(d.key), host: d.hosts.join(", ") }));
  rows.push(downloads.length ? { key: "Downloads", level: "warn", text: downloads.join(" · ") } : { key: "Downloads", level: "info", text: t("Video tools, plugin tools and models, only when needed") });
  if (flow.api.on) rows.push({ key: "Local API", level: flow.api.lan ? "warn" : "info", text: flow.api.lan ? t("Other devices on your network can control the app") : t("This PC only") });
  return rows;
}
function okLine(row) {
  return el("div", { className: "ok-line lv-" + row.level }, icon(LEVEL_ICONS[row.level] || "info", "sm"), el("b", { textContent: t(row.key) }), el("span", { textContent: row.text }));
}
function pcsView(s) {
  return pcCards(s).map((c) => el("div", { className: "okbox" + (c.dim ? " dim" : "") }, el("div", { className: "h3 first", textContent: c.me ? `${nameOf(c.id)} (${t("This PC")})` : nameOf(c.id) }), ...c.rows.map(okLine)));
}

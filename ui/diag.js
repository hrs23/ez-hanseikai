const LEVEL_ICONS = { ok: "ok", warn: "alert", fail: "fail" };

function flowRows(s, flow) {
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
  rows.push({ key: "Update check", level: "info", text: t("github.com · at start and every 4 h") });
  rows.push(flow.time.ok ? { key: "Time check", level: "info", text: t("{host} · at start and every hour", { host: flow.time.host }) } : { key: "Time check", level: "warn", text: `${flow.time.host} · ${t("Unreachable")}` });
  const downloads = flow.downloads.map((d) => t("Downloading {what} from {host}", { what: t("Video tools"), host: d.hosts.join(", ") }));
  rows.push(downloads.length ? { key: "Downloads", level: "warn", text: downloads.join(" · ") } : { key: "Downloads", level: "info", text: t("Video tools, only when needed") });
  if (flow.api.on) rows.push({ key: "Local API", level: "info", text: t("This PC only") });
  return rows;
}
function okLine(row) {
  return el("div", { className: "ok-line lv-" + row.level }, icon(LEVEL_ICONS[row.level] || "info", "sm"), el("b", { textContent: t(row.key) }), el("span", { textContent: row.text }));
}
const viaRelay = (peers) => peers.filter((p) => p.connected && p.path === "relay");
function netState(peers) {
  if (!peers.some((p) => p.connected)) return null;
  return viaRelay(peers).length ? "relay" : "direct";
}

function leftText(seconds) {
  if (seconds < 60) return t("{n} s left", { n: Math.max(1, Math.round(seconds)) });
  if (seconds < 3600) return t("{n} min left", { n: Math.round(seconds / 60) });
  return t("{n} h left", { n: Math.round(seconds / 360) / 10 });
}

function progressText({ n, speed, left }) {
  return [`${n}%`, speed ? `${bytes(speed)}/s` : null, left != null ? leftText(left) : null].filter(Boolean).join(" · ");
}

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
  rows.push({ key: "Relay server", level: "info", text: [t(s.relays && s.relays.length ? "Custom" : "Default"), relay, t("Sees only encrypted traffic")].filter(Boolean).join(" · ") });
  rows.push({ key: "Update check", level: "info", text: t("github.com · at start and every 4 h") });
  rows.push({ key: "Feedback", level: "info", text: flow.report.host ? t("{host} · only when you send it", { host: flow.report.host }) : t("Not in this build") });
  rows.push(flow.time.ok ? { key: "Time check", level: "info", text: t("{host} · at start and every hour", { host: flow.time.host }) } : { key: "Time check", level: "warn", text: `${flow.time.host} · ${t("Unreachable")}` });
  rows.push(flow.download ? { key: "Downloads", level: "warn", text: t("Downloading {what} from {host}", { what: t("Video tools"), host: flow.download }) } : { key: "Downloads", level: "info", text: t("Video tools, only when needed") });
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

const FEEDBACK_NAMES = { bug: "Problem", idea: "Request", comment: "Comment" };
function feedbackPreview(kind, comment, diagnostics, logBytes = 0) {
  const note = comment.trim() ? comment.slice(0, LIMITS.feedback) : t("(no comment)");
  const log = diagnostics != null && logBytes > 0 ? `+ ${t("The log, masked the same way and compressed ({size})", { size: bytes(logBytes) })}` : null;
  return [`${t("Kind")}: ${t(Object.hasOwn(FEEDBACK_NAMES, kind) ? FEEDBACK_NAMES[kind] : "Problem")}`, note, log, diagnostics == null ? null : `---\n${diagnostics}`].filter((x) => x != null).join("\n\n");
}
function feedbackReady(comment, withDiagnostics) {
  return withDiagnostics || comment.trim() !== "";
}
function errorReport(e) {
  return `${t("Error")}: ${t(String((e && e.message) || e))}`;
}
function shownError(last, visible) {
  return last && visible ? `${t("Error")}: ${last.text}` : "";
}

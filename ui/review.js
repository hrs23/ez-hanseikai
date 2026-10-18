const LIVE_EDGE_S = 3;
const RATE_WINDOW_MS = 10000;
const RATE_MIN_MS = 1500;
const SEEK_SHOWN_S = 2;
const KEEP_NEWEST = 3;

const shortId = (id) => `${String(id).slice(0, 4)}·${String(id).slice(4, 8)}`;
const voiceOnly = (set) => !!set && set.players.length > 0 && set.players.every((p) => (p.parts || []).every((part) => !part.v));
const isReview = (set) => voiceOnly(set) && (set.party || []).length > 0;

function positionAt(shared, at, t) {
  const from = typeof shared.at_us === "number" ? shared.at_us : at;
  return shared.wall_us + (shared.playing && t > from ? (t - from) * (shared.rate || 1) : 0);
}

function flowAt(records, t) {
  if (!records || !records.length) return null;
  let r = records[0];
  for (const x of records) {
    if (x.t_us > t) break;
    r = x;
  }
  const s = r.shared;
  const playing = !!s.playing && t >= r.t_us;
  return { set: s.set || null, playing, wall_us: typeof s.wall_us === "number" ? positionAt(s, r.t_us, t) : null, rate: s.rate || 1, focus: s.focus || null, mode: s.mode || "grid", slots: s.slots || 0, order: s.order || [], by: r.by, at: r.t_us };
}

function flowBands(records, start, end) {
  const out = [];
  (records || []).forEach((r, i) => {
    const set = r.shared.set;
    const from = Math.max(start, i ? r.t_us : start);
    const to = Math.min(end, i + 1 < records.length ? records[i + 1].t_us : end);
    if (!set || to <= from) return;
    const last = out[out.length - 1];
    if (last && last.set === set && last.to === from) last.to = to;
    else out.push({ set, from, to });
  });
  return out;
}

function flowSteps(records) {
  const out = [];
  let prev = null;
  for (const r of records || []) {
    const s = r.shared;
    const step = { t_us: r.t_us, set: s.set || null, wall_us: s.wall_us, by: r.by };
    if (!prev || s.set !== prev.shared.set) out.push({ ...step, kind: s.set ? "open" : "live" });
    else if (!!s.playing !== !!prev.shared.playing) out.push({ ...step, kind: s.playing ? "play" : "pause" });
    else if (Math.abs(positionAt(prev.shared, prev.t_us, r.t_us) - s.wall_us) > SEEK_SHOWN_S * 1e6) out.push({ ...step, kind: "seek" });
    prev = r;
  }
  return out;
}

function liveEdge(total, m) {
  const behind = Math.max(0, total - m);
  return { behind, at: behind <= LIVE_EDGE_S };
}

function rateOf(samples, now) {
  const recent = samples.filter((s) => now - s.t <= RATE_WINDOW_MS);
  if (recent.length < 2) return null;
  const first = recent[0], last = recent[recent.length - 1];
  if (last.t - first.t < RATE_MIN_MS) return null;
  return Math.max(0, ((last.have - first.have) * 1000) / (last.t - first.t));
}

function deletePreview(sets, me) {
  return sets.map((s) => ({
    id: s.id,
    bytes: s.progress.have,
    players: s.players.filter((p) => p.have > 0).map((p) => ({ owner: p.owner, bytes: p.have, alone: !(p.verified_by || []).some((id) => id !== me) })),
  }));
}

function oldSessions(sets, keep = KEEP_NEWEST) {
  return sets.filter((s) => !s.live && s.progress.have > 0).slice(keep).map((s) => s.id);
}

function netState(peers) {
  const on = peers.filter((p) => p.connected);
  if (!on.length) return null;
  return on.some((p) => p.path === "relay") ? "relay" : "direct";
}

function leftText(seconds) {
  if (seconds < 60) return t("{n} s left", { n: Math.max(1, Math.round(seconds)) });
  if (seconds < 3600) return t("{n} min left", { n: Math.round(seconds / 60) });
  return t("{n} h left", { n: Math.round(seconds / 360) / 10 });
}

function summaryLines(s, failed) {
  const lines = [];
  const on = s.peers.filter((p) => p.connected);
  const relay = on.filter((p) => p.path === "relay");
  const who = (p) => s.names[p.id] || shortId(p.id);
  lines.push({ key: "Connections", level: relay.length ? "warn" : "ok", text: on.length ? on.map((p) => `${who(p)} ${p.path === "relay" ? t("relay") : t("direct")} ${p.rtt_ms} ms`).join(" · ") : t("No one else is online") });
  const c = s.capture;
  const q = c.quality;
  const captureText = !c.source ? t("Screen off") : c.source_lost ? t("Capture source closed") : c.status ? t("Capture stopped. Retrying…") : [c.source.kind === "window" ? c.source.title : t("Display {n}", { n: c.source.index }), q.width ? `${q.width}×${q.height}` : "", `${q.fps} fps`, String(q.codec).toUpperCase()].filter(Boolean).join(" · ");
  lines.push({ key: "Capture", level: c.source && (c.source_lost || c.status) ? "warn" : c.source ? "ok" : "info", text: captureText });
  lines.push({ key: "Microphone", level: c.mix.mic.muted ? "info" : "ok", text: c.mix.mic.muted ? t("Muted") : t("On") });
  const receiving = s.sets.filter((x) => x.progress.total && x.progress.have < x.progress.total);
  const stuck = receiving.filter((x) => x.progress.stuck);
  lines.push({ key: "Transfers", level: stuck.length ? "warn" : "ok", text: receiving.length ? t("Receiving {n} sessions", { n: receiving.length }) : t("Everything is here") });
  const d = s.disk;
  const space = d.hours_left !== null && d.hours_left !== undefined ? t("{free} free · about {h} h left", { free: bytes(d.free), h: Math.floor(d.hours_left) }) : t("{free} free", { free: bytes(d.free) });
  lines.push({ key: "Storage", level: d.full ? "fail" : d.low ? "warn" : "ok", text: space });
  const other = on.filter((p) => p.version && p.version !== s.version);
  lines.push({ key: "Versions", level: other.length ? "warn" : "ok", text: other.length ? other.map((p) => t("{name} is on {version}", { name: who(p), version: p.version })).join(" · ") : s.version });
  lines.push({ key: "Plugins", level: failed ? "fail" : "ok", text: failed ? t(failed === 1 ? "{n} job failed" : "{n} jobs failed", { n: failed }) : t("No problems") });
  return lines;
}

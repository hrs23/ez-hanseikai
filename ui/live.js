const LIVE_EDGE_S = 3;
const RATE_WINDOW_MS = 10000;
const RATE_MIN_MS = 1500;
const JUMP_MIN = 1;

function ordered(present, saved) {
  const keep = (saved || []).filter((k) => present.includes(k));
  return [...keep, ...present.filter((k) => !keep.includes(k))];
}

function livePlayers(s) {
  const live = s.sets.find((x) => x.live) || null;
  const peers = new Map(s.peers.map((p) => [p.id, p]));
  return ordered([s.id, ...s.peers.map((p) => p.id)], s.layout && s.layout.order).map((key) => {
    const me = key === s.id;
    const peer = peers.get(key);
    const file = me ? s.capture.live : peer && peer.live;
    const relayed = !me && !!peer && peer.connected && !sendsPicture(peer);
    const mine = live ? live.players.find((p) => p.owner === key) : null;
    const online = me || !!(peer && peer.connected);
    const sharing = me ? !!s.capture.source : !!(peer && peer.sharing);
    return { key, me, online, relayed, picture: online && sharing, file: file && file.size > 0 && !relayed ? file : null, sharing, recording: !!(mine && mine.recording) && (!me || s.capture.working), last: mine ? mine.last_us : null, start: mine ? mine.start_us - (mine.offset_us || 0) : null };
  });
}

function markSpeak(nodes, speakingIds, selfId, selfMuted) {
  for (const e of nodes) {
    const off = e.dataset.speak === selfId && selfMuted;
    e.classList.toggle("speaking", speakingIds.has(e.dataset.speak) && !off);
    e.classList.toggle("micoff", off);
  }
}

const LIVE_LAG_MAX_S = 12;
const LIVE_HEADROOM_S = 2;
const LIVE_SHRINK_GAP_S = 3;
const LIVE_CATCH_UP = 1.05;

function liveAvail(slots) {
  const ends = slots.filter(([base, v]) => base !== null && bufferedEnd(v) > 0).map(([base, v]) => base + bufferedEnd(v));
  return ends.length ? Math.min(...ends) : null;
}

function liveFollow(end, at) {
  const lag = end - at;
  return { target: lag > 2 * LIVE_HEADROOM_S + LIVE_SHRINK_GAP_S ? end - LIVE_HEADROOM_S : at, playing: lag > 0 };
}

function liveStep(state, members, now, dt) {
  const loaded = members.filter((m) => m.avail !== null && !m.ended);
  const holding = loaded.filter((m) => m.avail > now - LIVE_LAG_MAX_S);
  const limit = holding.length ? Math.min(...holding.map((m) => m.avail)) - LIVE_HEADROOM_S : null;
  const rate = limit !== null && state.pos !== null && limit - state.pos > LIVE_SHRINK_GAP_S ? LIVE_CATCH_UP : 1;
  const next = state.pos === null ? (limit === null ? null : limit - LIVE_SHRINK_GAP_S) : state.pos + dt * rate;
  const held = limit !== null && next > limit;
  const pos = held ? Math.max(state.pos, limit) : next;
  return { pos, rate, held, late: loaded.filter((m) => !holding.includes(m) && m.avail < pos).map((m) => m.key) };
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

const sameParty = (a, b) => !!b && a.seq === b.seq && a.stamp === b.stamp && a.origin === b.origin;

function partyMove(p, here) {
  const m = Math.max(0, (p.wall_us - here.base) / 1e6 + (p.playing ? ((here.now - p.at_us) / 1e6) * here.rate : 0));
  if (p.playing !== here.playing) return { cmd: p.playing ? "play" : "pause", m, quiet: here.quiet };
  if (Math.abs(m - here.m) > JUMP_MIN) return { cmd: "seek", m, quiet: here.quiet };
  return here.quiet ? null : { m, quiet: true };
}

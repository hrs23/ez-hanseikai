const LIVE_EDGE_S = 3;
const RATE_WINDOW_MS = 10000;
const RATE_MIN_MS = 1500;

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
    const mine = live ? live.players.find((p) => p.owner === key) : null;
    return { key, me, online: me || !!(peer && peer.connected), file: file && file.size > 0 ? file : null, sharing: me ? !!s.capture.source : !!(peer && peer.sharing), recording: !!(mine && mine.recording) && (!me || s.capture.working), last: mine ? mine.last_us : null, start: mine ? mine.start_us - (mine.offset_us || 0) : null };
  });
}

const LIVE_LAG_MAX_S = 12;
const LIVE_HEADROOM_S = 2;
const LIVE_SHRINK_GAP_S = 3;
const LIVE_CATCH_UP = 1.05;

function liveAvail(slots) {
  const ends = slots.filter(([start, v]) => start !== null && v && v.readyState > 0 && isFinite(v.duration)).map(([start, v]) => start + v.duration);
  return ends.length ? Math.min(...ends) : null;
}

function liveFollow(duration, at, loadSec) {
  const keep = LIVE_HEADROOM_S + 2 * loadSec;
  const lag = duration - at;
  return { target: lag > keep + LIVE_HEADROOM_S + LIVE_SHRINK_GAP_S ? duration - keep : at, playing: lag > 0 };
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

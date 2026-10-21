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

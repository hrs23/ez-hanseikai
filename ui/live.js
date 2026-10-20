const LIVE_LAG_MAX_S = 12;
const LIVE_HEADROOM_S = 2;
const LIVE_SHRINK_GAP_S = 3;
const LIVE_CATCH_UP = 1.05;

function liveAvail(slots) {
  const ends = slots.filter(([start, v]) => start !== null && v && v.readyState > 0 && isFinite(v.duration)).map(([start, v]) => start + v.duration);
  return ends.length ? Math.min(...ends) : null;
}

function liveFollow(duration, at) {
  const lag = duration - at;
  return { target: lag > LIVE_HEADROOM_S + LIVE_SHRINK_GAP_S ? duration - LIVE_HEADROOM_S : at, playing: lag > 0 };
}

function liveStep(state, members, now, dt) {
  const loaded = members.filter((m) => m.avail !== null && !m.ended);
  const holding = loaded.filter((m) => m.avail > now - LIVE_LAG_MAX_S);
  const late = loaded.filter((m) => !holding.includes(m)).map((m) => m.key);
  if (!holding.length) return { pos: state.pos, rate: 1, held: false, late };
  const limit = Math.min(...holding.map((m) => m.avail)) - LIVE_HEADROOM_S;
  if (state.pos === null) return { pos: limit - LIVE_SHRINK_GAP_S, rate: 1, held: false, late };
  const rate = limit - state.pos > LIVE_SHRINK_GAP_S ? LIVE_CATCH_UP : 1;
  const next = state.pos + dt * rate;
  return { pos: Math.max(state.pos, Math.min(next, limit)), rate, held: next > limit, late };
}

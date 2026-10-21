const SIDE_MIN = 220;
const SIDE_MAX = 420;
const SIDE_DEFAULT = 264;
const SIDE_KEY_STEP = 16;
const SIDE_FACES = 4;

function clampSide(px) {
  const n = Number(px);
  return Number.isFinite(n) ? Math.max(SIDE_MIN, Math.min(SIDE_MAX, Math.round(n))) : SIDE_DEFAULT;
}

const METER_MIN_DB = -60;
const METER_TICKS = [-60, -40, -20, -10, 0];
const METER_YELLOW_DB = -20;
const METER_RED_DB = -9;

function meterPos(db) {
  return Math.max(0, Math.min(1, (db - METER_MIN_DB) / -METER_MIN_DB));
}

const toDb = (v) => (v > 0 ? 20 * Math.log10(v) : -Infinity);

function micLevel(v, muted) {
  return muted ? 0 : Math.round(meterPos(toDb(v)) * 100) / 100;
}

function captureStates(c, problem) {
  return { screen: !c.source ? "off" : problem ? "warn" : "on", mic: c.mix.mic.muted ? "off" : "on" };
}

const LIVE_LAG = 1.5;
const LIVE_SLACK = 3 * LIVE_LAG;
const RELOAD_AHEAD = 1;
const RELOAD_MIN_MS = 1000;
const RELOAD_GROWTH = 300000;
const RETRY_MS = 3000;
const FRAME_WAIT_MS = 1000;
const NEXT_TIMEOUT_MS = 10000;
const SEEK_TOLERANCE = 0.05;
const DRIFT_MAX = 0.4;

const slot = (video) => ({ video, src: null, size: 0, seen: 0, part: null, loadSec: 0, loadedAt: 0, failedAt: 0, complete: false, pending: null });
const srcOf = (file) => convertFileSrc(file.path, "ezh");
const pathOf = (part) => (part ? part.path : null);

function release(v) {
  if (!v) return;
  v.pause();
  v.removeAttribute("src");
  v.load();
  v.remove();
}
function dropPending(s) {
  release(s.pending && s.pending.video);
  s.pending = null;
}
function drop(s) {
  if (!s) return;
  dropPending(s);
  release(s.video);
}

function load(s, src, size, part, at) {
  const old = s.video;
  const v = el(old.tagName.toLowerCase(), { muted: old.muted, volume: old.volume, preload: "auto" });
  v.playbackRate = old.playbackRate;
  const next = { video: v, src, size, part };
  dropPending(s);
  s.pending = next;
  s.loadedAt = Date.now();
  v.onloadedmetadata = () => { const want = Math.max(0, at ?? old.currentTime); v.currentTime = Math.min(want, v.duration || want); };
  v.onseeked = () => {
    if (s.pending !== next) return;
    v.onseeked = null;
    if (!old.paused) v.play().catch(playFailed);
    const done = () => {
      if (s.pending !== next) return;
      s.pending = null;
      s.loadSec = (Date.now() - s.loadedAt) / 1000;
      Object.assign(s, { video: v, src, size, part });
      release(old);
    };
    if (v.requestVideoFrameCallback) v.requestVideoFrameCallback(done);
    else done();
    setTimeout(done, FRAME_WAIT_MS);
  };
  v.onerror = () => {
    if (s.pending !== next) return;
    dropPending(s);
    s.failedAt = Date.now();
  };
  v.src = src + "?r=" + Date.now();
  old.before(v);
}

function feed(s, file, following, part = null, at = 0) {
  const src = srcOf(file);
  if (s.pending) {
    if (Date.now() - s.loadedAt < NEXT_TIMEOUT_MS) return;
    dropPending(s);
  }
  if (Date.now() - s.failedAt < RETRY_MS) return;
  const v = s.video;
  if (s.src !== src) {
    const playing = following && s.src && !v.paused && !v.error;
    if (playing && !s.complete) { s.complete = true; load(s, s.src, s.size, s.part); return; }
    if (playing && !v.ended && isFinite(v.duration) && v.duration - v.currentTime > s.loadSec) return;
    s.complete = false;
    if (s.src) { load(s, src, file.size, part, at); return; }
    Object.assign(s, { src, size: file.size, part, loadedAt: Date.now() });
    v.src = src;
    return;
  }
  const settled = s.seen === file.size;
  s.seen = file.size;
  const grew = file.size > s.size;
  const left = isFinite(v.duration) ? v.duration - v.currentTime : Infinity;
  const since = Date.now() - s.loadedAt;
  const stuck = v.error && since > RETRY_MS;
  const due = grew && since > RELOAD_MIN_MS && (following ? left < s.loadSec + RELOAD_AHEAD || v.ended : v.paused && (settled || file.size > s.size + RELOAD_GROWTH));
  if (stuck || due) load(s, src, file.size, part);
}

function follow(s) {
  const v = s && s.video;
  if (!v || v.readyState < 1 || !isFinite(v.duration)) return;
  const lag = LIVE_LAG + 2 * s.loadSec;
  if (v.duration - v.currentTime > lag + LIVE_SLACK) v.currentTime = Math.max(0, v.duration - lag);
  if (v.paused && v.currentTime < v.duration - SEEK_TOLERANCE) v.play().catch(playFailed);
}

function sync(v, target, playing) {
  const end = isFinite(v.duration) ? v.duration : Infinity;
  const want = Math.max(0, Math.min(target, end));
  if (target < 0 || target > end || !playing) {
    if (!v.paused) v.pause();
    if (v.readyState > 0 && !v.seeking && Math.abs(v.currentTime - want) > SEEK_TOLERANCE) v.currentTime = want;
  } else if (v.readyState === 0 || v.seeking) {
    return;
  } else if (v.paused) {
    if (Math.abs(v.currentTime - target) > SEEK_TOLERANCE) v.currentTime = target;
    v.play().catch(playFailed);
  } else if (Math.abs(v.currentTime - target) > DRIFT_MAX) {
    v.currentTime = target;
  }
}

function partFor(p, series, wall) {
  let pick = null;
  for (const part of p.file.parts) if (part[series] && (!pick || part[series].start_us - p.off <= wall)) pick = part[series];
  return pick;
}
const shiftOf = (p, part, b) => (b + p.off - part.start_us) / 1e6;
const switching = (s) => !!s.pending && pathOf(s.pending.part) !== pathOf(s.part);

function feedParts(tl, p, m, b, playing) {
  const wall = b + m * 1e6;
  const mic = partFor(p, "m", wall);
  if (mic && !tl.mic) tl.mic = slot(tl.box.appendChild(el("audio", { preload: "auto" })));
  for (const [s, part] of [[tl.media, partFor(p, "v", wall)], [tl.mic, mic]]) {
    if (s && part && part.size) feed(s, part, playing && s.src === srcOf(part), part, m + shiftOf(p, part, b));
  }
}

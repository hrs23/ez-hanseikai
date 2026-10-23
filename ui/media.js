const AHEAD_S = 8;
const FRAGMENT_S = 1;
const PROBE_S = 0.01;
const RETRY_MS = 3000;
const HOLD_MAX_MS = 1000;
const SEEK_TOLERANCE = 0.05;
const DRIFT_MAX = 0.4;
const LONG_TASK_MS = 100;
const PROBE_MS = 1000;
const HAVE_FUTURE_DATA = 3;
const SPEAK_LEVEL = 0.02;
const SPEAK_HANG_MS = 300;
const VOICE_WINDOW = 8192;
const VOICE_SIDES = { auto: null, left: -1, right: 1, center: 0 };
const VOICE_SIDE_PARTY = "left";
const VOICE_TONES = { clear: { pan: 0.6 }, mild: { lowpass: 3000, pan: 0.6 }, strong: { highpass: 150, lowpass: 1800, pan: 0.8 } };
const VOICE_SIDE_DEFAULT = "auto";
const VOICE_TONE_DEFAULT = "clear";
const VOICE_FILTER_Q = Math.SQRT1_2;

let context = null;
const audio = () => (context ||= new AudioContext());
const talking = (last, peak, now) => (peak >= SPEAK_LEVEL ? now : last);
const stillTalking = (last, now) => now - last < SPEAK_HANG_MS;

const slot = (video, label = "") => {
  const s = { video, label, seekedAt: 0, dropped: 0, ms: null, sb: null, part: null, index: null, size: -1, base: null, shift: 0, got: new Map(), tail: 0, want: null, job: null, failedAt: 0, waitSince: 0 };
  video.onerror = () => broken(s);
  for (const type of ["waiting", "stalled", "seeking"]) video.addEventListener(type, () => videoEvent(s, type));
  return s;
};
const srcOf = (path) => convertFileSrc(path, "ezh");
const pathOf = (part) => (part ? part.path : null);
const CODECS = { avc1: "avc1.640028", hvc1: "hvc1.1.6.L93.B0", hev1: "hev1.1.6.L93.B0", av01: "av01.0.08M.08", vp09: "vp09.00.10.08", mp4a: "mp4a.40.2", Opus: "opus", fLaC: "flac" };
const typeOf = (s, init) => {
  const text = new TextDecoder("latin1").decode(init);
  return `${s.video.tagName === "VIDEO" ? "video" : "audio"}/mp4; codecs="${Object.keys(CODECS).filter((tag) => text.includes(tag)).map((tag) => CODECS[tag])}"`;
};
const rangesOf = (b) => Array.from({ length: b.length }, (_, i) => [b.start(i), b.end(i)]);
const inRanges = (list, t) => list.some(([a, b]) => a <= t && t <= b);
const bufferedEnd = (v) => (v.buffered.length ? v.buffered.end(v.buffered.length - 1) : 0);

function endOf({ segs, open }) {
  const last = segs.length - 1;
  if (last < 0) return 0;
  const at = segs[last].t_us / 1e6;
  return open ? at : at + (last ? at - segs[last - 1].t_us / 1e6 : FRAGMENT_S);
}
const covers = (index, t) => index.segs.length > 0 && t >= index.segs[0].t_us / 1e6 && t < endOf(index);
const wallEnd = (s) => (s.index && s.base !== null ? s.base + s.shift + endOf(s.index) : null);

function plan({ segs }, buffered, got, tail, t, shift) {
  const start = (i) => segs[i].t_us / 1e6 + shift;
  const holding = Math.max(segs.findLastIndex((_, k) => start(k) <= t), 0);
  const unfinished = holding > 0 && segs[holding - 1].n === tail && (got.get(tail) ?? 0) < segs[holding - 1].len;
  for (let i = holding - unfinished; i < segs.length && start(i) < t + AHEAD_S; i++) {
    const { n, len } = segs[i];
    const have = got.get(n) ?? 0;
    const end = i + 1 < segs.length ? start(i + 1) : Infinity;
    const there = buffered.some(([a, b]) => a < end && b > start(i) + PROBE_S);
    if (!there || have !== len) return { n, from: there && tail === n && have < len ? have : 0 };
  }
  return null;
}

function drained(s, t) {
  const last = s.index.segs.at(-1);
  return !s.index.open && s.got.get(last.n) === last.len && t >= bufferedEnd(s.video);
}

function waiting(s, ready, t, now) {
  const known = !!s.index && pathOf(s.part) === pathOf(s.want && s.want.part);
  const blocked = !ready && (s.video.paused || s.video.seeking) && !!s.want && (!known || (covers(s.index, t - s.shift) && !drained(s, t))) && !s.video.error && now - s.failedAt >= RETRY_MS;
  if (!blocked) { s.waitSince = 0; return false; }
  s.waitSince ||= now;
  return now - s.waitSince < HOLD_MAX_MS;
}

function detach(s) {
  if (!s.ms) return;
  URL.revokeObjectURL(s.video.src);
  s.video.removeAttribute("src");
  s.video.load();
  Object.assign(s, { ms: null, sb: null, part: null, index: null, size: -1, base: null, got: new Map(), tail: 0, job: null });
}
function broken(s) {
  detach(s);
  s.failedAt = Date.now();
}
function drop(s) {
  if (!s) return;
  detach(s);
  if (s.tap) s.tap.out.disconnect();
  s.video.remove();
}

function voiceTap(video) {
  const ctx = audio();
  const out = ctx.createMediaElementSource(video);
  const an = ctx.createAnalyser();
  an.fftSize = VOICE_WINDOW;
  const gain = ctx.createGain();
  out.connect(an);
  out.connect(gain).connect(ctx.destination);
  return { out, an, gain, buf: new Float32Array(VOICE_WINDOW) };
}
const known = (v, table, fallback) => (Object.hasOwn(table, v) ? v : fallback);
function voiceTone(side, tone, party) {
  const p = VOICE_TONES[tone];
  const pan = VOICE_SIDES[side === "auto" ? (party ? VOICE_SIDE_PARTY : "center") : side];
  if (!p || typeof pan !== "number") return null;
  const out = { ...p, pan: pan * p.pan };
  return out.pan || out.highpass || out.lowpass ? out : null;
}
function setTone(tap, tone) {
  const key = JSON.stringify(tone);
  if ((tap.tone || "null") === key) return;
  const ctx = tap.gain.context;
  const filter = (type, frequency) => new BiquadFilterNode(ctx, { type, frequency, Q: VOICE_FILTER_Q });
  const nodes = tone ? [tone.highpass && filter("highpass", tone.highpass), tone.lowpass && filter("lowpass", tone.lowpass), tone.pan && new StereoPannerNode(ctx, { pan: tone.pan })].filter(Boolean) : [];
  tap.gain.disconnect();
  for (const n of tap.nodes || []) n.disconnect();
  nodes.reduce((from, n) => from.connect(n), tap.gain).connect(ctx.destination);
  Object.assign(tap, { tone: key, nodes });
}
function peakOf({ an, buf }) {
  an.getFloatTimeDomainData(buf);
  return buf.reduce((peak, x) => Math.max(peak, Math.abs(x)), 0);
}
function setLevel(s, level) {
  if (s.tap) s.tap.gain.gain.value = level;
  else Object.assign(s.video, { muted: !level, volume: level });
}

const buffer = (sb, op) => new Promise((done, failed) => {
  sb.onupdateend = done;
  sb.onerror = failed;
  op();
});

function grownIndex(index, tail) {
  return tail.segs.length && tail.segs[0].n === index.segs.at(-1).n ? { ...tail, segs: index.segs.slice(0, -1).concat(tail.segs) } : null;
}

async function advance(s) {
  const { part, rel, first, wall } = s.want;
  const ms = s.ms;
  const get = async (what, query = "") => {
    const reply = await fetch(srcOf(`${part.path}/${what}`) + query);
    if (!reply.ok) throw new Error(`${what}: ${reply.status}`);
    return reply;
  };
  if (s.base === null) s.base = first;
  const shift = rel === null ? 0 : rel - s.base;
  const t = rel === null ? Infinity : wall / 1e6 - s.base;
  if (pathOf(s.part) !== part.path) {
    const init = await (await get("init")).arrayBuffer();
    if (s.ms !== ms) return false;
    const type = typeOf(s, init);
    if (s.sb) s.sb.changeType(type);
    else s.sb = ms.addSourceBuffer(type);
    probeDecode(s, type);
    s.sb.timestampOffset = shift;
    await buffer(s.sb, () => s.sb.appendBuffer(init));
    Object.assign(s, { part, shift, index: null, size: -1, got: new Map(), tail: 0 });
    return true;
  }
  s.part = part;
  const cue = `${part.size}${part.open ? "+" : ""}`;
  if (!s.index || s.size !== cue) {
    const last = s.index && s.index.segs.at(-1);
    const index = await (await get("index", last ? `?from=${last.n}` : "")).json();
    if (s.ms === ms) Object.assign(s, { index: last ? grownIndex(s.index, index) : index, size: cue });
    return s.ms === ms;
  }
  const sb = s.sb;
  const next = plan(s.index, rangesOf(sb.buffered), s.got, s.tail, t, s.shift);
  if (!next) {
    const last = s.index.segs.at(-1);
    if (last && !s.index.open && s.got.get(last.n) === last.len && ms.readyState === "open") ms.endOfStream();
    return false;
  }
  const bytes = await (await get(next.n, next.from ? `?from=${next.from}` : "")).arrayBuffer();
  if (s.ms !== ms) return false;
  if (!next.from && ms.readyState === "open") sb.abort();
  await buffer(sb, () => sb.appendBuffer(bytes));
  s.got.set(next.n, next.from + bytes.byteLength);
  s.tail = next.n;
  return true;
}

const relOf = (part, off) => (typeof part.start_us === "number" ? (part.start_us - off) / 1e6 : null);
function feed(s, part, off, wall, first = part) {
  s.want = part && part.size ? { part, rel: relOf(part, off), first: relOf(first, off), wall } : null;
  if (!s.want || s.job || Date.now() - s.failedAt < RETRY_MS) return;
  if (!s.ms) {
    s.ms = new MediaSource();
    s.video.src = URL.createObjectURL(s.ms);
  }
  const job = (async () => { while (s.want && s.ms && s.ms.readyState !== "closed" && (await advance(s))); })()
    .catch((e) => { if (s.job === job) { warn(e); broken(s); } })
    .finally(() => { if (s.job === job) s.job = null; });
  s.job = job;
}
const feedSeries = (s, p, series, wall) => feed(s, partFor(p, series, wall), p.off, wall, partFor(p, series, -Infinity));

function aim(s, t) {
  const v = s.video;
  const to = Math.min(t, isFinite(v.duration) ? v.duration : t);
  const off = Math.abs(v.currentTime - to);
  if (to >= 0 && isFinite(to) && off > (v.paused ? SEEK_TOLERANCE : DRIFT_MAX)) {
    if (!v.paused) playerLog(`${s.label} seek ${(v.currentTime - to).toFixed(2)} s ahead=${aheadOf(v)} rs=${v.readyState}`);
    s.seekedAt = Date.now();
    v.currentTime = to;
    if (off > JUMP_MIN) s.waitSince = 0;
  }
  return !v.seeking && v.readyState >= HAVE_FUTURE_DATA;
}

const aheadOf = (v) => Math.max(0, bufferedEnd(v) - v.currentTime).toFixed(1);
// Player lines for the app log: stalls and the codec at info, the rest at debug.
const playerLog = (text, level = "debug") => appLog(level, `player ${text}`);
function videoEvent(s, type) {
  const v = s.video;
  if (type === "seeking" && Date.now() - s.seekedAt < 200) return;
  playerLog(`${s.label} ${type} t=${v.currentTime.toFixed(2)} ahead=${aheadOf(v)} rs=${v.readyState}`, type === "seeking" ? "debug" : "info");
}
function playerProbe(slots, tiles, minimap) {
  const lost = [];
  for (const s of slots) {
    const q = s.video.getVideoPlaybackQuality && s.video.getVideoPlaybackQuality();
    if (!q) continue;
    if (q.droppedVideoFrames > s.dropped) lost.push(`${s.label}+${q.droppedVideoFrames - s.dropped}`);
    s.dropped = q.droppedVideoFrames;
  }
  if (lost.length) playerLog(`dropped ${lost.join(" ")} tiles=${tiles} minimap=${minimap ? "on" : "off"}`);
}
function watchLongTasks() {
  if (typeof PerformanceObserver !== "function") return;
  const types = ["longtask", "long-animation-frame"].filter((x) => (PerformanceObserver.supportedEntryTypes || []).includes(x));
  if (!types.length) return;
  const seen = new PerformanceObserver((list) => {
    for (const e of list.getEntries()) if (e.duration > LONG_TASK_MS) playerLog(`${e.entryType} ${Math.round(e.duration)} ms`);
  });
  for (const type of types) seen.observe({ type });
}
const probed = new Set();
async function probeDecode(s, type) {
  if (s.video.tagName !== "VIDEO" || probed.has(type) || !navigator.mediaCapabilities) return;
  probed.add(type);
  try {
    const info = await navigator.mediaCapabilities.decodingInfo({ type: "media-source", video: { contentType: type, ...CODEC_PROBE } });
    playerLog(`decode ${type.replace(/^.*codecs=/, "")} supported=${info.supported} smooth=${info.smooth} powerEfficient=${info.powerEfficient}`, "info");
  } catch (e) {
    warn(e);
  }
}

function pace(s, t, playing) {
  const v = s.video;
  if (playing && inRanges(rangesOf(v.buffered), t)) {
    if (v.paused) v.play().catch(playFailed);
  } else if (!v.paused && !playing) {
    v.pause();
  }
}

function partFor(p, series, wall) {
  let pick = null;
  for (const part of p.file.parts) if (part[series] && (!pick || part[series].start_us - p.off <= wall)) pick = part[series];
  return pick;
}
function partAt(p, wall) {
  let pick = null;
  for (const part of p.file.parts) if (!pick || (part.v || part.m).start_us - p.off <= wall) pick = part;
  return pick;
}

function feedParts(tl, p, m, b) {
  const wall = b + m * 1e6;
  const mic = partFor(p, "m", wall);
  if (mic && !tl.mic) {
    tl.mic = slot(tl.box.appendChild(el("audio", { preload: "auto" })), tl.key.slice(0, 4) + "/m");
    tl.mic.tap = voiceTap(tl.mic.video);
    applyAudio();
  }
  if (tl.media) feedSeries(tl.media, p, "v", wall);
  if (tl.mic) feedSeries(tl.mic, p, "m", wall);
}

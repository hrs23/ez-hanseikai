import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import dgram from "node:dgram";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const EXE = path.resolve(process.argv[2] || "src-tauri/target/release/ggi-hanseikai.exe");
const FFMPEG_DIR = path.resolve(process.argv[3] || path.dirname(EXE));
const OUT = path.resolve(process.env.E2E_OUT || "e2e-out");

const SEGMENT_S = 4;
const FRAGMENTS_PER_SEGMENT = 4;
const LOG_SYNC_S = 2;
const HELLO_EVERY = 3;
const DIAL_RETRY_S = 2;
const CONNECT_TIMEOUT_S = 15;
const REQUEST_TIMEOUT_S = 10;
const OFFLINE_S = 20;
const JOB_CHECK_S = 2;
const FINISH_WAIT_S = 10;
const KEEP_EVERY_S = 5;
const PARTY_REPEAT_S = 4;
const UI_REFRESH_S = 1;
const JUMP_MIN_S = 1;
const SERIES = ["v", "m"];
const PLUGIN = "apex-matches";

const RUNNER_SLOWDOWN = 3;
const COLD_START_S = 60;
const CALL_S = 30;
const POLL_MS = 500;
const SLOW_WAIT_S = 3;
const SEGMENTS_BEFORE_LATE_JOIN = 2;
const SEGMENTS_AFTER_RESTART = 2;
const SHORT_SET_SEGMENTS = 5;
const LIVE_ROUNDS = 2;
const KEYFRAME_DECODE_SPEED = 10;
const SEEK_TO_S = 5;
const LOG_TAIL_LINES = 15;
const APPROVAL_WAIT_S = LOG_SYNC_S * 3;
const SOURCE = { kind: "display", index: 1 };
const MODEL_BYTES = { base: 147_951_465, medium: 1_533_763_059 };
const WHISPER_BYTES = 8_573_270;
const LOG_LINE_CHARS = 200;
const EXPORT_FOLDER_S = 240;
const GONE = { kind: "window", exe: "gone.exe", title: "Gone" };
const CLOCK_SHIFT_S = 8.2;
const NTP_EPOCH_S = 2208988800;
const TRICKLE_MBPS = 0.01;
const LIVE_LAG_MAX_S = 12;
const LIVE_CATCH_UP = 1.05;
const CATCH_UP_S = 60;
const UNREACHABLE_S = 60;
const NTP = { c: "127.0.0.1:1" };
const LIVE_PROBE = `(() => {
  const grid = playSet() ? play : rec;
  const at = (tl) => {
    const s = tl.media;
    if (s.video.readyState < 2 || s.video.seeking || s.pending) return null;
    const p = players().find((x) => x.key === tl.key);
    const start = grid === play ? s.part && (s.part.start_us - p.off) / 1e6 : liveStart(tl.key, s.part);
    return start === null || start > liveClock.pos ? null : start + s.video.currentTime;
  };
  const tiles = Object.values(grid.tiles);
  return { now: grid === play, pos: liveClock.pos, rate: liveClock.rate, held: liveClock.held, avail: Object.fromEntries(liveMembers().map((m) => [m.key, m.avail])), at: tiles.map(at).filter((x) => x !== null), wait: tiles.some((tl) => tl.box.classList.contains("wait")), late: tiles.filter((tl) => tl.box.classList.contains("late")).map((tl) => tl.key) };
})()`;

const DIAL_S = DIAL_RETRY_S + CONNECT_TIMEOUT_S;
const SYNC_ROUND_S = LOG_SYNC_S * HELLO_EVERY;
const FRAGMENT_S = SEGMENT_S / FRAGMENTS_PER_SEGMENT;
const CRASH_AFTER_FRAGMENTS = FRAGMENTS_PER_SEGMENT / 2;
const PORT_BASE = Number(process.env.E2E_PORT_BASE || 9311);
const PORTS = { a: PORT_BASE, b: PORT_BASE + 1, c: PORT_BASE + 2 };
const API_PORT_BASE = Number(process.env.E2E_API_PORT_BASE || PORT_BASE + 100);
const API_PORTS = { a: API_PORT_BASE, b: API_PORT_BASE + 1, c: API_PORT_BASE + 2 };
const TOKEN_CHARS = 48;
const OLD_ROOM = "0ld0r00m";
const ROOM_NAME_CHARS = 40;
const OLD_OWNER = "0123456789abcdef".repeat(4);
const OLD_CAPTURE = "1700000000000";
const OLD_SET = "2026-01-01 00-00-00";
const NAMES = { a: "Ann", b: "Ben", c: "Cal" };

const within = (s) => s * RUNNER_SLOWDOWN;
const apps = {};
const timings = [];
let current = "setup";

function say(text) {
  console.log(`[${new Date().toISOString().slice(11, 19)}] ${text}`);
}

function fail(text) {
  throw new Error(text);
}

function check(ok, text) {
  if (!ok) fail(text);
}

function brief(value) {
  const text = typeof value === "string" ? value : JSON.stringify(value);
  return text === undefined ? "undefined" : text.slice(0, 400);
}

function exited() {
  return Object.values(apps).find((a) => a.proc && a.proc.exitCode !== null && !a.killed);
}

async function waitFor(label, seconds, probe) {
  const started = Date.now();
  const deadline = started + seconds * 1000;
  let last;
  for (;;) {
    const gone = exited();
    if (gone) fail(`${gone.key} exited with code ${gone.proc.exitCode} while waiting for: ${label}`);
    try {
      last = await probe();
      if (last) {
        if (Date.now() - started > SLOW_WAIT_S * 1000) say(`waited ${((Date.now() - started) / 1000).toFixed(1)} s for: ${label}`);
        return last;
      }
    } catch (e) {
      last = e.message;
    }
    if (Date.now() > deadline) fail(`timed out after ${seconds} s waiting for: ${label} (last: ${brief(last)})`);
    await sleep(POLL_MS);
  }
}

async function step(name, run) {
  current = name;
  const started = Date.now();
  say(`== ${name}`);
  await run();
  const seconds = (Date.now() - started) / 1000;
  timings.push({ step: name, seconds });
  say(`ok ${name} (${seconds.toFixed(1)} s)`);
}

function withTimeout(promise, seconds, label) {
  let timer;
  const limit = new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${label} took over ${seconds} s`)), seconds * 1000); });
  return Promise.race([promise, limit]).finally(() => clearTimeout(timer));
}

function openSocket(url) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    const cdp = { ws, next: 0, pending: new Map() };
    ws.onopen = () => resolve(cdp);
    ws.onerror = () => reject(new Error(`cannot open ${url}`));
    ws.onclose = () => {
      for (const p of cdp.pending.values()) p.reject(new Error("page closed"));
      cdp.pending.clear();
    };
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      const p = cdp.pending.get(msg.id);
      if (!p) return;
      cdp.pending.delete(msg.id);
      if (msg.error) p.reject(new Error(msg.error.message));
      else p.resolve(msg.result);
    };
  });
}

function send(app, method, params = {}) {
  const cdp = app.cdp;
  if (!cdp || cdp.ws.readyState !== WebSocket.OPEN) return Promise.reject(new Error(`${app.key} has no page`));
  const id = ++cdp.next;
  const reply = new Promise((resolve, reject) => cdp.pending.set(id, { resolve, reject }));
  cdp.ws.send(JSON.stringify({ id, method, params }));
  return withTimeout(reply, CALL_S, `${app.key} ${method}`);
}

async function evaluate(app, expression) {
  const r = await send(app, "Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
  if (r.exceptionDetails) {
    const ex = r.exceptionDetails.exception || {};
    fail(`${app.key}: ${ex.description || brief(ex.value) || r.exceptionDetails.text}`);
  }
  return r.result.value;
}

async function pressAndConfirm(app, call, yes = true) {
  await evaluate(app, `${call}; true`);
  await waitFor(`${app.key} asks before it goes on`, within(UI_REFRESH_S * 5), () => evaluate(app, `$("confirm").open`));
  const title = await evaluate(app, `$("confirmtitle").textContent`);
  app.confirmText = await evaluate(app, `$("confirmtext").textContent`);
  await evaluate(app, `$(${yes ? '"confirmyes"' : '"confirmno"'}).click(); true`);
  return title;
}

async function invoke(app, name, args = {}, seconds = CALL_S) {
  const r = await api(app, "POST", `/v1/ops/${name}`, { token: app.token, body: args, timeout: seconds });
  return r.status === 200 ? r.json : fail(r.json?.error || r.text);
}
const state = (app) => invoke(app, "state");
const onLive = (app) => evaluate(app, `page === "player" && navActive() === "live" && $("sidenav").querySelector("[aria-current=page]").dataset.nav === "live"`);

function frame(record) {
  const body = Buffer.from(JSON.stringify(record));
  const head = Buffer.alloc(8);
  head.writeUInt32LE(body.length, 0);
  head.writeUInt32LE(crc32(body), 4);
  return Buffer.concat([head, body]);
}

function crc32(bytes) {
  let crc = ~0;
  for (const b of bytes) {
    crc ^= b;
    for (let i = 0; i < 8; i++) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
  }
  return ~crc >>> 0;
}

function oldFiles() {
  const room = path.join("rooms", OLD_ROOM);
  const capture = path.join(room, OLD_OWNER, OLD_CAPTURE);
  const kept = path.join(room, "recordings", OLD_SET, OLD_OWNER, OLD_CAPTURE);
  return {
    [path.join(capture, "v", "init.mp4")]: "old init",
    [path.join(capture, "v", "000001.m4s")]: "old segment",
    [path.join(capture, "v", "open.json")]: JSON.stringify({ n: 2, set: OLD_SET }),
    [path.join(capture, "cap.json")]: JSON.stringify({ start_us: 1 }),
    [path.join(capture, "verified.json")]: JSON.stringify({ "v/000001.m4s": { size: 11, ok: true } }),
    [path.join(kept, "verified.json")]: JSON.stringify({ "v/000001.m4s": { size: 11, ok: false } }),
    [path.join(kept, "v", "init.mp4")]: "old init",
    [path.join(kept, "v", "000001.m4s")]: "old segment",
    [path.join(room, "recordings", "orphan", OLD_OWNER, OLD_CAPTURE, "v", "000001.m4s")]: "orphan segment",
    "rec-1767225600000.mp4": "old recording",
    "log.bin": frame({ k: "note", text: "old note", t_us: 1 }),
    "peer.txt": "old peer",
    "peers.txt": "old peers",
    [path.join("peer", "a1b2", "log.bin")]: frame({ k: "file", set: "s1", size: 1 }),
    [path.join("legacy", "notes.json")]: "[]",
    [path.join(room, "sets", "s1", "set.json")]: JSON.stringify({ id: "s1", players: ["someone"] }),
    [path.join(room, "sets", "s1", "someone.mp4")]: "old member file",
    [path.join(room, "live", "someone.mp4")]: "old live file",
    [path.join(room, "logs", "someone.bin")]: Buffer.concat([frame({ k: "rec", set: "s1", room: OLD_ROOM, start_us: 1 }), frame({ k: "file", set: "s1", size: 15 }), frame({ k: "note", text: "hi" })]),
  };
}

function keptOldFiles() {
  const room = path.join("rooms", OLD_ROOM);
  const kept = path.join(room, "recordings", OLD_SET, OLD_OWNER, OLD_CAPTURE);
  return [path.join("legacy", "notes.json"), path.join(kept, "v", "init.mp4"), path.join(kept, "v", "000001.m4s"), path.join(room, "recordings", "orphan", OLD_OWNER, OLD_CAPTURE, "v", "000001.m4s")];
}

const oldStandby = () => path.join("rooms", OLD_ROOM, "standby", OLD_OWNER, OLD_CAPTURE, "v", "000001.m4s");

function seedOld(dir) {
  for (const [name, body] of Object.entries({ ...oldFiles(), [oldStandby()]: "live segment" })) {
    fs.mkdirSync(path.dirname(path.join(dir, name)), { recursive: true });
    fs.writeFileSync(path.join(dir, name), body);
  }
  fs.writeFileSync(path.join(dir, "log.old.txt"), "old log");
  fs.writeFileSync(path.join(dir, "settings.json"), JSON.stringify({ limit_mbps: 0, keep_days: 0, source: "display:1", language: "xx", quality: "ultra", obs_port: 4455 }));
  fs.writeFileSync(path.join(dir, "rooms.json"), JSON.stringify({ active: null, list: [{ id: "oldshape", title: "from an older version" }] }));
}

function start(key, extra) {
  const home = path.join(OUT, "apps", key);
  const app = apps[key] || { key, name: NAMES[key], data: path.join(home, "data") };
  if (extra) app.extra = extra;
  if (key === "c" && !apps[key]) seedOld(app.data);
  fs.mkdirSync(app.data, { recursive: true });
  const env = {
    ...process.env,
    EZH_TEST: String(PORTS[key]),
    EZH_API: "1",
    EZH_API_PORT: String(API_PORTS[key]),
    EZH_DIR: app.data,
    EZH_NTP: NTP[key],
    ...app.extra,
  };
  const stderr = fs.openSync(path.join(OUT, `${key}-stderr.txt`), "a");
  app.proc = spawn(EXE, [], { env, stdio: ["ignore", "ignore", stderr], windowsHide: true });
  fs.closeSync(stderr);
  app.killed = false;
  app.cdp = null;
  apps[key] = app;
  return app;
}

function targets(port) {
  return new Promise((resolve, reject) => {
    const req = http.get({ host: "127.0.0.1", port, path: "/json/list", agent: false, timeout: CALL_S * 1000 }, (res) => {
      let body = "";
      res.setEncoding("utf8");
      res.on("data", (chunk) => { body += chunk; });
      res.on("end", () => {
        try {
          resolve(JSON.parse(body));
        } catch {
          reject(new Error(`HTTP ${res.statusCode}: ${body.slice(0, 200)}`));
        }
      });
    });
    req.on("timeout", () => req.destroy(new Error("no answer")));
    req.on("error", (e) => reject(new Error(e.code || e.message)));
  });
}

async function attach(app) {
  const target = await waitFor(`${app.key} page on port ${PORTS[app.key]}`, within(COLD_START_S), async () => {
    const list = await targets(PORTS[app.key]).catch((e) => {
      app.seen = { ...app.seen, [e.message]: (app.seen?.[e.message] || 0) + 1 };
      fail(brief(app.seen));
    });
    return list.find((t) => t.type === "page" && t.url.startsWith("http") && t.webSocketDebuggerUrl);
  });
  app.cdp = await openSocket(target.webSocketDebuggerUrl);
  await waitFor(`${app.key} UI ready`, within(COLD_START_S), () => evaluate(app, "typeof refresh === 'function' && !!window.__TAURI__ && ready === true"));
  app.token = await waitFor(`${app.key} api.json`, within(COLD_START_S), () => readJson(path.join(app.data, "api.json"))?.token);
  app.id = (await state(app)).id;
  return app;
}

async function share(app) {
  check(!(await state(app)).capture.source, `${app.key} starts with a capture source`);
  await invoke(app, "set_source", { source: SOURCE });
}

async function launch(key, extra) {
  const app = await attach(start(key, extra));
  await invoke(app, "set_name", { name: app.name });
  await invoke(app, "set_language", { language: "en" });
  await invoke(app, "set_quality", { id: "light" });
  await share(app);
  say(`${key} is ${app.id.slice(0, 10)}`);
  return app;
}

function timeServer(shiftS) {
  const socket = dgram.createSocket("udp4");
  socket.on("message", (asked, from) => {
    const answer = Buffer.alloc(48);
    answer[0] = 0x24;
    answer[1] = 1;
    asked.copy(answer, 24, 40, 48);
    const ms = Date.now() + shiftS * 1000;
    for (const at of [32, 40]) {
      answer.writeUInt32BE(Math.floor(ms / 1000) + NTP_EPOCH_S, at);
      answer.writeUInt32BE(Math.floor(((ms % 1000) / 1000) * 2 ** 32), at + 4);
    }
    socket.send(answer, from.port, from.address);
  });
  return new Promise((done) => socket.bind(0, "127.0.0.1", () => done(`127.0.0.1:${socket.address().port}`)));
}

async function recordsAgain(app, room, set, old) {
  await waitFor(`${app.key} records the set again in a new capture`, within(SEGMENT_S * 2), () => {
    const own = logOf(app, room, app.id);
    const ended = own.findLastIndex((r) => r.k === "rec_end" && r.set === set);
    return own.findLastIndex((r) => r.k === "rec" && r.set === set) > ended && segsOf(own, set).some((s) => !old.includes(s.capture));
  });
}

const marker = (app, name, on) => (on ? fs.writeFileSync(path.join(app.data, name), "") : fs.rmSync(path.join(app.data, name), { force: true }));

const lastJoin = (app, room, set) => logOf(app, room, app.id).findLast((r) => r.set === set && (r.k === "rec" || r.k === "rec_end"));


async function watchLive(app, b, seconds, until = () => false) {
  const end = Date.now() + seconds * 1000;
  let last = null;
  while (Date.now() < end) {
    const x = await evaluate(app, LIVE_PROBE);
    check(!x.wait, "a live tile with a picture hid it");
    check(x.at.length < 2 || Math.max(...x.at) - Math.min(...x.at) < 1, `live tiles are apart: ${brief(x)}`);
    check(x.pos === null || x.avail[b.id] === null || x.late.includes(b.id) || x.pos <= x.avail[b.id], `the live position passed B's media: ${brief(x)}`);
    check(!last || (x.pos >= last.pos && x.now === last.now), `the live position went back or the view changed: ${brief([last, x])}`);
    if (until(x)) return x;
    last = x;
    await sleep(POLL_MS);
  }
  return last;
}

async function connects(app, peer, since = 0) {
  const view = await invoke(app, "debug_view", { tab: "connections", since, set: null });
  return view.events.filter((e) => e.text.startsWith(peer.id.slice(0, 10)) && e.text.includes(": connected")).length;
}

function api(app, method, route, { token, host, body, connect, timeout = CALL_S } = {}) {
  const port = API_PORTS[app.key];
  return new Promise((resolve, reject) => {
    const payload = body === undefined ? null : Buffer.from(typeof body === "string" ? body : JSON.stringify(body));
    const headers = { Host: host || `127.0.0.1:${port}`, ...(token ? { Authorization: `Bearer ${token}` } : {}), ...(payload ? { "Content-Type": "application/json", "Content-Length": payload.length } : {}) };
    const req = http.request({ host: connect || "127.0.0.1", port, method, path: route, headers, agent: false, timeout: timeout * 1000 }, (res) => {
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => {
        const text = Buffer.concat(chunks).toString("utf8");
        let json = null;
        try {
          json = JSON.parse(text);
        } catch {}
        resolve({ status: res.statusCode, text, json });
      });
    });
    req.on("timeout", () => req.destroy(new Error("no answer")));
    req.on("error", (e) => reject(new Error(e.code || e.message)));
    if (payload) req.write(payload);
    req.end();
  });
}

function bytes(n) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  while (n >= 1000 && i < units.length - 1) { n /= 1000; i++; }
  return `${n.toFixed(i && n < 10 ? 1 : 0)} ${units[i]}`;
}

function descendants(pid) {
  const out = spawnSync("powershell", ["-NoProfile", "-Command", "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId) $($_.ParentProcessId)\" }"], { encoding: "utf8" }).stdout || "";
  const pairs = out.trim().split(/\r?\n/).map((line) => line.split(" ").map(Number));
  const found = [pid];
  for (let i = 0; i < found.length; i++) found.push(...pairs.filter(([child, parent]) => parent === found[i] && !found.includes(child)).map(([child]) => child));
  return found;
}

function alive(pid) {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

function kill(app, tree = true) {
  if (!app.proc || app.proc.exitCode !== null) return;
  app.killed = true;
  app.tree = descendants(app.proc.pid);
  spawnSync("taskkill", ["/PID", String(app.proc.pid), ...(tree ? ["/T"] : []), "/F"], { stdio: "ignore" });
}

async function stopped(app) {
  await waitFor(`${app.key} and its child processes exit`, within(CALL_S), () => app.proc.exitCode !== null && !app.tree.some(alive));
}

function records(file) {
  if (!fs.existsSync(file)) return [];
  const bytes = fs.readFileSync(file);
  const out = [];
  let at = 0;
  while (at + 8 <= bytes.length) {
    const len = bytes.readUInt32LE(at);
    if (at + 8 + len > bytes.length) break;
    try {
      out.push(JSON.parse(bytes.subarray(at + 8, at + 8 + len).toString("utf8")));
    } catch {
      break;
    }
    at += 8 + len;
  }
  return out;
}

const roomDir = (app, room) => path.join(app.data, "rooms", room);
const logFile = (app, room, owner) => path.join(roomDir(app, room), "logs", `${owner}.bin`);
const logOf = (app, room, owner) => records(logFile(app, room, owner));
const segName = (n) => `${String(n).padStart(6, "0")}.m4s`;
const sha256 = (file) => createHash("sha256").update(fs.readFileSync(file)).digest("hex");
const readJson = (file) => { try { return JSON.parse(fs.readFileSync(file, "utf8")); } catch { return null; } };
const fileSize = (file) => (fs.existsSync(file) ? fs.statSync(file).size : 0);

function segsOf(owned, set, series) {
  return owned.filter((r) => r.k === "seg" && r.set === set && (!series || r.series === series));
}

function mediaFile(app, room, set, owner, capture, series, name) {
  return path.join(roomDir(app, room), "recordings", set, owner, capture, series, name);
}

function filesUnder(dir) {
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? filesUnder(path.join(dir, e.name)) : [path.join(dir, e.name)]));
}

function notInStandby(holder, room, owners, set) {
  for (const owner of owners) {
    const stray = pieces(logOf(owner, room, owner.id), set).filter((p) => p.name !== "init.mp4" && fs.existsSync(path.join(roomDir(holder, room), "standby", owner.id, p.capture, p.series, p.name)));
    check(!stray.length, `${holder.key} keeps ${owner.key}'s ${stray[0]?.capture}/${stray[0]?.series}/${stray[0]?.name} of the recording in standby`);
  }
}

function pieces(owned, set, limit = Infinity) {
  const caps = Object.fromEntries(owned.filter((r) => r.k === "cap").map((r) => [r.capture, r]));
  const segs = SERIES.flatMap((series) => segsOf(owned, set, series).slice(0, limit));
  const runs = [...new Set(segs.map((s) => `${s.capture}/${s.series}`))].map((k) => k.split("/"));
  const inits = runs.map(([capture, series]) => ({ capture, series, name: "init.mp4", ...(caps[capture]?.init?.[series] || {}) }));
  return [...inits.map((i) => ({ ...i, set })), ...segs.map((s) => ({ set, capture: s.capture, series: s.series, name: segName(s.n), size: s.size, sha256: s.sha256 }))];
}

function badCopies(holder, room, owner, list) {
  return list
    .filter((p) => {
      const file = mediaFile(holder, room, p.set, owner, p.capture, p.series, p.name);
      return !p.sha256 || fileSize(file) !== p.size || sha256(file) !== p.sha256;
    })
    .map((p) => `${p.capture}/${p.series}/${p.name}`);
}

function wholeFragments(file) {
  if (!fs.existsSync(file)) return 0;
  const bytes = fs.readFileSync(file);
  let at = 0;
  let n = 0;
  while (at + 8 <= bytes.length) {
    const size = bytes.readUInt32BE(at);
    if (size < 8 || at + size > bytes.length) break;
    if (bytes.toString("latin1", at + 4, at + 8) === "mdat") n++;
    at += size;
  }
  return n;
}

function setOf(st, set) {
  return st.sets.find((s) => s.id === set);
}

function heldByAll(st, set, list) {
  const found = setOf(st, set);
  return !!found && found.players.length > 0 && found.players.every((p) => list.every((x) => p.verified_by.includes(x.id)));
}

async function onlyOnDownload(others, holder, set) {
  await sleep(SYNC_ROUND_S * 1000);
  for (const app of others) {
    const row = setOf(await state(app), set);
    check(row && row.progress.have === 0 && !row.wanted && row.progress.total > 0, `${app.key} fetched a finished recording nobody asked for: ${brief(row?.progress)}`);
    const shown = await evaluate(app, `(() => { const row = [...document.querySelectorAll("#sessions .srow.gone")].find((r) => r.textContent.includes(sessionName(${JSON.stringify(set)}))); return row ? [row.textContent.includes(t("Not on this PC")), !!row.querySelector("button"), row.textContent.includes(bytes(setById(${JSON.stringify(set)}).progress.total))] : null; })()`);
    check(shown && shown.every(Boolean), `${app.key} does not list the recording as not on this PC with its size and Download: ${brief(shown)}`);
    check(!fs.existsSync(path.join(roomDir(app, (await state(app)).room.id), "recordings", set)), `${app.key} holds a folder of a recording it did not fetch`);
  }
  for (const app of others) await invoke(app, "download_set", { room: (await state(app)).room.id, set });
  check((await state(holder)).sets.find((x) => x.id === set).progress.have > 0, `${holder.key} lost its own recording`);
}

function connected(st, ids) {
  return ids.every((id) => st.peers.some((p) => p.id === id && p.connected));
}

async function allSee(list, label, seconds, probe) {
  await Promise.all(list.map((app) => waitFor(`${app.key}: ${label}`, seconds, async () => probe(await state(app), app))));
}

const meshed = (list, room) => allSee(list, "connected to each other", within(DIAL_S * 2), (st, app) => (!room || (st.room?.id === room && !st.room.pending)) && connected(st, list.filter((x) => x !== app).map((x) => x.id)));

async function asks(approver, joiner, inviter) {
  await invoke(joiner, "join_room", { code: await invoke(inviter, "create_invite", { reuse: false }) });
  await waitFor(`${approver.key} sees ${joiner.key} asking to join`, within(DIAL_S * 2), async () => (await state(approver)).asks.some((x) => x.id === joiner.id));
}

function holdEverything(list, room, set, owners, limits = {}) {
  for (const holder of list) {
    for (const owner of owners) {
      const bad = badCopies(holder, room, owner.id, pieces(logOf(owner, room, owner.id), set, limits[owner.key]));
      if (bad.length) return `${holder.key} lacks ${bad.length} of ${owner.key}'s pieces, first ${bad[0]}`;
    }
  }
  return "";
}

function ffmpeg(tool, args) {
  const r = spawnSync(path.join(FFMPEG_DIR, `${tool}.exe`), args, { encoding: "utf8", maxBuffer: 64 << 20 });
  return { ok: r.status === 0, out: r.stdout || "", err: (r.stderr || "").trim() };
}

async function screenshot(app, name) {
  const shot = await send(app, "Page.captureScreenshot", { format: "png" });
  fs.writeFileSync(path.join(OUT, `${name}.png`), Buffer.from(shot.data, "base64"));
}

function recordedFrame(app, room, name) {
  const mine = logOf(app, room, app.id).filter((r) => r.k === "seg" && r.series === "v");
  const last = mine[mine.length - 1];
  if (!last) return;
  const joined = path.join(OUT, `${name}.mp4`);
  const file = (leaf) => mediaFile(app, room, last.set, app.id, last.capture, "v", leaf);
  fs.writeFileSync(joined, Buffer.concat([fs.readFileSync(file("init.mp4")), fs.readFileSync(file(segName(last.n)))]));
  ffmpeg("ffmpeg", ["-v", "error", "-y", "-i", joined, "-frames:v", "1", path.join(OUT, `${name}.png`)]);
  fs.rmSync(joined, { force: true });
}

function annotate(kind, title, text) {
  const escaped = text.replace(/%/g, "%25").replace(/\r/g, "").replace(/\n/g, "%0A");
  console.log(`::${kind} title=${title}::${escaped}`);
}

async function collect(room) {
  for (const app of Object.values(apps)) {
    const log = path.join(app.data, "log.txt");
    if (fs.existsSync(log)) fs.copyFileSync(log, path.join(OUT, `${app.key}-log.txt`));
    if (app.cdp) {
      await screenshot(app, `${app.key}-window`).catch(() => {});
      await state(app).then((st) => fs.writeFileSync(path.join(OUT, `${app.key}-state.json`), JSON.stringify(st, null, 1))).catch(() => {});
    }
    if (room && app.id) {
      try {
        recordedFrame(app, room, `${app.key}-frame`);
      } catch {}
    }
  }
}

function processes() {
  const script = "Get-CimInstance Win32_Process | Where-Object { $_.Name -match 'ggi-hanseikai' -or $_.CommandLine -match 'remote-debugging-port' } | ForEach-Object { \"$($_.ProcessId) $($_.ParentProcessId) $($_.Name) $($_.CreationDate.ToString('HH:mm:ss')) $(if ($_.CommandLine -match 'remote-debugging-port=(\\d+)') { $matches[1] })\" }; Get-Process ggi-hanseikai, msedgewebview2 -ErrorAction SilentlyContinue | ForEach-Object { \"$($_.Id) $($_.ProcessName) responding=$($_.Responding) window=$($_.MainWindowHandle) title=$($_.MainWindowTitle) threads=$($_.Threads.Count) cpu=$([int]$_.CPU)\" }; netstat -ano | Select-String ':93[0-9][0-9] '; Get-ChildItem $env:E2E_OUT\\apps\\*\\data | ForEach-Object { \"$($_.FullName.Substring($env:E2E_OUT.Length)) $($_.LastWriteTime.ToString('HH:mm:ss'))\" }; Get-ChildItem -Recurse -Filter DevToolsActivePort $env:E2E_OUT | ForEach-Object { \"$($_.FullName) $($_.LastWriteTime.ToString('HH:mm:ss')) $((Get-Content $_.FullName) -join ' ')\" }; Get-Date -Format HH:mm:ss";
  const r = spawnSync("powershell", ["-NoProfile", "-Command", script], { encoding: "utf8" });
  return (r.stdout || r.stderr || "").trim() || "no process with a remote-debugging port and nothing on ports 9300 to 9399";
}

function logTail(app) {
  const log = path.join(app.data, "log.txt");
  if (!fs.existsSync(log)) return "no log";
  return fs.readFileSync(log, "utf8").trim().split("\n").slice(-LOG_TAIL_LINES).map((l) => l.slice(0, LOG_LINE_CHARS)).join("\n");
}

async function live(ctx) {
  let a, b, c;
  await step("start A and B", async () => {
    a = await launch("a");
    b = await launch("b");
  });

  await step("recording alone, the player follows the picture as it grows", async () => {
    ctx.room = await invoke(a, "create_room", { name: "Practice" });
    await waitFor("A opens the room on Live", within(UI_REFRESH_S * 5), () => onLive(a));
    await allSee([a], "encoding its test source", within(COLD_START_S), (st) => st.capture.live);
    await evaluate(a, "openNow(); true");
    await pressAndConfirm(a, "toggleRecording()");
    const alone = (await waitFor("A records", within(UI_REFRESH_S * 5), async () => (await state(a)).active_set?.id)) || "";
    const view = () => evaluate(a, `(() => { const v = Object.values(play.tiles)[0]?.media.video; return { shown: selected === ${JSON.stringify(alone)}, review: !!review, playing: clock.playing, cur: v ? v.currentTime : 0, dur: v && isFinite(v.duration) ? v.duration : 0 }; })()`);
    const first = await waitFor("A's player plays the growing recording", within(SEGMENT_S * 4), async () => { const x = await view(); return x.shown && !x.review && x.playing && x.cur > 0 && x.dur > 0 && x; });
    await waitFor("A's player moves on with the picture", within(SEGMENT_S * 3), async () => { const x = await view(); return !x.review && x.cur > first.cur + FRAGMENT_S * 2 && x.dur > first.dur + SEGMENT_S && x; });
    await pressAndConfirm(a, "toggleRecording()");
    await waitFor("A offers to discard", within(UI_REFRESH_S * 5), () => evaluate(a, `$("confirm").open && $("confirmtitle").textContent === t("Discard this recording?")`));
    await evaluate(a, `$("confirmyes").click(); true`);
    await waitFor("A dropped the recording", within(UI_REFRESH_S * 5), async () => { const st = await state(a); return !st.active_set && !setOf(st, alone); });
    await waitFor("A is back on Live", within(UI_REFRESH_S * 5), () => onLive(a));
    await evaluate(a, "openLive(); true");
    await waitFor("A shares no recording", within(UI_REFRESH_S * 5), async () => !(await state(a)).party.shared.set);
  });

  await step("B joins A's room with approval", async () => {
    await asks(a, b, a);
    await sleep(APPROVAL_WAIT_S * 1000);
    await invoke(a, "answer_ask", { id: b.id, approve: true });
    await meshed([a, b], ctx.room);
    const names = (await state(b)).names;
    check(names[a.id] === a.name, `B knows A as ${names[a.id]}`);
    const dials = await connects(b, a);
    check(dials === 1, `B connected to A ${dials} times while waiting for approval`);
  });

  await step("a second app on A's data folder exits and leaves A running", async () => {
    const env = { ...process.env, EZH_TEST: String(PORT_BASE + 3), EZH_DIR: a.data };
    const second = spawn(EXE, [], { env, stdio: "ignore", windowsHide: true });
    const code = await withTimeout(new Promise((done) => second.on("exit", done)), CALL_S, "the second app");
    check(code === 0, `the second app exited with ${code}`);
    const starts = fs.readFileSync(path.join(a.data, "log.txt"), "utf8").split("\n").filter((l) => l.includes(" started, process ")).length;
    check(starts === 1, `A's log shows ${starts} starts`);
    check(fs.existsSync(path.join(a.data, "EBWebView")), "A's WebView2 profile is not in its data folder");
    const st = await state(a);
    check(connected(st, [b.id]), "A lost B when a second app started");
  });

  await step("Record on A starts the set on B", async () => {
    await allSee([a, b], "encoding its test source", within(COLD_START_S), (st) => st.capture.live);
    await invoke(a, "start_recording");
    ctx.set = (await state(a)).active_set.id;
    await allSee([b], "recording the same set", within(LOG_SYNC_S * 2), (st) => st.active_set?.id === ctx.set);
  });

  await step("C joins late through B's invite while B is cut off, approved on A once B is back", async () => {
    await waitFor(`A has logged ${SEGMENTS_BEFORE_LATE_JOIN} segments`, within(SEGMENT_S * (SEGMENTS_BEFORE_LATE_JOIN + 1)), () => segsOf(logOf(a, ctx.room, a.id), ctx.set, "v").length >= SEGMENTS_BEFORE_LATE_JOIN);
    c = await launch("c");
    const code = await invoke(b, "create_invite", { reuse: false });
    marker(b, "offline", true);
    await invoke(c, "join_room", { code });
    await sleep(DIAL_S * 1000);
    check(!(await state(a)).asks.some((x) => x.id === c.id) && (await state(c)).room?.pending, "C's join did not wait for its inviter");
    marker(b, "offline", false);
    await waitFor("A sees C asking to join", within(DIAL_S * 2), async () => (await state(a)).asks.some((x) => x.id === c.id));
    await invoke(a, "answer_ask", { id: c.id, approve: true });
    const all = [a, b, c];
    await meshed(all, ctx.room);
    await allSee([c], "recording the running set", within(SYNC_ROUND_S * 2), (st) => st.active_set?.id === ctx.set);
  });

  await step("live segments flow and C gets the history", async () => {
    const all = [a, b, c];
    for (let round = 0; round < LIVE_ROUNDS; round++) {
      const limits = Object.fromEntries(all.map((x) => [x.key, segsOf(logOf(x, ctx.room, x.id), ctx.set, "v").length]));
      await waitFor(`everyone holds what was logged by now (${brief(limits)})`, within((SEGMENT_S + LOG_SYNC_S) * 2), async () => {
        check((await state(a)).active_set?.id === ctx.set, "the set stopped early");
        return !holdEverything(all, ctx.room, ctx.set, all, limits);
      });
    }
    for (const holder of all) notInStandby(holder, ctx.room, all, ctx.set);
    await allSee([b, c], "A's live picture available", within(SEGMENT_S + LOG_SYNC_S), (st) => st.peers.find((p) => p.id === a.id)?.live);
  });

  await step("B's time answer runs 8.2 s ahead and C gets none, and the recording still lines up", async () => {
    check(Math.abs((await state(b)).clock.offset_us - CLOCK_SHIFT_S * 1e6) < 5e5, `B measured ${brief((await state(b)).clock)}`);
    check(await evaluate(b, `pcCards(st)[0].rows.find((r) => r.key === "Clock").text === "8.2 s behind"`), "B's own card does not say its clock is 8.2 s behind");
    const offset = (st, x) => st.peers.find((p) => p.id === x.id)?.offset_us;
    await waitFor("A measures B 8.2 s ahead and C in step", within(SYNC_ROUND_S * 2), async () => { const st = await state(a); return Math.abs(offset(st, b) - CLOCK_SHIFT_S * 1e6) < 1e6 && Math.abs(offset(st, c)) < 1e6; });
    check((await state(c)).clock.offset_us === null && (await evaluate(c, `pcCards(st)[0].rows.find((r) => r.key === "Clock").text === t("Unreachable")`)), "C's own card does not say its time check is unreachable");
    const flow = await invoke(c, "debug_view", { tab: "flow", since: 0, set: null });
    check(await evaluate(c, `flowRows(st, ${JSON.stringify(flow)}, (k) => k).find((r) => r.key === "Time check").level === "warn"`), "C's Where data goes does not warn about the time check");
    const players = setOf(await state(a), ctx.set).players;
    const begins = (x) => { const p = players.find((y) => y.owner === x.id); return (p.start_us - p.offset_us) / 1e6; };
    check(Math.abs(begins(b) - begins(a)) < within(1), `B's part starts ${begins(b) - begins(a)} s from A's`);
  });

  await step("while recording the start box says Stop recording with the time and the list gets the running recording", async () => {
    check(await evaluate(a, `$("startbtn").textContent.includes(t("Stop recording")) && !!$("startbtn").querySelector("#livetimer") && !!$("startbtn").querySelector(".recdot") && $("standby").hidden && !!$("sessions").querySelector(".liverow .thumb.live")`), "the start box or the running recording's row is wrong");
  });

  await step("Live shows a tile for everyone who shares a screen, also while recording", async () => {
    for (const app of [a, b, c]) {
      await evaluate(app, `openLive(); true`);
      await waitFor(`${app.key} shows three live tiles`, within(UI_REFRESH_S * 10), () => evaluate(app, `!$("livetiles").hidden && Object.keys(rec.tiles).length === 3 && $("ptitle").textContent === t("Live")`));
    }
    check(await evaluate(a, `$("miccap").classList.contains("off") && $("screencap").classList.contains("recording") && getComputedStyle($("miccap")).borderTopStyle === "dashed" && !!$("miclvl")`), "the bottom-left buttons are not in the shared off/recording states");
    await screenshot(a, "live-recording");
    await waitFor("A marks only Live, not the running recording, and names the online count once", within(UI_REFRESH_S * 3), () => evaluate(a, `!$("sessions").querySelector(".liverow.on") && $("sidenav").querySelector("[aria-current=page]").dataset.nav === "live" && $("psub").textContent === t("{n} online", { n: 3 })`));
    await evaluate(a, `$("sessions").querySelector(".liverow").click(); true`);
    await waitFor("A marks only the running recording after opening it", within(UI_REFRESH_S * 3), () => evaluate(a, `!!$("sessions").querySelector(".liverow.on") && !$("sidenav").querySelector("[aria-current=page]")`));
    await evaluate(a, `openLive(); true`);
  });

  await step("connections stay up while live tiles stay in step, a member stalls, and the others stop waiting for one too far behind", async () => {
    const inStep = async (view) => {
      for (const x of [a, b, c]) await evaluate(x, `${view}; true`);
      await waitFor(`A's live tiles in step after ${view}`, within(SEGMENT_S * 3), async () => { const x = await evaluate(a, LIVE_PROBE); check(x.pos !== null && x.at.length === 3 && Math.max(...x.at) - Math.min(...x.at) < 1, brief(x)); return true; });
    };
    const all = [a, b, c];
    const since = Object.fromEntries(await Promise.all(all.map(async (x) => [x.key, (await state(x)).now_us])));
    await inStep("openNow()");
    await watchLive(a, b, SEGMENT_S * 2);
    for (const app of all) {
      for (const peer of all.filter((x) => x !== app)) {
        const n = await connects(app, peer, since[app.key]);
        check(n === 0, `${app.key} connected to ${peer.key} ${n} times while the live tiles ran`);
      }
      check(connected(await state(app), all.filter((x) => x !== app).map((x) => x.id)), `${app.key} lost a peer`);
    }
    await inStep("openLive()");
    await invoke(b, "set_limit", { mbps: TRICKLE_MBPS });
    const held = await watchLive(a, b, LIVE_LAG_MAX_S + SEGMENT_S, (x) => x.held || x.late.length > 0);
    await invoke(b, "set_limit", { mbps: null });
    check(held.held, `A never held for B: ${brief(held)}`);
    const fast = await watchLive(a, b, SEGMENT_S * 3, (x) => x.rate === LIVE_CATCH_UP);
    check(fast.rate === LIVE_CATCH_UP, `A never caught up at ${LIVE_CATCH_UP}: ${brief(fast)}`);
    await waitFor("A plays at 1 again once the delay is small", within(CATCH_UP_S), async () => (await evaluate(a, LIVE_PROBE)).rate === 1);
    await evaluate(a, `openNow(); true`);
    await invoke(b, "set_limit", { mbps: TRICKLE_MBPS });
    const gone = await waitFor("A stops waiting for B and marks its tile", within(LIVE_LAG_MAX_S + SEGMENT_S * 2), async () => { const x = await evaluate(a, LIVE_PROBE); return x.late.join() === b.id && x.pos > x.avail[b.id] && x; });
    await sleep(1000);
    check((await evaluate(a, LIVE_PROBE)).pos > gone.pos, "the others stopped while waiting for B");
    await invoke(b, "set_limit", { mbps: null });
    await waitFor("B's tile is no longer late", within(SEGMENT_S * 3), async () => !(await evaluate(a, LIVE_PROBE)).late.length);
  });

  await step("a note with a tag, its edit and its deletion reach everyone", async () => {
    const id = await invoke(b, "add_event", { kind: "note", text: "Rotate early #rotate", target: { t_us: Date.now() * 1000 }, values: null });
    const key = `${b.id}/${id}`;
    const others = [a, c];
    const seen = (st) => st.events.find((e) => e.key === key);
    await allSee(others, "the note with its tag", within(LOG_SYNC_S * 2), (st) => seen(st)?.tags.includes("rotate"));
    await invoke(b, "event_action", { kind: "edit", id, text: "Rotate later #late" });
    await allSee(others, "the edit", within(LOG_SYNC_S * 2), (st) => seen(st)?.text === "Rotate later #late" && seen(st).tags.join() === "late");
    await invoke(b, "event_action", { kind: "delete", id, text: "" });
    await allSee(others, "the deletion", within(LOG_SYNC_S * 2), (st) => !seen(st));
  });


  await step("the set stops and everyone has it", async () => {
    const all = [a, b, c];
    await invoke(a, "stop_recording");
    await allSee(all, "the set ended and everyone has it", within((SEGMENT_S + SYNC_ROUND_S) * 3), (st) => !st.active_set && setOf(st, ctx.set) && !setOf(st, ctx.set).live && heldByAll(st, ctx.set, all));
  });

  await step("C loses its own log and gets all of it back before writing again", async () => {
    kill(c);
    await stopped(c);
    const copy = fs.readFileSync(logFile(a, ctx.room, c.id));
    fs.rmSync(logFile(c, ctx.room, c.id));
    start("c");
    await attach(c);
    await share(c);
    const pending = path.join(roomDir(c, ctx.room), "pending.bin");
    await waitFor("C holds its whole log again", within(DIAL_S + SYNC_ROUND_S * 2), () => fileSize(logFile(c, ctx.room, c.id)) >= copy.length && !fs.existsSync(pending));
    check(fs.readFileSync(logFile(c, ctx.room, c.id)).subarray(0, copy.length).equals(copy), "C's restored log differs from A's copy of it");
    const all = [a, b, c];
    await meshed(all);
    const id = await invoke(c, "add_event", { kind: "note", text: "After the restore", target: {}, values: null });
    await allSee([a, b], "C's note written after the restore", within(REQUEST_TIMEOUT_S + DIAL_S + LOG_SYNC_S), (st) => st.events.some((e) => e.key === `${c.id}/${id}`));
  });

  await step("removing C for the whole room while a recording runs stops it fetching and leaves the others recording", async () => {
    for (const x of [a, b, c]) await invoke(x, "party_on", { on: false });
    await invoke(a, "start_recording");
    const running = (await state(a)).active_set.id;
    await allSee([b, c], "recording the new set", within(LOG_SYNC_S * 2), (st) => st.active_set?.id === running);
    const logged = (x) => segsOf(logOf(x, ctx.room, x.id), running, "v").length;
    await waitFor("A and C log two segments", within(SEGMENT_S * 4), () => logged(a) >= 2 && logged(c) >= 2);
    const held = pieces(logOf(c, ctx.room, c.id), running);
    await waitFor("A and B hold C's segments so far", within((SEGMENT_S + SYNC_ROUND_S) * 2), () => ![a, b].some((x) => badCopies(x, ctx.room, c.id, held).length));
    await invoke(a, "remove_peer", { id: c.id });
    await allSee([a, b], "C no longer a peer", within(LOG_SYNC_S * 2), (st) => !st.peers.some((p) => p.id === c.id));
    await sleep(LOG_SYNC_S * 1000);
    const before = fileSize(logFile(c, ctx.room, a.id));
    const id = await invoke(a, "add_event", { kind: "note", text: "After the removal", target: {}, values: null });
    const key = `${a.id}/${id}`;
    await allSee([b], "the new note", within(LOG_SYNC_S * 2), (st) => st.events.some((e) => e.key === key));
    const more = { a: logged(a) + 2, b: logged(b) + 2 };
    await waitFor("A and B record on without C", within(SEGMENT_S * 4), () => logged(a) >= more.a && logged(b) >= more.b);
    check(fileSize(logFile(c, ctx.room, a.id)) === before, "C still received A's log after its removal");
    check(!(await state(c)).events.some((e) => e.key === key), "C shows a note written after its removal");
    for (const holder of [a, b]) check(!badCopies(holder, ctx.room, c.id, held).length, `${holder.key} lost C's segments after the removal`);
    await invoke(a, "stop_recording");
    await allSee([a, b], "the set ended", within(LOG_SYNC_S * 2), (st) => !st.active_set && setOf(st, running) && !setOf(st, running).live);
    await invoke(c, "stop_recording");
  });

  await step("B leaves the room from its menu, enters it again with its recordings and notes, and deletes it from the list, which forgets B's personal plan", async () => {
    const plan = JSON.stringify(`board/${ctx.room}`);
    await evaluate(b, `localStorage.setItem(${plan}, JSON.stringify("# Mine")); true`);
    const kept = (st) => ({ ids: [st.sets.map((s) => s.id), st.events.map((e) => e.key)].map((x) => x.sort().join()).join(), have: Object.fromEntries(st.sets.flatMap((s) => s.players.map((p) => [`${s.id} ${p.owner}`, p.have]))) });
    const before = kept(await state(b));
    await waitFor("B shows the room header", within(UI_REFRESH_S * 5), () => evaluate(b, `!$("app").hidden && !!st.room`));
    await evaluate(b, `$("roomname").click(); [...$("menu").querySelectorAll("button")].find((x) => x.textContent === t("Leave room")).click(); true`);
    await waitFor("B shows the start screen and still lists the room", within(UI_REFRESH_S * 5), async () => { const st = await state(b); return !st.room && st.rooms.some((r) => r.id === ctx.room) && evaluate(b, `!$("start").hidden && $("app").hidden`); });
    await invoke(b, "enter_room", { id: ctx.room });
    await meshed([a, b], ctx.room);
    const after = kept(await state(b));
    check(after.ids === before.ids && Object.entries(before.have).every(([k, n]) => after.have[k] >= n), "B lost recordings or notes by leaving and entering again");
    await invoke(b, "close_room", { id: ctx.room });
    await waitFor("B lists the room", within(UI_REFRESH_S * 5), () => evaluate(b, `!$("start").hidden && $("roomlist").children.length > 0`));
    await pressAndConfirm(b, `[...$("roomlist").children].find((r) => r.firstChild.textContent === st.rooms.find((x) => x.id === ${JSON.stringify(ctx.room)}).name).querySelector(".more").click(); [...$("menu").querySelectorAll("button")].find((x) => x.textContent === t("Delete…")).click()`);
    await waitFor("B no longer lists the room", within(UI_REFRESH_S * 5), async () => { const st = await state(b); return !st.room && !st.rooms.some((r) => r.id === ctx.room); });
    await waitFor("B forgets the personal plan of the room it deleted", within(UI_REFRESH_S * 5), async () => (await evaluate(b, `localStorage.getItem(${plan})`)) === null);
    await waitFor("B removes the folder of the room it left", within(UI_REFRESH_S * 5), () => !fs.existsSync(roomDir(b, ctx.room)));
  });

  await step("C removes the room from this PC while sharing and its folder does not come back", async () => {
    const dir = roomDir(c, ctx.room);
    check((await state(c)).capture.sharing && filesUnder(path.join(dir, "standby", c.id)).length > 0, "C shares no live picture before removing the room");
    await invoke(c, "close_room", { id: ctx.room });
    await invoke(c, "remove_room", { id: ctx.room });
    check(!(await state(c)).rooms.some((r) => r.id === ctx.room), "C still lists the room it removed");
    let since = 0;
    await waitFor("C's room folder gone for good", within(FINISH_WAIT_S + KEEP_EVERY_S * 3), () => {
      since = fs.existsSync(dir) ? 0 : since || Date.now();
      return since > 0 && Date.now() - since > (FINISH_WAIT_S + KEEP_EVERY_S) * 1000;
    });
    check(fs.existsSync(path.join(c.data, "rooms", OLD_ROOM)), "removing the room touched a folder of an older version");
  });
}

async function gather(ctx, extra) {
  const a = await launch("a");
  const b = await launch("b");
  const c = await launch("c", extra);
  ctx.room = await invoke(a, "create_room", { name: "Practice" });
  await asks(a, b, a);
  await invoke(a, "answer_ask", { id: b.id, approve: true });
  await meshed([a, b], ctx.room);
  await allSee([a, b], "encoding its test source", within(COLD_START_S), (st) => st.capture.live);
  await invoke(a, "start_recording");
  ctx.set = (await state(a)).active_set.id;
  await waitFor(`A has logged ${SEGMENTS_BEFORE_LATE_JOIN} segments`, within(SEGMENT_S * (SEGMENTS_BEFORE_LATE_JOIN + 1)), () => segsOf(logOf(a, ctx.room, a.id), ctx.set, "v").length >= SEGMENTS_BEFORE_LATE_JOIN);
  await asks(a, c, b);
  await invoke(a, "answer_ask", { id: c.id, approve: true });
  await meshed([a, b, c], ctx.room);
  await waitFor("C logs a picture segment of the running set", within(COLD_START_S), () => segsOf(logOf(c, ctx.room, c.id), ctx.set, "v").length > 0);
  return [a, b, c];
}

async function recover(ctx) {
  let a, b, c;
  await step("A, B and C record the same set, C joining late and reaching the others only through the relay", async () => {
    [a, b, c] = await gather(ctx, { EZH_RELAY_ONLY: "1" });
    await allSee([a, b], "C's path is the relay", within(DIAL_S), (st) => st.peers.find((p) => p.id === c.id)?.path === "relay");
    await allSee([b, c], "A's live picture available", within(COLD_START_S), (st) => st.peers.find((p) => p.id === a.id)?.live);
  });

  await step("B crashes mid-recording and recovers its open segment", async () => {
    const capture = segsOf(logOf(b, ctx.room, b.id), ctx.set, "v").at(-1).capture;
    const dir = path.join(roomDir(b, ctx.room), "recordings", ctx.set, b.id, capture);
    const openPart = (series) => {
      const name = filesUnder(path.join(dir, series)).map((f) => path.basename(f)).filter((f) => f.endsWith(".m4s.part")).sort().at(-1);
      return name && { series, n: Number(name.split(".")[0]), whole: wholeFragments(path.join(dir, series, name)) };
    };
    await waitFor(`B's open segment has ${CRASH_AFTER_FRAGMENTS} whole fragments`, within(SEGMENT_S * 2), () => {
      const v = openPart("v");
      return v && v.whole >= CRASH_AFTER_FRAGMENTS;
    });
    const standby = filesUnder(path.join(roomDir(b, ctx.room), "standby"));
    check(standby.length > 0, "B holds no live segments before the crash");
    kill(b, false);
    await stopped(b);
    const open = SERIES.map(openPart).filter((o) => o && o.whole > 0);
    check(open.some((o) => o.series === "v"), "no open picture segment at the crash");
    ctx.notes.push(`killed B with open segments ${brief(open)}`);
    start("b");
    await attach(b);
    const kept = standby.filter((f) => fs.existsSync(f));
    check(!kept.length, `B kept ${kept.length} live files over its restart`);
    const recovery = (await state(b)).recovery;
    check(!recovery, `B says ${brief(recovery)} after the crash`);
    await share(b);
    await waitFor("B logged its recovered segments and ended its part", within(COLD_START_S), () => {
      const own = logOf(b, ctx.room, b.id);
      const logged = open.every((o) => own.some((r) => r.k === "seg" && r.set === ctx.set && r.capture === capture && r.series === o.series && r.n === o.n));
      return logged && own.some((r) => r.k === "rec_end" && r.set === ctx.set);
    });
    const recovered = pieces(logOf(b, ctx.room, b.id), ctx.set).filter((p) => p.capture === capture && open.some((o) => o.series === p.series && segName(o.n) === p.name));
    check(!badCopies(b, ctx.room, b.id, recovered).length, "a recovered segment does not match its log record");
    const all = [a, b, c];
    await meshed(all);
    const logged = (x) => segsOf(logOf(x, ctx.room, x.id), ctx.set, "v").length;
    const target = logged(a) + SEGMENTS_AFTER_RESTART;
    await waitFor(`A logs ${SEGMENTS_AFTER_RESTART} more segments`, within(SEGMENT_S * (SEGMENTS_AFTER_RESTART + 1)), () => logged(a) >= target);
    const limits = { a: target, c: logged(c) };
    await waitFor("B holds them after its restart", within((SEGMENT_S + LOG_SYNC_S) * 2), () => !holdEverything([b], ctx.room, ctx.set, [a, c], limits));
    await recordsAgain(b, ctx.room, ctx.set, [capture]);
  });

  await step("B loses its network while everyone records, keeps recording its part, and everything reaches everyone once it is back", async () => {
    const all = [a, b, c];
    const logged = (x) => segsOf(logOf(x, ctx.room, x.id), ctx.set, "v").length;
    const before = { a: logged(a), b: logged(b) };
    marker(b, "offline", true);
    await allSee([a, c], "B gone", within(DIAL_S), (st) => !connected(st, [b.id]));
    await waitFor("B records on while cut off", within(OFFLINE_S + SEGMENT_S * 2), () => logged(b) >= before.b + OFFLINE_S / SEGMENT_S);
    check((await state(b)).active_set?.id === ctx.set && logged(a) >= before.a + OFFLINE_S / SEGMENT_S, "B or A stopped recording while B was cut off");
    marker(b, "offline", false);
    await meshed(all);
    const limits = Object.fromEntries(all.map((x) => [x.key, logged(x)]));
    await waitFor(`everyone holds what was logged by now (${brief(limits)})`, within((SEGMENT_S + SYNC_ROUND_S) * 4), () => !holdEverything(all, ctx.room, ctx.set, all, limits));
  });

  await step("A's encoder is killed and A keeps recording the set in a new part", async () => {
    const sink = (await invoke(a, "debug_view", { tab: "recording", since: 0, set: null })).engine.sink;
    check(sink && sink.pid, "A has no encoder");
    spawnSync("taskkill", ["/PID", String(sink.pid), "/F"], { stdio: "ignore" });
    const parts = () => segsOf(logOf(a, ctx.room, a.id), ctx.set, "v").filter((r) => r.capture !== sink.capture).length;
    await waitFor(`A logs ${SEGMENTS_AFTER_RESTART} segments of the set from a new encoder`, within(SEGMENT_S * (SEGMENTS_AFTER_RESTART + 2)), () => parts() >= SEGMENTS_AFTER_RESTART);
    const st = await state(a);
    check(st.active_set?.id === ctx.set && st.capture.working, `A is not recording the set again: ${brief(st.capture.status)}`);
  });

  await step("closing the captured window during the recording goes on with the voice, and a new screen adds a part", async () => {
    const before = segsOf(logOf(a, ctx.room, a.id), ctx.set).map((s) => s.capture);
    await evaluate(a, `openLive(); true`);
    await invoke(a, "set_source", { source: GONE });
    const voice = await waitFor("A records the set voice only in a new part", within(SEGMENT_S * 3), () => { const added = segsOf(logOf(a, ctx.room, a.id), ctx.set).filter((s) => !before.includes(s.capture)); return added.length && added.every((s) => s.series === "m") && added[0].capture; });
    const st = await state(a);
    check(st.capture.working && st.capture.source_lost && !st.capture.sharing, `A does not record only its voice: ${brief(st.capture)}`);
    await waitFor("A's own tile and Screen button say the source closed", within(UI_REFRESH_S * 5), () => evaluate(a, `$("screensub").textContent === t("Capture source closed") && rec.tiles[st.id].what.textContent === t("Capture source closed")`));
    await evaluate(a, `$("screenbtn").click(); true`);
    await waitFor("the Screen button offers the picker", within(UI_REFRESH_S * 5), () => evaluate(a, `!$("pop").hidden`));
    await evaluate(a, `closePop(); true`);
    await invoke(a, "set_source", { source: SOURCE });
    await recordsAgain(a, ctx.room, ctx.set, [...before, voice]);
    await waitFor("B holds A's voice-only part", within(SEGMENT_S + LOG_SYNC_S * 2), () => !badCopies(b, ctx.room, a.id, pieces(logOf(a, ctx.room, a.id), ctx.set).filter((p) => p.capture === voice)).length);
  });

  await step("C's disk is full during the recording: its part ends with a notice, the others go on, and C completes its copy once there is room", async () => {
    const logged = (x) => segsOf(logOf(x, ctx.room, x.id), ctx.set).length;
    marker(c, "full", true);
    await waitFor("C ends its part and says the disk is almost full", within(KEEP_EVERY_S * 3), async () => lastJoin(c, ctx.room, ctx.set)?.k === "rec_end" && (await state(c)).notice.includes("disk is almost full"));
    check((await state(a)).active_set?.id === ctx.set && (await state(b)).active_set?.id === ctx.set, "A or B stopped recording when C's disk filled");
    const refused = await invoke(c, "start_recording").then(() => "", (e) => String(e));
    check(refused.includes("disk is almost full"), `C was not refused a start: "${refused}"`);
    const held = () => pieces(logOf(a, ctx.room, a.id), ctx.set).filter((p) => fs.existsSync(mediaFile(c, ctx.room, p.set, a.id, p.capture, p.series, p.name))).length;
    await sleep(SYNC_ROUND_S * 1000);
    const stuck = held();
    const target = logged(a) + 2;
    await waitFor("A logs two more segments", within(SEGMENT_S * 3), () => logged(a) >= target);
    await sleep(SYNC_ROUND_S * 1000);
    check(held() === stuck, "C received files although its disk was full");
    marker(c, "full", false);
    const limits = Object.fromEntries([a, b].map((x) => [x.key, segsOf(logOf(x, ctx.room, x.id), ctx.set, "v").length]));
    await waitFor("C completes its copy of A and B", within((SEGMENT_S + SYNC_ROUND_S) * 4), () => !holdEverything([c], ctx.room, ctx.set, [a, b], limits));
  });

  await step("B leaving ends only its part and rejoins, a crashed C does not keep the stopped set open, and everyone has it", async () => {
    const all = [a, b, c];
    const before = segsOf(logOf(b, ctx.room, b.id), ctx.set).map((s) => s.capture);
    await invoke(b, "close_room", { id: ctx.room });
    check(lastJoin(b, ctx.room, ctx.set)?.stop === false, "B did not end only its own part");
    await allSee([a, c], "the recording goes on without B", within(UNREACHABLE_S + SYNC_ROUND_S), (st) => st.active_set?.id === ctx.set && setOf(st, ctx.set).live && setOf(st, ctx.set).players.find((p) => p.owner === b.id).recording === false);
    await sleep(SYNC_ROUND_S * 1000);
    await allSee([a, c], "still recording", 1, (st) => st.active_set?.id === ctx.set);
    await invoke(b, "enter_room", { id: ctx.room });
    await recordsAgain(b, ctx.room, ctx.set, before);
    await meshed([a, b]);
    await allSee([a], "B recording again", within(SEGMENT_S + LOG_SYNC_S * 2), (st) => setOf(st, ctx.set).players.find((p) => p.owner === b.id).recording);
    kill(c);
    await stopped(c);
    await invoke(a, "stop_recording");
    const lastOf = (app) => Math.max(...segsOf(logOf(app, ctx.room, c.id), ctx.set).map((s) => s.t_us + s.d_us));
    await allSee([a, b], "the set shown ended at once, not before C's last segment", within(LOG_SYNC_S * 2), (st, app) => { const set = setOf(st, ctx.set); return !st.active_set && set && !set.live && set.end_us >= lastOf(app) - set.players.find((p) => p.owner === c.id).offset_us; });
    const ended = setOf(await state(a), ctx.set);
    ctx.durationS = (ended.end_us - ended.start_us) / 1e6;
    start("c");
    await attach(c);
    await share(c);
    await waitFor("C ends its part on restart", within(COLD_START_S), () => lastJoin(c, ctx.room, ctx.set)?.k === "rec_end");
    await meshed(all);
    await allSee(all, "the set ended", within(LOG_SYNC_S * 2), (st) => !st.active_set && setOf(st, ctx.set) && !setOf(st, ctx.set).live);
    await allSee(all, "every live picture back after the stop", within(COLD_START_S), (st) => st.capture.live && st.peers.length === 2 && st.peers.every((p) => p.live));
    await allSee(all, "Everyone has it", within((SEGMENT_S + SYNC_ROUND_S) * 2), (st) => heldByAll(st, ctx.set, all));
    const st = await state(a);
    const players = setOf(st, ctx.set).players.map((p) => p.owner).sort();
    check(players.join() === all.map((x) => x.id).sort().join(), `players are ${brief(players)}`);
    for (const holder of all) {
      for (const owner of all) {
        const theirs = segsOf(logOf(owner, ctx.room, owner.id), ctx.set).length;
        const copy = segsOf(logOf(holder, ctx.room, owner.id), ctx.set).length;
        check(copy === theirs, `${holder.key} has ${copy} of ${owner.key}'s ${theirs} segment records`);
      }
    }
    const missing = holdEverything(all, ctx.room, ctx.set, all);
    check(!missing, missing);
    const count = all.reduce((n, owner) => n + pieces(logOf(owner, ctx.room, owner.id), ctx.set).length, 0);
    ctx.notes.push(`verified ${count} pieces by SHA-256 on each of ${all.length} members, set of ${ctx.durationS.toFixed(1)} s`);
  });
}

async function library(ctx) {
  let a, b, c;
  await step("A, B and C record a short set, C joining late", async () => {
    [a, b, c] = await gather(ctx);
    const all = [a, b, c];
    await waitFor(`A has logged ${SHORT_SET_SEGMENTS} segments`, within(SEGMENT_S * SHORT_SET_SEGMENTS), () => segsOf(logOf(a, ctx.room, a.id), ctx.set, "v").length >= SHORT_SET_SEGMENTS);
    await invoke(a, "stop_recording");
    await allSee(all, "the set ended and everyone has it", within((SEGMENT_S + SYNC_ROUND_S) * 3), (st) => !st.active_set && setOf(st, ctx.set) && !setOf(st, ctx.set).live && heldByAll(st, ctx.set, all));
    const ended = setOf(await state(a), ctx.set);
    ctx.durationS = (ended.end_us - ended.start_us) / 1e6;
  });

  await step("each ended recording gets a thumbnail, and the room page lists recordings as cards", async () => {
    await waitFor("A makes the thumbnail of the set", within(30), async () => setOf(await state(a), ctx.set)?.thumb === true);
    const size = await evaluate(a, `new Promise((done) => { const i = new Image(); i.onload = () => done([i.naturalWidth, i.naturalHeight]); i.onerror = () => done(null); i.src = thumbSrc(setById(${JSON.stringify(ctx.set)})); })`);
    check(size && size[0] > 0 && size[1] > 0, `the page could not load the thumbnail: ${brief(size)}`);
    const file = fs.readFileSync(path.join(roomDir(a, ctx.room), "thumbs", `${ctx.set}.jpg`));
    check(file[0] === 0xff && file[1] === 0xd8 && file.length > 500, "the thumbnail file is not a JPEG");
    await evaluate(a, `$("recsopen").click(); true`);
    await waitFor("A lists the set as a card with a picture", within(UI_REFRESH_S * 5), () => evaluate(a, `[...$("roomtab").querySelectorAll(".reccard")].some((c) => c.querySelector("img") && c.querySelectorAll(".acts button").length === 2) && $("homename").textContent === t("Recordings") && $("recsopen").getAttribute("aria-current") === "page"`));
    check(await evaluate(a, `!!$("sessions").querySelector(".srow:not(.liverow) .thumb img") && !$("allrecs") && $("sessions").querySelectorAll(".srow:not(.liverow)").length === recordingRows().length && !!$("recfilter")`), "the sidebar does not list every recording with a thumbnail, or the filter is not in the main area");
    await evaluate(a, `$("recfilter").value = "zzzz"; $("recfilter").dispatchEvent(new Event("input")); true`);
    check(await evaluate(a, `$("roomtab").querySelectorAll(".reccard").length === 0`), "the filter did not hide the cards");
    await evaluate(a, `$("recfilter").value = ""; $("recfilter").dispatchEvent(new Event("input")); true`);
    await waitFor("the cards come back", within(UI_REFRESH_S * 3), () => evaluate(a, `$("roomtab").querySelectorAll(".reccard").length > 0`));
    await screenshot(a, "recordings-tab");
    await evaluate(a, `$("sidenav").querySelector("[data-nav=members]").click(); true`);
    await waitFor("A shows the members", within(UI_REFRESH_S * 3), () => evaluate(a, `$("homename").textContent === t("Members") && navActive() === "members"`));
    await screenshot(a, "room-members");
  });

  await step("an Apex matches job for the set is claimed once and finished", async () => {
    const all = [a, b, c];
    const sizes = Object.fromEntries(all.map((owner) => [owner.key, fileSize(logFile(owner, ctx.room, owner.id))]));
    await waitFor("every member pulls every log again", within(REQUEST_TIMEOUT_S + DIAL_S + LOG_SYNC_S), () => all.every((holder) => all.every((owner) => fileSize(logFile(holder, ctx.room, owner.id)) >= sizes[owner.key])));
    await invoke(a, "set_room_plugin", { id: PLUGIN, on: true, settings: {} });
    const seconds = within(JOB_CHECK_S + LOG_SYNC_S + ctx.durationS / KEYFRAME_DECODE_SPEED);
    await Promise.all(all.map((app) => waitFor(`${app.key}: the set's one job done`, seconds, async () => {
      const jobs = (await invoke(app, "jobs")).jobs.filter((j) => j.plugin === PLUGIN && j.set === ctx.set);
      return jobs.length === 1 && jobs[0].state === "done";
    })));
    await sleep(LOG_SYNC_S * 2 * 1000);
    const runs = all.flatMap((owner) => logOf(a, ctx.room, owner.id).filter((r) => r.k === "job" && r.plugin === PLUGIN).map((r) => ({ ...r, by: owner.key })));
    for (const key of new Set(runs.map((r) => r.job))) {
      const yielded = runs.filter((r) => r.job === key && r.state === "yield").map((r) => `${r.by}/${r.run}`);
      const claims = runs.filter((r) => r.job === key && r.state === "claim" && !yielded.includes(`${r.by}/${r.run}`));
      const done = runs.filter((r) => r.job === key && r.state === "done");
      check(claims.length === 1 && done.length === 1 && claims[0].run === done[0].run, `job ${key}: ${claims.length} claims kept, ${yielded.length} yielded, ${done.length} done`);
      check(done[0].events === 0, `job ${key} found ${done[0].events} matches in the test picture`);
    }
    check(new Set(runs.map((r) => r.job)).size === 1, `the set had ${new Set(runs.map((r) => r.job)).size} Apex jobs`);
    ctx.notes.push(`one plugin job run once (${runs.filter((r) => r.state === "yield").length} later claims yielded)`);
    const shown = (await state(b)).events.filter((e) => e.plugin);
    check(!shown.length, `plugin events shown: ${brief(shown)}`);
  });

  await step("the transcription plugin follows the chosen model, with one job row per person", async () => {
    await invoke(b, "set_room_plugin", { id: "transcribe", on: true, settings: {} });
    await evaluate(b, `openPlugin = "transcribe"; roomTab = "plugins"; go("room"); pollPlugins(); true`);
    const shown = `[...$("roomtab").querySelectorAll(".plug")].find((p) => p.querySelector(".pt b").textContent === t("Transcription"))`;
    for (const [model, size] of Object.entries(MODEL_BYTES)) {
      await waitFor("B shows the transcription settings", within(UI_REFRESH_S * 5), () => evaluate(b, `!!${shown} && !!${shown}.querySelector(".field select")`));
      await evaluate(b, `(() => { const s = ${shown}.querySelector(".field select"); s.value = ${JSON.stringify(model)}; s.dispatchEvent(new Event("change")); return true; })()`);
      await waitFor("B asks before changing the room", within(UI_REFRESH_S * 5), () => evaluate(b, `$("confirm").open`));
      const said = await evaluate(b, `$("confirmtext").textContent`);
      check(said === "Applies to everyone in the room.", `the change says "${said}" instead of that it applies to everyone`);
      await evaluate(b, `$("confirmyes").click(); true`);
      const text = bytes(size + WHISPER_BYTES);
      await waitFor(`B's card offers ${text}`, within(LOG_SYNC_S * 2), () => evaluate(b, `${shown}.textContent.includes(${JSON.stringify(text)})`));
      for (const app of [b, a]) {
        await waitFor(`${app.key} offers the ${model} model`, within(LOG_SYNC_S * 2), async () => {
          const p = (await invoke(app, "room_plugins")).find((x) => x.id === "transcribe");
          return p.settings.model === model && p.download_bytes === size + WHISPER_BYTES;
        });
      }
    }
    const members = await evaluate(b, `${shown}.querySelectorAll(".wrow").length`);
    check(members === 3, `B lists ${members} members under Processing`);
    const choices = await evaluate(b, `[...${shown}.querySelectorAll("select[aria-label='" + t("Who runs it") + "'] option")].map((o) => o.textContent).join()`);
    check(choices.split(",").sort().join() === ["Auto", a.name, b.name, c.name].sort().join(), `the runner choices are ${choices}`);
    await invoke(b, "dismiss_recovery");
    await screenshot(b, "plugins");
    await evaluate(b, `(() => { const s = ${shown}.querySelector("select[aria-label='" + t("Who runs it") + "']"); s.value = ${JSON.stringify(c.id)}; s.dispatchEvent(new Event("change")); return true; })()`);
    await waitFor("B asks before changing who runs it", within(UI_REFRESH_S * 5), () => evaluate(b, `$("confirm").open`));
    await evaluate(b, `$("confirmyes").click(); true`);
    for (const app of [a, b, c]) await waitFor(`${app.key} sees C as the runner`, within(LOG_SYNC_S * 3), async () => (await invoke(app, "room_plugins")).find((x) => x.id === "transcribe").runner === c.id && (await invoke(app, "jobs")).jobs.filter((j) => j.plugin === "transcribe").every((j) => j.runner === c.id));
    await evaluate(b, `[...$("roomtab").querySelectorAll(".seg button")].find((x) => x.firstChild.textContent === t("Jobs")).click(); true`);
    await waitFor("B names C's PC in the jobs list", within(UI_REFRESH_S * 5), () => evaluate(b, `[...$("roomtab").querySelectorAll("tbody tr:not(.err)")].filter((r) => r.children[0].textContent.includes(t("Transcription"))).length > 0 && [...$("roomtab").querySelectorAll("tbody tr:not(.err)")].filter((r) => r.children[0].textContent.includes(t("Transcription"))).every((r) => r.children[4].textContent === t("{name}'s PC", { name: ${JSON.stringify(c.name)} }))`));
    await screenshot(b, "jobs");
    await invoke(b, "set_plugin_runner", { id: "transcribe", runner: null });
    for (const app of [a, b, c]) await waitFor(`${app.key} sees Auto again`, within(LOG_SYNC_S * 3), async () => (await invoke(app, "room_plugins")).find((x) => x.id === "transcribe").runner === null);
    const jobs = (await invoke(b, "jobs")).jobs.filter((j) => j.plugin === "transcribe" && j.set === ctx.set);
    const owners = jobs.map((j) => j.owner);
    check(owners.length > 0 && new Set(owners).size === owners.length, `transcription rows per person in the set: ${brief(owners)}`);
  });

  await step("a member turns processing off for one plugin and everyone sees it", async () => {
    await invoke(c, "set_process_plugin", { id: "transcribe", on: false });
    for (const app of [a, b, c]) {
      await waitFor(`${app.key} sees C not processing`, within(LOG_SYNC_S * 3), async () => (await invoke(app, "room_plugins")).find((x) => x.id === "transcribe").workers.some((w) => w.id === c.id && w.on === false));
    }
    check((await invoke(c, "room_plugins")).find((x) => x.id === PLUGIN).process_here === true, "turning off one plugin turned off another");
    await invoke(a, "set_process_plugin", { id: "transcribe", on: false });
    for (const app of [a, b, c]) await waitFor(`${app.key} names B as the only one who runs the transcription`, within(LOG_SYNC_S * 3), async () => {
      const jobs = (await invoke(app, "jobs")).jobs.filter((j) => j.plugin === "transcribe");
      return jobs.length > 0 && jobs.every((j) => j.runner === b.id);
    });
    await invoke(a, "set_process_plugin", { id: "transcribe", on: true });
    await invoke(c, "set_process_plugin", { id: "transcribe", on: true });
    for (const app of [a, b, c]) await waitFor(`${app.key} sees nobody named when two process`, within(LOG_SYNC_S * 3), async () => (await invoke(app, "jobs")).jobs.filter((j) => j.plugin === "transcribe").every((j) => j.runner === null));
    await invoke(b, "set_room_plugin", { id: "transcribe", on: false, settings: {} });
  });

  await step("the room header menu lists the room commands and a rename in its settings reaches everyone", async () => {
    await evaluate(a, `$("sidenav").querySelector("[data-nav=board]").click(); true`);
    await waitFor("A shows the game plan in the main area", within(UI_REFRESH_S * 3), () => evaluate(a, `!$("boardpane").hidden && !$("roompage").hidden && navActive() === "board"`));
    for (const [by, name] of [[b, "Drill room"], [a, "Practice"]]) {
      await invoke(by, "rename_room", { name });
      for (const app of [a, b, c]) await waitFor(`${app.key} shows ${name}`, within(LOG_SYNC_S * 2), async () => (await state(app)).room.name === name);
    }
    await waitFor("the sidebar header shows the room name as a menu", within(UI_REFRESH_S * 5), () => evaluate(a, `$("roomname").firstElementChild.textContent === "Practice" && $("roomname").getAttribute("aria-haspopup") === "menu"`));
    await evaluate(a, `$("roomname").click(); true`);
    const labels = await evaluate(a, `[...$("menu").querySelectorAll("button")].map((b) => b.textContent)`);
    check(labels.filter((l) => l !== "Cancel invite").join() === ["Room settings…", "Copy invite code", "Leave room"].join(), `the room menu lists ${labels}`);
    await evaluate(a, `[...$("menu").querySelectorAll("button")].find((b) => b.textContent === t("Room settings…")).click(); true`);
    await waitFor("A shows the room settings", within(UI_REFRESH_S * 3), () => evaluate(a, `$("roomdlg").open && $("roomnameinput").value === "Practice" && $("roomsave").disabled`));
    await screenshot(a, "room-settings");
    await evaluate(a, `(() => { const i = $("roomnameinput"); i.value = "Scrim room"; i.dispatchEvent(new Event("input")); $("roomsave").click(); return true; })()`);
    for (const app of [a, b, c]) await waitFor(`${app.key} shows the name set in the room settings`, within(LOG_SYNC_S * 2), async () => (await state(app)).room.name === "Scrim room");
    await waitFor("A's header shows the new name", within(UI_REFRESH_S * 3), () => evaluate(a, `$("roomname").firstElementChild.textContent === "Scrim room"`));
    await invoke(a, "rename_room", { name: "Practice" });
    for (const app of [a, b, c]) await waitFor(`${app.key} shows Practice again`, within(LOG_SYNC_S * 2), async () => (await state(app)).room.name === "Practice");
  });

  await step("the game plan renders Markdown safely, ticks tasks, and keeps a personal plan on this PC", async () => {
    const plan = ["# Plan", "**Goal**: win", "## Checklist", "- [ ] first", "- [x] second", "## Table", "| Who | Job |", "|---|---|", "| Ann | scout |", "## Code", "```", "let ring = 1;", "```", "## Moments", `[Opening](ezh://moment/${encodeURIComponent(ctx.set)}/5/all) [site](https://example.com/x) [bad](javascript:alert(1)) <img src=x onerror=alert(1)>`].join("\n");
    await evaluate(a, `editBoard("room"); $("boardtext").value = ${JSON.stringify(plan)}; renderMini(); saveBoard(); true`);
    for (const app of [b, c]) await waitFor(`${app.key} receives the game plan`, within(LOG_SYNC_S * 2), async () => (await state(app)).events.some((e) => e.kind === "board" && e.text === plan));
    await waitFor("A shows the cards", within(UI_REFRESH_S * 5), () => evaluate(a, `!!$("canvas").querySelector(".bsec table") && !!$("canvas").querySelector(".bsec pre code")`));
    const dom = await evaluate(a, `(() => { const c = $("canvas"); return { cards: c.querySelectorAll(".bsec").length, boxes: c.querySelectorAll("input[type=checkbox]").length, links: [...c.querySelectorAll("a")].map((x) => [x.getAttribute("href"), x.title]), moments: c.querySelectorAll("button.link").length, images: c.querySelectorAll("img").length, text: c.textContent }; })()`);
    check(dom.cards === 4 && dom.boxes === 2 && dom.moments === 1 && dom.images === 0 && dom.links.length === 1 && dom.links[0][0] === null && dom.links[0][1] === "https://example.com/x", `the game plan shows ${brief(dom)}`);
    check(dom.text.includes("javascript:alert(1)") && dom.text.includes("<img src=x onerror=alert(1)>"), "the unsafe link and tag are not shown as plain text");
    await screenshot(a, "board-room");
    await evaluate(a, `(() => { const [one, two] = $("canvas").querySelectorAll("input[type=checkbox]"); one.click(); two.click(); return true; })()`);
    const ticked = plan.replace("- [ ] first", "- [x] first").replace("- [x] second", "- [ ] second");
    for (const app of [a, b, c]) await waitFor(`${app.key} sees both quick ticks`, within(LOG_SYNC_S * 2 + UI_REFRESH_S * 2), async () => (await evaluate(app, `boardEvent() && boardEvent().text`)) === ticked);
    const boards = async (app) => (await state(app)).events.filter((e) => e.kind === "board").length;
    const before = { a: await boards(a), b: await boards(b), c: await boards(c) };
    await evaluate(a, `setBoardScope("personal"); $("boardedit").click(); true`);
    await waitFor("A's Edit button opens the personal plan", within(UI_REFRESH_S * 3), () => evaluate(a, `boardEditing && boardScope === "personal" && $("boardtext").value === boardText() && $("boardtext").value !== boardEvent().text && $("boardscope").querySelector("[data-scope=personal]").getAttribute("aria-pressed") === "true"`));
    await evaluate(a, `$("boardtext").value = "# Mine\\n- [ ] only me"; renderMini(); saveBoard(); true`);
    await waitFor("A shows the personal plan", within(UI_REFRESH_S * 5), () => evaluate(a, `boardScope === "personal" && $("canvas").textContent.includes("only me") && $("homesub").textContent === t("Only on this PC")`));
    await screenshot(a, "board-personal");
    await sleep(SYNC_ROUND_S * 1000);
    for (const app of [a, b, c]) {
      check((await boards(app)) === before[app.key] && !JSON.stringify((await state(app)).events).includes("only me"), `${app.key} has the personal game plan as an event`);
      check(!fs.readFileSync(logFile(app, ctx.room, app.id)).includes("only me"), `${app.key} wrote the personal plan to its log`);
    }
    await evaluate(a, `setBoardScope("room"); true`);
    await waitFor("A shows the room plan again", within(UI_REFRESH_S * 5), () => evaluate(a, `boardScope === "room" && $("canvas").textContent.includes("Checklist")`));
    await evaluate(a, `writeBoard(" "); true`);
    await waitFor("A shows the starter page for a cleared room plan", within(UI_REFRESH_S * 5), () => evaluate(a, `!boardEvent() && boardDraft === boardText() && $("canvas").querySelectorAll(".bsec").length === 3 && !$("homesub").textContent`));
    await evaluate(a, `$("sidenav").querySelector("[data-nav=members]").click(); true`);
  });

  await step("the sidebar width is dragged, kept, and never scrolls sideways", async () => {
    const over = (width) => evaluate(a, `(() => { setSideWidth(${width}); const side = $("side"); const box = side.getBoundingClientRect(); return { width: Math.round(box.width), scroll: [side, $("sessions"), $("sidenav"), document.querySelector(".dock")].filter((e) => e.scrollWidth > e.clientWidth + 1).map((e) => e.className || e.id), outside: [...side.querySelectorAll("*")].filter((e) => e.getBoundingClientRect().right > box.right + 1 && !e.hidden).map((e) => e.className || e.tagName).slice(0, 5) }; })()`);
    const limits = await evaluate(a, `[clampSide(0), clampSide(9999)]`);
    for (const width of [limits[0], 264, limits[1]]) {
      const r = await over(width);
      check(r.width === width && !r.scroll.length && !r.outside.length, `the sidebar at ${width}: ${brief(r)}`);
    }
    await invoke(a, "rename_room", { name: "W".repeat(ROOM_NAME_CHARS) });
    await waitFor("A's header truncates the long name", within(LOG_SYNC_S * 2), () => evaluate(a, `(() => { const n = $("roomname").firstElementChild; return n.textContent.length === ${ROOM_NAME_CHARS} && n.scrollWidth > n.clientWidth && getComputedStyle(n).textOverflow === "ellipsis" && $("side").scrollWidth <= $("side").clientWidth + 1; })()`));
    await screenshot(a, "sidebar-narrow");
    await invoke(a, "rename_room", { name: "Practice" });
    await evaluate(a, `setSideWidth(${264}); true`);
    const bar = await evaluate(a, `(() => { const r = $("splitter").getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; })()`);
    const mouse = (type, [x, y], buttons) => send(a, "Input.dispatchMouseEvent", { type, x, y, button: type === "mouseMoved" && !buttons ? "none" : "left", buttons, clickCount: 1 });
    await mouse("mouseMoved", bar, 0);
    await mouse("mousePressed", bar, 1);
    for (let i = 1; i <= 4; i++) await mouse("mouseMoved", [bar[0] + 15 * i, bar[1]], 1);
    await mouse("mouseReleased", [bar[0] + 60, bar[1]], 0);
    await waitFor("dragging the splitter widens the sidebar and keeps the width", within(UI_REFRESH_S * 3), () => evaluate(a, `Math.round($("side").getBoundingClientRect().width) === 324 && localStorage.getItem("sideWidth") === "324"`));
    await evaluate(a, `$("splitter").dispatchEvent(new MouseEvent("dblclick", { bubbles: true })); true`);
    check(await evaluate(a, `Math.round($("side").getBoundingClientRect().width) === 264`), "a double click on the splitter does not reset the width");
    check(await evaluate(a, `(() => { const r = $("sessions").getBoundingClientRect(); const hit = document.elementFromPoint(r.right - 6, r.top + 10); return $("sessions").contains(hit) && document.elementFromPoint(r.right + 1, r.top + 10) === $("splitter"); })()`), "the splitter covers the recordings list's scrollbar");
  });

  await step("each member row says one short state", async () => {
    const rows = await evaluate(a, `people().map((id) => memberOf(id).status.text)`);
    check(await evaluate(a, `(() => { const e = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 60, clientY: 60 }); document.body.dispatchEvent(e); return e.defaultPrevented && $("menu").hidden; })()`), "the browser's context menu is not blocked");
    await evaluate(a, `openSettings("general"); true`);
    check(await evaluate(a, `(() => { $("name").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 300 })); const labels = [...$("menu").querySelectorAll("button")].map((b) => b.textContent).join(); closeMenu(); $("settings").close(); return labels === [t("Cut"), t("Copy"), t("Paste"), t("Select all")].join(); })()`), "a text field has no edit menu");
    check(await evaluate(a, `!$("sessions").querySelector(".liverow") && !$("standby").hidden && $("standby").textContent.includes(t("Standby"))`), "the idle list holds a live entry or the start box lacks Standby");
    await evaluate(a, `openLive(); true`);
    await waitFor("A shows the live tiles while idle", within(UI_REFRESH_S * 10), () => evaluate(a, `!$("livetiles").hidden && Object.keys(rec.tiles).length === 3 && navActive() === "live" && $("psub").textContent === ""`));
    await screenshot(a, "live-idle");
    check(rows.length === 3 && rows.every((x) => x === "Standby" || /^Receiving \d+%$/.test(x)), `member states ${brief(rows)}`);
  });

  await step("Watch Party converges on a seek and a play, and a member who joins midway converges too", async () => {
    const all = [a, b, c];
    const seconds = within(PARTY_REPEAT_S + UI_REFRESH_S * 2);
    const follow = async (from, others, playing, position) => {
      const shared = await waitFor(`${from.key} shares its action`, seconds, async () => {
        const mine = (await state(from)).party.shared;
        return mine.origin === from.id && mine.set === ctx.set && mine.playing === playing && mine;
      });
      await Promise.all(others.map((app) => waitFor(`${app.key} follows ${from.key}`, seconds, async () => {
        const mine = (await state(app)).party.shared;
        const same = mine.seq === shared.seq && mine.stamp === shared.stamp && mine.origin === shared.origin;
        return same && evaluate(app, `selected === ${JSON.stringify(ctx.set)} && clock.playing === ${playing} && Math.abs(nowM() - ${position}) < ${JUMP_MIN_S}`);
      })));
    };
    await evaluate(c, `setParty(false); true`);
    await waitFor("C leaves the party", seconds, async () => !(await state(c)).party.on);
    await evaluate(a, `openSet(${JSON.stringify(ctx.set)}); act("seek", ${SEEK_TO_S}); true`);
    await follow(a, [b], false, SEEK_TO_S);
    await evaluate(b, `act("play", nowM()); true`);
    await follow(b, [a], true, "nowM()");
    check(await evaluate(c, `!clock.playing`), "C moved with a party it had left");
    await evaluate(c, `setParty(true); true`);
    await follow(b, [c], true, "nowM()");
    const clocks = await Promise.all(all.map((app) => evaluate(app, "nowM()")));
    check(Math.max(...clocks) - Math.min(...clocks) < JUMP_MIN_S, `clocks differ: ${brief(clocks)}`);
    check(Math.min(...clocks) >= SEEK_TO_S, `played from before the seek: ${brief(clocks)}`);
    await evaluate(a, `act("pause", nowM()); true`);
    await invoke(b, "party_typing", { on: true });
    await waitFor("A sees B typing", seconds, async () => (await state(a)).party.typing.includes(b.id));
    await invoke(b, "party_typing", { on: false });
    await evaluate(a, `window.e2eDraw = null; window.__TAURI__.event.listen("draw", (e) => { window.e2eDraw = e.payload; }).then(() => true)`);
    await invoke(b, "draw", { stroke: { clear: true, set: ctx.set } });
    await waitFor("A receives B's stroke", seconds, async () => (await evaluate(a, "window.e2eDraw"))?.from === b.id);
  });

  await step("dragging a name label swaps two tiles", async () => {
    await evaluate(a, `openSession(${JSON.stringify(ctx.set)}); true`);
    await waitFor("A shows the tiles", within(UI_REFRESH_S * 5), () => evaluate(a, `page === "player" && play.keys.length >= 2 && play.keys.every((k) => play.tiles[k] && play.tiles[k].box.getBoundingClientRect().width > 0)`));
    const before = await evaluate(a, `play.keys.join()`);
    const pts = await evaluate(a, `(() => { const [k0, k1] = play.keys; const l = play.tiles[k0].lab.getBoundingClientRect(); const t = play.tiles[k1].box.getBoundingClientRect(); return { from: [l.left + l.width / 2, l.top + l.height / 2], to: [t.left + t.width / 2, t.top + t.height / 2] }; })()`);
    const mouse = (type, [x, y], buttons) => send(a, "Input.dispatchMouseEvent", { type, x, y, button: type === "mouseMoved" && !buttons ? "none" : "left", buttons, clickCount: 1 });
    await mouse("mouseMoved", pts.from, 0);
    await mouse("mousePressed", pts.from, 1);
    for (let i = 1; i <= 5; i++) await mouse("mouseMoved", [pts.from[0] + ((pts.to[0] - pts.from[0]) * i) / 5, pts.from[1] + ((pts.to[1] - pts.from[1]) * i) / 5], 1);
    await mouse("mouseReleased", pts.to, 0);
    const [x0, x1, ...rest] = before.split(",");
    await waitFor("dragging a label swaps the two tiles", within(UI_REFRESH_S * 3), () => evaluate(a, `play.keys.join() === ${JSON.stringify([x1, x0, ...rest].join())}`));
    await screenshot(a, "player-swapped");
    await evaluate(a, `go("room"); true`);
  });

  await step("a session started while watching together records voices and the Watch Party", async () => {
    const all = [a, b, c];
    await allSee(all, "watching the set together", within(PARTY_REPEAT_S + UI_REFRESH_S * 2), (st) => st.party.on && st.party.shared.set === ctx.set);
    check(await evaluate(a, `startState() === "review" && $("startbtn").textContent === t("Record debrief")`), "A's start button does not offer to record the debrief");
    await evaluate(a, `$("startmore").click(); true`);
    check(await evaluate(a, `[...$("menu").querySelectorAll("button")].map((b) => b.textContent).join() === [t("Start recording"), t("Record debrief")].join()`), "the record menu does not offer Start recording and Record debrief");
    await evaluate(a, `closeMenu(); true`);
    await evaluate(b, `window.e2eNotes = []; window.__TAURI__.event.listen("session", (e) => e2eNotes.push(sessionNotice(e.payload.action, e.payload.by))).then(() => true)`);
    await pressAndConfirm(a, "toggleRecording()", false);
    await sleep(UI_REFRESH_S * 1000);
    check(!(await state(a)).active_set, "A started recording although it answered No");
    const asked = await pressAndConfirm(a, "toggleRecording()");
    check(asked === "Record the debrief for everyone?" && a.confirmText === "Only the voice and the viewing flow are recorded.", `A was asked "${asked}" / "${a.confirmText}"`);
    await waitFor("B is told who started", within(LOG_SYNC_S * 2), () => evaluate(b, `e2eNotes.includes(${JSON.stringify(a.name + " started recording")})`));
    ctx.review = (await state(a)).active_set.id;
    await allSee(all, "recording the review voice only", within(LOG_SYNC_S * 2), (st) => st.active_set?.id === ctx.review && st.active_set.voice === true);
    await waitFor(`A logs ${SEGMENTS_AFTER_RESTART} voice segments of the review`, within(SEGMENT_S * (SEGMENTS_AFTER_RESTART + 2)), () => segsOf(logOf(a, ctx.room, a.id), ctx.review, "m").length >= SEGMENTS_AFTER_RESTART);
    await evaluate(c, `act("seek", ${SEEK_TO_S * 2}); true`);
    await waitFor("C logs its seek as a party record", within(LOG_SYNC_S * 2), () => logOf(c, ctx.room, c.id).some((r) => r.k === "party" && r.set === ctx.review && r.shared.set === ctx.set));
    const ending = await pressAndConfirm(a, "toggleRecording()");
    check(ending === "Stop recording for everyone?", `A was asked "${ending}"`);
    await waitFor("A offers to discard the short debrief", within(UI_REFRESH_S * 5), () => evaluate(a, `$("confirm").open && $("confirmtitle").textContent === t("Discard this recording?")`));
    await evaluate(a, `$("confirmno").click(); true`);
    await waitFor("B is told who ended", within(LOG_SYNC_S * 2), () => evaluate(b, `e2eNotes.includes(${JSON.stringify(a.name + " stopped recording")})`));
    await allSee(all, "the review ended", within(LOG_SYNC_S * 2), (st) => !st.active_set && setOf(st, ctx.review) && !setOf(st, ctx.review).live);
    for (const app of all) {
      const own = logOf(app, ctx.room, app.id);
      check(segsOf(own, ctx.review, "m").length > 0 && segsOf(own, ctx.review, "v").length === 0, `${app.key} recorded ${segsOf(own, ctx.review, "v").length} picture and ${segsOf(own, ctx.review, "m").length} voice segments in the review`);
    }
    check(logOf(a, ctx.room, a.id).some((r) => r.k === "party" && r.set === ctx.review), "A did not log what was watched when the review started");
    await allSee(all, "two party records in the review", within(SYNC_ROUND_S * 2), (st) => (setOf(st, ctx.review)?.party || []).length >= 2);
    await waitFor("A sees a review session", within(UI_REFRESH_S * 3), () => evaluate(a, `isReview(setById(${JSON.stringify(ctx.review)}))`));
    await evaluate(a, `openSession(${JSON.stringify(ctx.review)}); true`);
    await waitFor("A opens the review", within(UI_REFRESH_S * 3), () => evaluate(a, `review?.id === ${JSON.stringify(ctx.review)} && reviewTotal() > 0`));
    for (const app of [b, c]) await waitFor(`${app.key} replays the review with A`, within(PARTY_REPEAT_S + UI_REFRESH_S * 2), () => evaluate(app, `review?.id === ${JSON.stringify(ctx.review)}`));
    for (const app of all) check((await state(app)).party.shared.origin === a.id, `${app.key} took over the review it only followed`);
    await evaluate(a, `act("seek", reviewTotal()); true`);
    for (const app of all) await waitFor(`${app.key} replays the watched session at the seek`, within(UI_REFRESH_S * 3), () => evaluate(app, `review?.id === ${JSON.stringify(ctx.review)} && selected === ${JSON.stringify(ctx.set)} && Math.abs(nowM() - ${SEEK_TO_S * 2}) < ${JUMP_MIN_S}`));
    await evaluate(a, `endReview(); openSession(${JSON.stringify(ctx.set)}); true`);
    ctx.notes.push(`review session with ${(await state(a)).sets.find((s) => s.id === ctx.review).party.length} party records replayed`);
  });

  await step("the local API is on from launch, reads recordings and takes a note and a game plan that everyone sees, and the page switches it off and on", async () => {
    check((await state(a)).api.on && fs.existsSync(path.join(a.data, "api.json")), "the API is not on from launch");
    await evaluate(a, `openSettings("api"); true`);
    await waitFor("A shows the API on", within(UI_REFRESH_S * 5), () => evaluate(a, `st.api.on && !$("apirows").hidden && $("apiaddr").textContent === "http://127.0.0.1:${API_PORTS.a}"`));
    const token = a.token;
    check(token.length === TOKEN_CHARS, `the token has ${token.length} characters`);
    const llms = await api(a, "GET", "/llms.txt");
    check(llms.status === 200 && llms.text.includes("Authorization: Bearer") && !llms.text.includes(token), `llms.txt: ${llms.status}`);
    check((await api(a, "GET", "/v1/rooms")).status === 401, "a request without the token was not refused");
    check((await api(a, "GET", "/v1/rooms", { token: "0".repeat(TOKEN_CHARS) })).status === 401, "a wrong token was not refused");
    check((await api(a, "GET", "/v1/rooms", { token, host: "evil.example" })).status === 403, "a foreign Host header was not refused");
    check((await api(a, "GET", "/v1/rooms", { token, host: `localhost.evil.example:${API_PORTS.a}` })).status === 403, "a look-alike Host header was not refused");
    const info = readJson(path.join(a.data, "api.json"));
    check(info?.token === token && info.address === `127.0.0.1:${API_PORTS.a}`, "api.json does not hold the address and token");
    const rooms = await api(a, "GET", "/v1/rooms", { token });
    const mine = rooms.json?.rooms?.find((r) => r.id === ctx.room);
    check(rooms.status === 200 && mine?.active && mine.members.length === 3, `rooms: ${brief(rooms.json)}`);
    const room = `/v1/rooms/${ctx.room}`;
    const recording = (id) => `${room}/recordings/${encodeURIComponent(id)}`;
    const list = await api(a, "GET", `${room}/recordings`, { token });
    const found = list.json?.recordings?.find((r) => r.id === ctx.set);
    check(found && found.kind === "play" && found.members.length >= 2 && found.length_s > 0 && found.end_ms > found.start_ms, `recordings: ${brief(list.json)}`);
    check(list.json.recordings.find((r) => r.id === ctx.review)?.kind === "debrief", "the review is not listed as a debrief");
    for (const tail of ["events", "chapters", "transcript"]) {
      const r = await api(a, "GET", `${recording(ctx.set)}/${tail}`, { token });
      check(r.status === 200 && Array.isArray(r.json?.[tail === "events" ? "events" : tail === "chapters" ? "chapters" : "lines"]), `${tail}: ${r.status} ${brief(r.text)}`);
    }
    const plugins = await api(a, "GET", `${room}/plugins`, { token });
    check(plugins.status === 200 && ["apex-matches", "transcribe"].every((id) => plugins.json.plugins.some((p) => p.id === id)), `plugins: ${brief(plugins.json)}`);
    const jobs = await api(a, "GET", `${room}/jobs`, { token });
    check(jobs.status === 200 && jobs.json.jobs.some((j) => j.plugin === PLUGIN && j.state === "done"), `jobs: ${brief(jobs.json)}`);
    check((await api(a, "POST", `${recording(ctx.set)}/events`, { token, body: { text: "" } })).status === 400, "an empty note was accepted");
    check((await api(a, "GET", `${recording(ctx.set)}/clips`, { token })).status === 404, "there is an endpoint for clips");
    const text = "Noted through the API #fromapi";
    const made = await api(a, "POST", `${recording(ctx.set)}/events`, { token, body: { text, at_s: SEEK_TO_S, tags: ["api"] } });
    check(made.status === 201 && made.json.at_s === SEEK_TO_S && made.json.tags.join() === "fromapi,api" && made.json.author === a.id, `the note: ${made.status} ${brief(made.text)}`);
    await allSee([a, b, c], "the note written through the API", within(LOG_SYNC_S * 2), (st) => st.events.some((e) => e.key === made.json.key && e.text === `${text} #api` && e.kind === "note"));
    const tagged = await api(a, "GET", `${recording(ctx.set)}/events?tag=API`, { token });
    check(tagged.json.events.some((e) => e.key === made.json.key), "the note is not found by its tag");
    await evaluate(a, `side = "notes"; openSession(${JSON.stringify(ctx.set)}); true`);
    await waitFor("A lists the note in the player", within(UI_REFRESH_S * 5), () => evaluate(a, `$("notelist").textContent.includes("Noted through the API")`));
    const status = await api(a, "GET", "/v1/status", { token });
    check(status.status === 200 && status.json.room === ctx.room && status.json.recording === false && ["idle", "standby"].includes(status.json.state) && status.json.members.length === 3 && status.json.members.some((m) => m.id === a.id && m.state === status.json.state), `status: ${status.status} ${brief(status.json)}`);
    const diag = await api(a, "GET", "/v1/diagnostics?everyone=true", { token });
    const diagText = diag.json && diag.json.text || "";
    check(diag.status === 200 && diagText.includes("=== this PC ===\nggi-hanseikai") && diagText.includes("recent log") && diagText.split("=== member ").length === 3, `diagnostics: ${diag.status} ${brief(diagText.slice(0, 200))}`);
    for (const secret of [NAMES.a, NAMES.b, NAMES.c, a.id, b.id]) check(!diagText.includes(secret), `the API report holds ${secret.slice(0, 4)}`);
    const operation = (name, body) => api(a, "POST", `/v1/ops/${name}`, { token, body });
    const opened = await operation("enter_room", { id: ctx.room });
    check(opened.status === 200 && opened.json === null, `enter_room: ${opened.status} ${brief(opened.text)}`);
    check((await operation("enter_room", { id: "nope" })).status === 400 && (await operation("enter_room", {})).json.error === "id is missing or not valid.", "enter_room takes a room that is not in the list");
    check((await operation("nothing", {})).status === 404 && (await api(a, "GET", "/v1/ops/state", { token })).status === 405 && (await operation("state", "[]")).status === 400, "an operation takes any name, method or body");
    check((await operation("state")).json.id === a.id, "an operation without a body is not run as {}");
    const board = "# Through the API\n**Goal: hold the high ground**";
    const put = await api(a, "PUT", `${room}/board`, { token, body: { text: board } });
    check(put.status === 200 && put.json.text === board, `the game plan: ${put.status} ${brief(put.text)}`);
    await allSee([a, b, c], "the game plan written through the API", within(LOG_SYNC_S * 2), (st) => st.events.some((e) => e.kind === "board" && e.text === board));
    check((await api(a, "GET", `${room}/board`, { token })).json.text === board, "the game plan reads back differently");
    await waitFor("A shows the game plan", within(UI_REFRESH_S * 5), () => evaluate(a, `boardText() === ${JSON.stringify(board)}`));
    await evaluate(a, `$("settings").close(); go("room"); true`);
    await waitFor("A shows the API badge", within(UI_REFRESH_S * 5), () => evaluate(a, `!$("apibadge").hidden && !$("apibadge").classList.contains("lan")`));
    await evaluate(a, `$("apibadge").click(); true`);
    check(await evaluate(a, `$("settings").open && settingsPane === "api" && !$("settings").querySelector('section[data-pane="api"]').hidden`), "the badge does not open Settings > API");
    await waitFor("A lists the endpoints", within(UI_REFRESH_S * 5), () => evaluate(a, `$("apiref").querySelectorAll("details").length >= 15 && $("apiref").textContent.includes("/v1/status") && $("apiref").querySelector("pre") !== null`));
    await evaluate(a, `$("settings").close(); go("room"); true`);
    await evaluate(a, `$("aibtn").click(); true`);
    await waitFor("A opens the AI menu", within(UI_REFRESH_S * 5), () => evaluate(a, `!$("menu").hidden && $("menu").querySelectorAll("button").length === 3`));
    await evaluate(a, `$("menu").querySelector("button").click(); true`);
    await waitFor("A shows the prompt before copying", within(UI_REFRESH_S * 5), () => evaluate(a, `$("promptdlg").open && $("prompttext").readOnly && $("prompttext").value.includes("llms.txt")`));
    check(await evaluate(a, `!$("prompttext").value.includes(promptFull.match(/Bearer <token>\\): (\\S+)/)[1]) && promptFull.includes("Bearer")`), "the prompt preview shows the token");
    await screenshot(a, "ai-prompt");
    await evaluate(a, `$("promptdlg").close(); true`);
    const prompt = await evaluate(a, `closeMenu(), invoke("api_info").then((i) => aiPrompt(aiPurposes()[0], i, st.room, newestRecording()))`);
    for (const part of [token, `http://127.0.0.1:${API_PORTS.a}/llms.txt`, ctx.room, ctx.review]) check(prompt.includes(part), `the prompt lacks ${part === token ? "the token" : part}`);
    await evaluate(a, `openSettings("api"); true`);
    await pressAndConfirm(a, `$("apinew").click()`);
    const fresh = await waitFor("A has a new token", within(UI_REFRESH_S * 5), () => {
      const next = readJson(path.join(a.data, "api.json"))?.token;
      return next !== token && next;
    });
    a.token = fresh;
    check((await api(a, "GET", "/v1", { token })).status === 401 && (await api(a, "GET", "/v1", { token: fresh })).status === 200, "the old token still works or the new one does not");
    check((await invoke(a, "api_info")).token === fresh, "the app keeps the old token");
    const hosts = (await invoke(a, "api_info")).hosts;
    check(hosts.length === 0, "the PC's addresses are listed while only this PC may connect");
    await evaluate(a, `document.querySelector('#apiaccess [data-lan="1"]').click(); true`);
    await waitFor("A allows other devices", within(UI_REFRESH_S * 5), () => evaluate(a, `st.api.lan && !$("apilanbox").hidden`));
    const own = (await invoke(a, "api_info")).hosts.find((h) => /^\d+\.\d+\.\d+\.\d+:/.test(h));
    check(readJson(path.join(a.data, "settings.json")).api_lan === true, "settings.json does not keep the access choice");
    if (own) {
      const [address] = own.split(":");
      const reached = await api(a, "GET", "/v1", { token: fresh, host: own, connect: address });
      check(reached.status === 200, `this PC's own address: ${reached.status}`);
      check((await api(a, "GET", "/v1", { host: own, connect: address })).status === 401, "an address without the token was not refused");
      check((await api(a, "GET", "/v1", { token: fresh, host: `evil.example:${API_PORTS.a}`, connect: address })).status === 403, "a foreign Host header was not refused for other devices");
    }
    await waitFor("A shows the warning on the badge", within(UI_REFRESH_S * 5), () => evaluate(a, `go("room"), $("apibadge").classList.contains("lan") && $("apibadge").title.includes("Other devices on your network")`));
    const faces = await evaluate(a, `(() => { const was = sideWidth; setSideWidth(clampSide(0), false); const f = $("roomfaces"); const r = { shown: f.querySelectorAll(".av").length, clipped: f.scrollWidth > f.clientWidth }; setSideWidth(was, false); return r; })()`);
    check(faces.shown === 3 && !faces.clipped, `the faces beside the badge at the narrowest sidebar: ${brief(faces)}`);
    await evaluate(a, `openSettings("api"); document.querySelector('#apiaccess [data-lan="0"]').click(); true`);
    await waitFor("A limits the API to this PC", within(UI_REFRESH_S * 5), () => evaluate(a, `!st.api.lan && $("apilanbox").hidden`));
    if (own) check(await api(a, "GET", "/v1", { token: fresh, host: own, connect: "127.0.0.1" }).then((r) => r.status === 403), "this PC's own address is accepted again for this PC only");
    await evaluate(a, `$("apion").click(); true`);
    await waitFor("A switches the API off", within(UI_REFRESH_S * 5), () => evaluate(a, `!st.api.on && $("apirows").hidden`));
    check(await api(a, "GET", "/llms.txt").then(() => false, () => true), "the API still answers after it was switched off");
    check(readJson(path.join(a.data, "settings.json")).api === false, "settings.json does not keep the switch");
    check(!fs.existsSync(path.join(a.data, "api.json")), "api.json stays after the API stopped");
    await evaluate(a, `$("apion").click(); true`);
    await waitFor("A switches the API on again", within(UI_REFRESH_S * 5), () => evaluate(a, `st.api.on && !$("apirows").hidden`));
    check((await api(a, "GET", "/v1", { token: fresh })).status === 200 && readJson(path.join(a.data, "api.json"))?.token === fresh, "the API does not answer with the same token after it was switched on again");
    await evaluate(a, `$("settings").close(); true`);
  });

  await step("Diagnostics is one screen with a masked report and the old developer window is gone", async () => {
    check((await evaluate(a, `invoke("open_debug").then(() => "open", () => "gone")`)) === "gone", "the developer window can still be opened");
    await evaluate(a, `openDiagnostics("summary"); true`);
    check(await evaluate(a, `[...$("diagdlg").querySelectorAll("[data-dtab]")].map((x) => x.dataset.dtab).join() === "summary,flow,pcs,traffic,log,jobs"`), "Diagnostics lacks its tabs");
    for (const tab of ["flow", "pcs", "traffic", "log", "jobs"]) {
      await evaluate(a, `openDiagnostics(${JSON.stringify(tab)}); true`);
      await waitFor(`Diagnostics shows ${tab}`, within(UI_REFRESH_S * 5), () => evaluate(a, `$("diagbody").textContent.length > 0 && !$("diagbody").textContent.includes(t("Loading…"))`));
    }
    await evaluate(a, `openDiagnostics("flow"); true`);
    await screenshot(a, "diagnostics");
    const report = await evaluate(a, `diagnosticsText()`);
    check(report.includes("ggi-hanseikai") && report.includes("jobs") && report.includes("recent log") && report.includes("\nhardware\n") && report.includes("\nclocks\n") && report.includes("\nwhere data goes\n"), "the report lacks its parts");
    for (const secret of [NAMES.a, NAMES.b, NAMES.c, a.id, b.id]) check(!report.includes(secret), `the report holds ${secret.slice(0, 4)}`);
    check(!/[A-Za-z]:\\/.test(report) && !/\b\d{1,3}(\.\d{1,3}){3}\b/.test(report), "the report holds a path or an address");
    await evaluate(a, `$("diagdlg").close(); true`);
  });

  await step("the set exports to a folder of plain files lined up on its start", async () => {
    const start = setOf(await state(a), ctx.set).start_us;
    await invoke(a, "add_event", { kind: "note", text: "Warm up", target: { set: ctx.set, from_us: start, to_us: start + 3e6 }, values: null });
    const out = await invoke(a, "export_folder", { room: ctx.room, set: ctx.set, folder: path.join(a.data, "bundles", ctx.set) }, EXPORT_FOLDER_S);
    check(out.files === 3 && out.skipped.length === 0, `folder export: ${brief(out)}`);
    for (const x of [a, b, c]) {
      for (const [file, kinds] of [[`${x.name}.mp4`, "audio,video"], [`${x.name} voice.m4a`, "audio"]]) {
        const probe = ffmpeg("ffprobe", ["-v", "error", "-show_entries", "format=duration:stream=codec_type", "-of", "json", path.join(out.folder, file)]);
        check(probe.ok, `ffprobe ${file}: ${probe.err}`);
        const info = JSON.parse(probe.out);
        check(info.streams.map((y) => y.codec_type).sort().join() === kinds, `${file} has streams ${brief(info.streams)}`);
        check(Math.abs(Number(info.format.duration) - ctx.durationS) <= SEGMENT_S, `${file} lasts ${info.format.duration} s, the set ${ctx.durationS} s`);
      }
    }
    const decode = ffmpeg("ffmpeg", ["-v", "error", "-i", path.join(out.folder, `${c.name}.mp4`), "-f", "null", "-"]);
    check(decode.ok && !decode.err, `${c.name}.mp4 does not decode cleanly: ${decode.err.slice(0, 300)}`);
    check(fs.readFileSync(path.join(out.folder, "notes.csv"), "utf8").includes("Noted through the API"), "notes.csv lacks the note written through the API");
    check(fs.readFileSync(path.join(out.folder, "chapters.vtt"), "utf8").includes("Warm up"), "chapters.vtt lacks the chapter");
    check(await invoke(a, "export_folder", { room: ctx.room, set: ctx.set, folder: out.folder }, EXPORT_FOLDER_S).then(() => false, () => true), "an export into a folder that is not empty was accepted");
  });

  const confirmDeletion = async (app, set, form) => {
    await evaluate(app, `confirmDelete([${JSON.stringify(set)}]); true`);
    await waitFor(`${app.key} asks before deleting`, within(UI_REFRESH_S * 5), () => evaluate(app, `$("deletedlg").open`));
    const said = await evaluate(app, `[$("deltitle").textContent, ...[...$("dellist").querySelectorAll(".delcard .small")].map((x) => x.textContent)]`);
    check(said[0] === "Delete this recording?" && said.length === 2 && said[1] === form, `${app.key} says ${brief(said)} instead of "${form}"`);
    await evaluate(app, `$("delyes").click(); true`);
  };
  const mediaLeft = (app, set) => logOf(app, ctx.room, app.id).filter((r) => r.k === "seg" && r.set === set).filter((r) => fs.existsSync(mediaFile(app, ctx.room, set, app.id, r.capture, r.series, segName(r.n)))).length;

  await step("deleting a recording others still hold keeps the row as not on this PC until Download", async () => {
    const all = [a, b, c];
    const mine = setOf(await state(b), ctx.review);
    check(mine.shared === true && mine.progress.have > 0 && await evaluate(b, `canDelete(setById(${JSON.stringify(ctx.review)}))`), `the review before deleting: ${brief({ shared: mine.shared, have: mine.progress.have })}`);
    await confirmDeletion(b, ctx.review, "Others still have it");
    await waitFor("B lists the review as not on this PC", within(UI_REFRESH_S * 3), async () => { const row = setOf(await state(b), ctx.review); return row && row.progress.have === 0 && !row.wanted; });
    check(mediaLeft(b, ctx.review) === 0 && !fs.existsSync(path.join(roomDir(b, ctx.room), "recordings", ctx.review)), "B kept media of the review after deleting it");
    check(!logOf(b, ctx.room, b.id).some((r) => r.k === "set_del"), "B removed the review for everyone although others hold it");
    await onlyOnDownload([b], a, ctx.review);
    await allSee(all, "everyone still lists the review", within(SYNC_ROUND_S), (st) => !!setOf(st, ctx.review));
    for (const app of [a, c]) check(setOf(await state(app), ctx.review).progress.have > 0, `${app.key} lost the review`);
    await waitFor("B fetches the review on request", within((SEGMENT_S + SYNC_ROUND_S) * 3), async () => heldByAll(await state(b), ctx.review, [b]) && !holdEverything([b], ctx.room, ctx.review, all));
  });

  await step("deleting a recording nobody else holds removes it, its jobs and its entry everywhere", async () => {
    const all = [a, b, c];
    marker(b, "offline", true);
    marker(c, "offline", true);
    await allSee([a], "A alone", within(DIAL_S), (st) => !connected(st, [b.id]) && !connected(st, [c.id]));
    for (const x of all) await invoke(x, "party_on", { on: false });
    await invoke(a, "start_recording");
    const bare = (await state(a)).active_set.id;
    await waitFor("A logs two segments", within(SEGMENT_S * 4), () => segsOf(logOf(a, ctx.room, a.id), bare, "v").length >= 2);
    await invoke(a, "stop_recording");
    await waitFor("A ended it", within(UI_REFRESH_S * 5), async () => { const st = await state(a); return !st.active_set && setOf(st, bare) && !setOf(st, bare).live; });
    marker(b, "offline", false);
    marker(c, "offline", false);
    await meshed(all);
    await allSee(all, "the recording nobody else holds listed", within(LOG_SYNC_S * 3), (st) => !!setOf(st, bare));
    const rowsOf = async (app) => (await invoke(app, "jobs")).jobs.filter((j) => j.set === bare).length;
    await waitFor("A and B list a job row for the recording nobody else holds", within(JOB_CHECK_S + LOG_SYNC_S * 3), async () => (await rowsOf(a)) > 0 && (await rowsOf(b)) > 0);
    check((await state(a)).sets.find((x) => x.id === bare).shared === false, "A thinks others hold the recording nobody else holds");
    check(await evaluate(b, `!canDelete(setById(${JSON.stringify(bare)}))`), "B may delete a recording it does not hold while A does");
    await waitFor("A made the thumbnail", within(30), async () => setOf(await state(a), bare)?.thumb === true);
    await confirmDeletion(a, bare, "This deletes it completely");
    await allSee(all, "the recording nobody else holds gone from the list", within(LOG_SYNC_S * 3), (st) => !setOf(st, bare));
    for (const app of all) await waitFor(`${app.key} has no job row for it`, within(JOB_CHECK_S + LOG_SYNC_S), async () => (await rowsOf(app)) === 0);
    check(logOf(a, ctx.room, a.id).some((r) => r.k === "set_del" && r.set === bare), "A did not log the removal");
    check(mediaLeft(a, bare) === 0 && !fs.existsSync(path.join(roomDir(a, ctx.room), "recordings", bare)) && !fs.existsSync(path.join(roomDir(a, ctx.room), "thumbs", `${bare}.jpg`)), "A kept media or the thumbnail");
    await waitFor("B's sidebar drops the recording nobody else holds", within(UI_REFRESH_S * 5), () => evaluate(b, `![...document.querySelectorAll("#sessions .srow")].some((r) => r.textContent.includes(sessionName(${JSON.stringify(bare)})))`));
  });

  await step("a short recording is offered for discarding when stopped and goes everywhere", async () => {
    const all = [a, b, c];
    await pressAndConfirm(a, "toggleRecording()");
    ctx.short = (await waitFor("A records", within(UI_REFRESH_S * 5), async () => (await state(a)).active_set?.id)) || "";
    await allSee(all, "recording the short one", within(LOG_SYNC_S * 2), (st) => st.active_set?.id === ctx.short);
    await waitFor("A logs a segment", within(SEGMENT_S * 3), () => segsOf(logOf(a, ctx.room, a.id), ctx.short).length > 0);
    await pressAndConfirm(a, "toggleRecording()");
    await waitFor("A offers to discard", within(UI_REFRESH_S * 5), () => evaluate(a, `$("confirm").open && $("confirmtitle").textContent === t("Discard this recording?")`));
    await evaluate(a, `$("confirmyes").click(); true`);
    await allSee(all, "the short recording gone", within(LOG_SYNC_S * 3), (st) => !st.active_set && !setOf(st, ctx.short));
    for (const app of all) await waitFor(`${app.key} removed the media`, within(SYNC_ROUND_S * 2), () => mediaLeft(app, ctx.short) === 0 && !fs.existsSync(path.join(roomDir(app, ctx.room), "recordings", ctx.short)));
  });

  await step("a member cut off for a whole recording lists it as not on this PC and fetches it only when asked", async () => {
    const all = [a, b, c];
    marker(c, "offline", true);
    await allSee([a, b], "C gone", within(DIAL_S), (st) => !connected(st, [c.id]));
    await invoke(a, "start_recording");
    const apart = (await state(a)).active_set.id;
    await allSee([b], "recording the set", within(LOG_SYNC_S * 2), (st) => st.active_set?.id === apart);
    await waitFor("A logs two segments", within(SEGMENT_S * 4), () => segsOf(logOf(a, ctx.room, a.id), apart, "v").length >= 2);
    await invoke(a, "stop_recording");
    await allSee([a, b], "the set ended and both hold it", within((SEGMENT_S + SYNC_ROUND_S) * 3), (st) => !st.active_set && setOf(st, apart) && !setOf(st, apart).live && heldByAll(st, apart, [a, b]));
    marker(c, "offline", false);
    await meshed(all);
    await allSee([c], "the set listed", within(LOG_SYNC_S * 3), (st) => !!setOf(st, apart));
    check(!logOf(c, ctx.room, c.id).some((r) => r.set === apart), "C joined a recording it was cut off from");
    await onlyOnDownload([c], a, apart);
    await allSee(all, "everyone has it", within((SEGMENT_S + SYNC_ROUND_S) * 3), (st) => heldByAll(st, apart, all));
  });

  await step("Start pressed on A and B together makes one recording on every PC", async () => {
    const all = [a, b, c];
    const sets = () => new Set(all.flatMap((x) => logOf(x, ctx.room, x.id).filter((r) => r.k === "rec").map((r) => r.set)));
    const known = sets();
    await allSee(all, "encoding its test source", within(COLD_START_S), (st) => st.capture.live);
    await Promise.all([invoke(a, "start_recording"), invoke(b, "start_recording")]);
    const winner = await waitFor("all three record the same set", within(SYNC_ROUND_S * 2), async () => {
      const ids = (await Promise.all(all.map(state))).map((st) => st.active_set?.id);
      return ids[0] && ids.every((id) => id === ids[0]) && ids[0];
    });
    await waitFor("each logs a segment of it", within(SEGMENT_S * 3), () => all.every((x) => segsOf(logOf(x, ctx.room, x.id), winner).length > 0));
    for (const other of [...sets()].filter((id) => !known.has(id) && id !== winner)) {
      for (const x of all) check(!lastJoin(x, ctx.room, other) || lastJoin(x, ctx.room, other).k === "rec_end", `${x.key} still records ${other}`);
    }
    await invoke(a, "stop_recording");
    await allSee(all, "the set ended", within(LOG_SYNC_S * 3), (st) => !st.active_set && setOf(st, winner) && !setOf(st, winner).live);
  });

  await step("the update channel is kept in the settings", async () => {
    check((await state(b)).channel === "stable", "the channel does not start as stable");
    await invoke(b, "set_channel", { channel: "beta" });
    check(readJson(path.join(b.data, "settings.json")).channel === "beta", "settings.json does not keep the beta channel");
    check(await invoke(b, "check_update").then(() => true, (e) => String(e).length > 0), "checking the beta channel failed");
    await invoke(b, "set_channel", { channel: "stable" });
  });

  await step("the microphone, the mix and the hotkey are kept in the settings", async () => {
    const { capture, hotkey } = await state(b);
    await invoke(b, "set_mic", { id: "none" });
    await invoke(b, "set_mix", { kind: "mic", volume: 0.5, muted: true });
    await invoke(b, "set_hotkey", { keys: "Ctrl+Alt+F11" });
    const now = await state(b);
    check(now.capture.mic === "none" && now.capture.mix.mic.volume === 0.5 && now.capture.mix.mic.muted === true && now.hotkey === "Ctrl+Alt+F11", `settings read back as ${brief({ mic: now.capture.mic, mix: now.capture.mix.mic, hotkey: now.hotkey })}`);
    const saved = readJson(path.join(b.data, "settings.json"));
    check(saved.mic === "none" && saved.mix.mic.muted === true && saved.hotkey === "Ctrl+Alt+F11", "settings.json does not keep them");
    check(await invoke(b, "set_hotkey", { keys: "K" }).then(() => false, () => true), "a hotkey without Ctrl or Alt was accepted");
    await invoke(b, "set_mic", { id: capture.mic });
    await invoke(b, "set_mix", { kind: "mic", ...capture.mix.mic });
    await invoke(b, "set_hotkey", { keys: hotkey });
  });

  await step("C kept every file of an older version until Delete old files, which removes exactly those", async () => {
    const data = c.data;
    const old = Object.keys(oldFiles()).filter((name) => !keptOldFiles().includes(name));
    for (const [name, body] of Object.entries(oldFiles())) {
      const file = path.join(data, name);
      check(fs.existsSync(file) && fs.readFileSync(file).equals(Buffer.from(body)), `${name} was changed or removed`);
    }
    check(!fs.existsSync(path.join(data, oldStandby())), "C kept a live segment of an earlier run");
    const bytesOld = old.reduce((n, name) => n + Buffer.from(oldFiles()[name]).length, 0);
    const info = await invoke(c, "data_info");
    check(info.old_bytes === bytesOld, `data_info counts ${info.old_bytes} B of old files, not ${bytesOld}`);
    const rooms = path.join(data, "rooms", OLD_ROOM);
    const rootBefore = fs.readdirSync(data);
    const insideBefore = filesUnder(rooms).map((f) => path.relative(data, f));
    await evaluate(c, `loadStorage(); true`);
    await waitFor("C offers to delete the old files", within(UI_REFRESH_S * 5), () => evaluate(c, `!$("oldfiles").hidden && $("oldfiles").textContent === t("Delete old files ({size})", { size: bytes(${info.old_bytes}) })`));
    const title = await pressAndConfirm(c, `$("oldfiles").click()`);
    check(title === "Delete old files?" && c.confirmText === "", `C asked "${title}" with "${c.confirmText}"`);
    await waitFor("the old files are gone and the button hides", within(UI_REFRESH_S * 5), async () => old.every((name) => !fs.existsSync(path.join(data, name))) && (await invoke(c, "data_info")).old_bytes === 0 && evaluate(c, `$("oldfiles").hidden`));
    for (const name of keptOldFiles()) check(fs.existsSync(path.join(data, name)), `${name} was removed`);
    check(fs.existsSync(path.join(data, "key.bin")), "key.bin was removed");
    const rootGone = rootBefore.filter((name) => !fs.existsSync(path.join(data, name))).sort().join();
    check(rootGone === ["log.bin", "peer", "peer.txt", "peers.txt", "rec-1767225600000.mp4"].sort().join(), `the data folder lost ${rootGone}`);
    const insideGone = insideBefore.filter((f) => !fs.existsSync(path.join(data, f))).sort().join();
    check(insideGone === old.filter((name) => name.startsWith(path.join("rooms", OLD_ROOM))).sort().join(), `the old room lost ${insideGone}`);
    await allSee([a, b, c], "every recording still held", within(SYNC_ROUND_S), (st) => heldByAll(st, ctx.set, [a, b, c]));
  });
}

async function main() {
  fs.rmSync(OUT, { recursive: true, force: true });
  fs.mkdirSync(OUT, { recursive: true });
  check(fs.existsSync(EXE), `no app at ${EXE}`);
  check(fs.existsSync(path.join(FFMPEG_DIR, "ffprobe.exe")), `no ffprobe in ${FFMPEG_DIR}`);
  const ctx = { notes: [] };
  NTP.a = await timeServer(0);
  NTP.b = await timeServer(CLOCK_SHIFT_S);
  const started = Date.now();
  let failed = null;
  try {
    for (const part of (process.env.E2E_PART || "live,recover,library").split(",")) {
      for (const app of Object.values(apps)) kill(app);
      for (const key of Object.keys(apps)) delete apps[key];
      fs.rmSync(path.join(OUT, "apps"), { recursive: true, force: true, maxRetries: 10, retryDelay: 500 });
      await { live, recover, library }[part](ctx);
    }
  } catch (e) {
    failed = e;
  }
  const total = (Date.now() - started) / 1000;
  if (failed) {
    await collect(ctx.room);
    say(`FAILED in "${current}": ${failed.stack}`);
    fs.writeFileSync(path.join(OUT, "failure.txt"), `${current}\n${failed.stack}\n`);
    annotate("error", `e2e failed: ${current}`, failed.message);
    for (const app of Object.values(apps)) annotate("notice", `${app.key} log tail`, logTail(app));
    annotate("notice", "processes", processes());
  }
  for (const app of Object.values(apps)) kill(app);
  fs.writeFileSync(path.join(OUT, "timings.json"), JSON.stringify({ total, steps: timings }, null, 1));
  const summary = `${failed ? "failed" : "passed"} in ${total.toFixed(0)} s\n${timings.map((t) => `${t.step}: ${t.seconds.toFixed(0)} s`).join("\n")}\n${ctx.notes.join("\n")}`;
  say(summary);
  annotate("notice", "e2e summary", summary);
  process.exit(failed ? 1 : 0);
}

main();

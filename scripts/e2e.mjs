import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import dgram from "node:dgram";
import fs from "node:fs";
import http from "node:http";
import path from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

const EXE = path.resolve(process.argv[2] || "src-tauri/target/release/ez-hanseikai.exe");
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
const RELAY_WAIT_S = 10;
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
const STAY_AT_S = 3;
const FAR_FROM_END_S = 8;
const JUST_ENTERED_S = 1;
const PLAY_START_S = 8;
const LOG_TAIL_LINES = 15;
const APPROVAL_WAIT_S = LOG_SYNC_S * 3;
const SOURCE = { kind: "display", index: 1 };
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
    if (s.video.readyState < 2 || s.video.seeking || s.base === null || s.base + s.shift > liveClock.pos) return null;
    return s.base + s.video.currentTime;
  };
  const tiles = Object.values(grid.tiles);
  return { now: grid === play, wall: nowUs() / 1e6, pos: liveClock.pos, rate: liveClock.rate, held: liveClock.held, avail: Object.fromEntries(liveMembers().map((m) => [m.key, m.avail])), at: tiles.map(at).filter((x) => x !== null), wait: tiles.some((tl) => tl.box.classList.contains("wait")), why: tiles.map((tl) => [tl.key.slice(0, 4), !!tl.media.part, !!tl.media.index, tl.box.classList.contains("wait"), tl.media.video.readyState]), late: tiles.filter((tl) => tl.box.classList.contains("late")).map((tl) => tl.key) };
})()`;

const DIAL_S = DIAL_RETRY_S + CONNECT_TIMEOUT_S;
const REJOIN_S = DIAL_RETRY_S * 2 + SLOW_WAIT_S;
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

const mouse = (app, type, [x, y], buttons) => send(app, "Input.dispatchMouseEvent", { type, x, y, button: type === "mouseMoved" && !buttons ? "none" : "left", buttons, clickCount: 1 });

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
  fs.writeFileSync(path.join(dir, "settings.json"), JSON.stringify({ limit_mbps: 0, keep_days: 0, source: "display:1", language: "xx", quality: "ultra", obs_port: 4455, channel: "beta", api_lan: true }));
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
    check(!x.wait, `a live tile with a picture hid it: ${JSON.stringify(x)}`);
    check(x.at.length < 2 || Math.max(...x.at) - Math.min(...x.at) < 1, `live tiles are apart: ${brief(x)}`);
    check(x.pos === null || x.avail[b.id] === null || x.late.includes(b.id) || x.pos <= x.avail[b.id], `the live position passed B's media: ${brief(x)}`);
    check(!last || (x.pos >= last.pos && x.now === last.now), `the live position went back or the view changed: ${brief([last, x])}`);
    if (until(x)) return x;
    last = x;
    await sleep(POLL_MS);
  }
  return last;
}

async function follow(ctx, from, others, playing, position) {
  const seconds = within(PARTY_REPEAT_S + UI_REFRESH_S * 2);
  const shared = await waitFor(`${from.key} shares its action`, seconds, async () => {
    const mine = (await state(from)).party.shared;
    return mine.origin === from.id && mine.set === ctx.set && mine.playing === playing && mine;
  });
  await Promise.all(others.map((app) => waitFor(`${app.key} follows ${from.key}`, seconds, async () => {
    const mine = (await state(app)).party.shared;
    const ui = await evaluate(app, `({ ok: selected === ${JSON.stringify(ctx.set)} && clock.playing === ${playing} && Math.abs(nowM() - ${position}) < ${JUMP_MIN_S}, selected, playing: clock.playing, m: nowM(), last: partyLast && [partyLast.seq, partyLast.stamp, partyLast.origin.slice(0, 4)] })`);
    check(mine.seq === shared.seq && mine.stamp === shared.stamp && mine.origin === shared.origin && ui.ok, brief({ seq: mine.seq, stamp: mine.stamp, ui }));
    return true;
  })));
}

const TILES_PROBE = `(() => {
  const wall = wallOf(nowM()) / 1e6;
  return { m: nowM(), playing: clock.playing, tiles: Object.values(play.tiles).map(({ media: s }) => { const want = wall - s.base; return { has: s.base !== null && !!s.index && covers(s.index, want - s.shift), into: s.index && s.index.segs.length ? want - s.shift - s.index.segs[0].t_us / 1e6 : -1, paused: s.video.paused, ready: s.video.readyState, off: s.video.currentTime - want, at: s.video.currentTime, buf: JSON.stringify(rangesOf(s.video.buffered)), dur: s.video.duration, segs: s.index ? s.index.segs.length + (s.index.open ? "+" : "") : -1, end: s.index ? endOf(s.index) : -1, shift: s.shift, seeking: s.video.seeking, err: !!s.video.error }; }) };
})()`;

async function together(app, [from, to]) {
  const wrong = [];
  let seen = null;
  const first = await waitFor(`${app.key} plays every tile`, within(PLAY_START_S), async () => {
    const x = await evaluate(app, TILES_PROBE);
    seen = x;
    if (x.m < from || x.m > to) return false;
    const shown = x.tiles.filter((t) => t.has && t.into >= JUST_ENTERED_S);
    if (!x.playing || shown.some((t) => t.paused !== shown[0].paused)) wrong.push(x);
    return shown.length > 0 && shown.every((t) => !t.paused && t.ready >= 3 && Math.abs(t.off) < 1) && x;
  }).catch((e) => { throw new Error(`${e.message} ${JSON.stringify(seen)}`); });
  check(!wrong.length, `${app.key}'s tiles started apart or Play was undone: ${brief(wrong[0])}`);
  await sleep(1000);
  const later = await evaluate(app, TILES_PROBE);
  check(later.playing && later.tiles.every((t, i) => !t.has || !first.tiles[i].has || t.at > first.tiles[i].at), `${app.key}'s tiles did not move on: ${JSON.stringify([first, later])}`);
}

function logLines(app) {
  return fs.readFileSync(path.join(app.data, "log.txt"), "utf8").split("\n");
}

function connects(app, peer, from = 0) {
  return logLines(app).slice(from).filter((l) => l.includes(` ${peer.id.slice(0, 10)} `) && l.includes(": connected (")).length;
}

function api(app, method, route, { token, host, body, timeout = CALL_S } = {}) {
  const port = API_PORTS[app.key];
  return new Promise((resolve, reject) => {
    const payload = body === undefined ? null : Buffer.from(typeof body === "string" ? body : JSON.stringify(body));
    const headers = { Host: host || `127.0.0.1:${port}`, ...(token ? { Authorization: `Bearer ${token}` } : {}), ...(payload ? { "Content-Type": "application/json", "Content-Length": payload.length } : {}) };
    const req = http.request({ host: "127.0.0.1", port, method, path: route, headers, agent: false, timeout: timeout * 1000 }, (res) => {
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () => {
        const raw = Buffer.concat(chunks);
        const text = raw.toString("utf8");
        let json = null;
        try {
          json = JSON.parse(text);
        } catch {}
        resolve({ status: res.statusCode, text, json, raw, type: res.headers["content-type"] });
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

function processTable() {
  const out = spawnSync("powershell", ["-NoProfile", "-Command", "Get-CimInstance Win32_Process | ForEach-Object { \"$($_.ProcessId) $($_.ParentProcessId) $($_.CreationDate.ToFileTimeUtc()) $($_.Name)\" }"], { encoding: "utf8" }).stdout || "";
  return out.trim().split(/\r?\n/).map((line) => line.split(" ")).map(([pid, parent, made, name]) => ({ pid: Number(pid), parent: Number(parent), made: Number(made), name }));
}

function descendants(pid) {
  const all = processTable();
  const found = all.filter((p) => p.pid === pid);
  for (let i = 0; i < found.length; i++) found.push(...all.filter((p) => p.parent === found[i].pid && p.made >= found[i].made && !found.includes(p)));
  return found;
}

function running(tree) {
  const now = new Set(processTable().map((p) => `${p.pid}@${p.made}`));
  return tree.filter((p) => now.has(`${p.pid}@${p.made}`));
}

function kill(app, tree = true) {
  if (!app.proc || app.proc.exitCode !== null) return;
  app.killed = true;
  app.tree = descendants(app.proc.pid);
  spawnSync("taskkill", ["/PID", String(app.proc.pid), ...(tree ? ["/T"] : []), "/F"], { stdio: "ignore" });
}

async function stopped(app) {
  await waitFor(`${app.key} and its child processes exit`, within(CALL_S), () => { const left = running(app.tree); check(!left.length, brief(left)); return app.proc.exitCode !== null; });
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

const meshed = (list, room, seconds = DIAL_S * 2) => allSee(list, "connected to each other", within(seconds), (st, app) => (!room || (st.room?.id === room && !st.room.pending)) && connected(st, list.filter((x) => x !== app).map((x) => x.id)));

async function refusedOutsideARoom(app) {
  const room = (await state(app)).room?.id || "";
  const writes = [["add_event", { kind: "note", text: "x", target: {}, values: null }], ["create_invite", { reuse: false }], ["put_board_card", { room, card: "", text: "x" }], ["start_recording", {}]];
  for (const [op, args] of writes) {
    const said = await invoke(app, op, args).then(() => "", (e) => String(e.message || e));
    check(said.includes("Join a room first."), `${app.key}'s ${op} outside a room: ${brief(said)}`);
  }
}

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
  const script = "Get-CimInstance Win32_Process | Where-Object { $_.Name -match 'ez-hanseikai' -or $_.CommandLine -match 'remote-debugging-port' } | ForEach-Object { \"$($_.ProcessId) $($_.ParentProcessId) $($_.Name) $($_.CreationDate.ToString('HH:mm:ss')) $(if ($_.CommandLine -match 'remote-debugging-port=(\\d+)') { $matches[1] })\" }; Get-Process ez-hanseikai, msedgewebview2 -ErrorAction SilentlyContinue | ForEach-Object { \"$($_.Id) $($_.ProcessName) responding=$($_.Responding) window=$($_.MainWindowHandle) title=$($_.MainWindowTitle) threads=$($_.Threads.Count) cpu=$([int]$_.CPU)\" }; netstat -ano | Select-String ':93[0-9][0-9] '; Get-ChildItem $env:E2E_OUT\\apps\\*\\data | ForEach-Object { \"$($_.FullName.Substring($env:E2E_OUT.Length)) $($_.LastWriteTime.ToString('HH:mm:ss'))\" }; Get-ChildItem -Recurse -Filter DevToolsActivePort $env:E2E_OUT | ForEach-Object { \"$($_.FullName) $($_.LastWriteTime.ToString('HH:mm:ss')) $((Get-Content $_.FullName) -join ' ')\" }; Get-Date -Format HH:mm:ss";
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
  await step("start A and B, and nothing is written before A is in a room", async () => {
    a = await launch("a");
    b = await launch("b");
    await refusedOutsideARoom(a);
    const rooms = path.join(a.data, "rooms");
    check(!fs.existsSync(rooms) || !fs.readdirSync(rooms).length, "a write without a room made a room folder");
  });

  await step("recording alone, the player follows the picture as it grows", async () => {
    ctx.room = await invoke(a, "create_room", { name: "Practice" });
    await waitFor("A opens the room on Live", within(UI_REFRESH_S * 5), () => onLive(a));
    await allSee([a], "encoding its test source", within(COLD_START_S), (st) => st.capture.live);
    await pressAndConfirm(a, "toggleRecording()");
    const alone = (await waitFor("A records", within(UI_REFRESH_S * 5), async () => (await state(a)).active_set?.id)) || "";
    await waitFor("A opens Live when the recording starts", within(UI_REFRESH_S * 5), () => onLive(a));
    await evaluate(a, "openNow(); true");
    await waitFor("A's player holds the growing recording still while this PC records", within(SEGMENT_S * 4), () => evaluate(a, `selected === ${JSON.stringify(alone)} && !clock.playing && liveClock.pos === null`));
    await evaluate(a, "act('play', nowM()); true");
    const view = () => evaluate(a, `(() => { const v = Object.values(play.tiles)[0]?.media.video; return { shown: selected === ${JSON.stringify(alone)}, playing: clock.playing, cur: v ? v.currentTime : 0, dur: v ? bufferedEnd(v) : 0 }; })()`);
    const first = await waitFor("A's player plays the growing recording once play is pressed", within(SEGMENT_S * 4), async () => { const x = await view(); return x.shown && x.playing && x.cur > 0 && x.dur > 0 && x; });
    await waitFor("A's player moves on with the picture", within(SEGMENT_S * 3), async () => { const x = await view(); return x.cur > first.cur + FRAGMENT_S * 2 && x.dur > first.dur + SEGMENT_S && x; });
    await pressAndConfirm(a, "toggleRecording()");
    await waitFor("A stopped the recording", within(UI_REFRESH_S * 5), async () => !(await state(a)).active_set);
    await invoke(a, "delete_set", { room: ctx.room, set: alone });
    await waitFor("A dropped the recording", within(UI_REFRESH_S * 5), async () => !setOf(await state(a), alone));
    await evaluate(a, "openLive(); true");
    await waitFor("A is back on Live", within(UI_REFRESH_S * 5), () => onLive(a));
    await evaluate(a, "setParty(true); openLive(); true");
    await waitFor("A is in the Watch Party", within(UI_REFRESH_S * 5), async () => (await state(a)).party.on);
    await waitFor("A shares no recording", within(UI_REFRESH_S * 5), async () => !(await state(a)).party.shared.set);
  });

  await step("B joins A's room with approval", async () => {
    await asks(a, b, a);
    await refusedOutsideARoom(b);
    await sleep(APPROVAL_WAIT_S * 1000);
    await invoke(a, "answer_ask", { id: b.id, approve: true });
    await meshed([a, b], ctx.room);
    const names = (await state(b)).names;
    check(names[a.id] === a.name, `B knows A as ${names[a.id]}`);
    const dials = connects(b, a);
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
    const offset = (st, x) => st.peers.find((p) => p.id === x.id)?.offset_us;
    await waitFor("A measures B 8.2 s ahead and C in step", within(SYNC_ROUND_S * 2), async () => { const st = await state(a); return Math.abs(offset(st, b) - CLOCK_SHIFT_S * 1e6) < 1e6 && Math.abs(offset(st, c)) < 1e6; });
    check((await state(c)).clock.offset_us === null, "C has a time answer");
    const flow = await invoke(c, "debug_flow");
    check(await evaluate(c, `flowRows(st, ${JSON.stringify(flow)}).find((r) => r.key === "Time check").level === "warn"`), "C's Where data goes does not warn about the time check");
    const players = setOf(await state(a), ctx.set).players;
    const begins = (x) => { const p = players.find((y) => y.owner === x.id); return (p.start_us - p.offset_us) / 1e6; };
    check(Math.abs(begins(b) - begins(a)) < within(1), `B's part starts ${begins(b) - begins(a)} s from A's`);
  });

  await step("while recording the start box says Stop recording with the time and the list gets the running recording", async () => {
    check(await evaluate(a, `$("startbtn").textContent.includes(t("Stop recording")) && !!$("startbtn").querySelector("#livetimer") && !!$("startbtn").querySelector(".recdot") && !!$("sessions").querySelector(".liverow .thumb.live")`), "the start box or the running recording's row is wrong");
  });

  await step("Live shows a tile for everyone who shares a screen, also while recording", async () => {
    for (const app of [a, b, c]) {
      await evaluate(app, `openLive(); true`);
      await waitFor(`${app.key} shows three live tiles`, within(UI_REFRESH_S * 10), () => evaluate(app, `!$("livetiles").hidden && Object.keys(rec.tiles).length === 3 && $("ptitle").textContent === t("Live")`));
    }
    check(await evaluate(a, `$("miccap").classList.contains("off") && $("screencap").classList.contains("on") && getComputedStyle($("miccap")).borderTopStyle === "dashed" && !!$("miclvl")`), "the bottom-left buttons are not in the shared off/recording states");
    await screenshot(a, "live-recording");
    await waitFor("A marks only Live, not the running recording", within(UI_REFRESH_S * 3), () => evaluate(a, `!$("sessions").querySelector(".liverow.on") && $("sidenav").querySelector("[aria-current=page]").dataset.nav === "live"`));
    await evaluate(a, `$("sessions").querySelector(".liverow").click(); true`);
    await waitFor("A marks only the running recording after opening it", within(UI_REFRESH_S * 3), () => evaluate(a, `!!$("sessions").querySelector(".liverow.on") && !$("sidenav").querySelector("[aria-current=page]")`));
    await evaluate(a, `openLive(); true`);
  });

  await step("connections stay up while live tiles stay in step, a member stalls, and the others stop waiting for one too far behind", async () => {
    const inStep = async (view) => {
      for (const x of [a, b, c]) await evaluate(x, `${view}; if (playSet()) { act("play", nowM()); goLive(); } true`);
      await waitFor(`A's live tiles in step after ${view}`, within(SEGMENT_S * 3), async () => { const x = await evaluate(a, LIVE_PROBE); check(x.pos !== null && x.at.length === 3 && Math.max(...x.at) - Math.min(...x.at) < 1, brief(x)); return true; });
    };
    const all = [a, b, c];
    const since = Object.fromEntries(all.map((x) => [x.key, logLines(x).length]));
    await inStep("openNow()");
    await watchLive(a, b, SEGMENT_S * 2);
    for (const app of all) {
      for (const peer of all.filter((x) => x !== app)) {
        const n = connects(app, peer, since[app.key]);
        check(n === 0, `${app.key} connected to ${peer.key} ${n} times while the live tiles ran`);
      }
      check(connected(await state(app), all.filter((x) => x !== app).map((x) => x.id)), `${app.key} lost a peer`);
    }
    await inStep("openLive()");
    await invoke(b, "set_limit", { mbps: TRICKLE_MBPS });
    const captures = () => logOf(b, ctx.room, b.id).filter((r) => r.k === "cap").length;
    const before = captures();
    const held = await watchLive(a, b, LIVE_LAG_MAX_S + SEGMENT_S, (x) => x.held || x.late.length > 0);
    await invoke(b, "set_limit", { mbps: null });
    check(held.held || held.wall - held.pos > LIVE_LAG_MAX_S, `A never held for B: ${brief(held)}`);
    const fast = await watchLive(a, b, SEGMENT_S * 3, (x) => x.rate === LIVE_CATCH_UP);
    check(fast.rate === LIVE_CATCH_UP, `A never caught up at ${LIVE_CATCH_UP}: ${brief(fast)}`);
    await waitFor("A plays at 1 again once the delay is small", within(CATCH_UP_S), async () => (await evaluate(a, LIVE_PROBE)).rate === 1);
    await evaluate(a, `openNow(); act("play", nowM()); goLive(); true`);
    await invoke(b, "set_limit", { mbps: TRICKLE_MBPS });
    const gone = await waitFor("A stops waiting for B and marks its tile", within(LIVE_LAG_MAX_S + SEGMENT_S * 2), async () => { const x = await evaluate(a, LIVE_PROBE); check(x.late.join() === b.id && x.pos > x.avail[b.id], brief(x)); return x; });
    await sleep(1000);
    check((await evaluate(a, LIVE_PROBE)).pos > gone.pos, "the others stopped while waiting for B");
    await invoke(b, "set_limit", { mbps: null });
    await waitFor("B's tile is no longer late", within(SEGMENT_S * 3), async () => !(await evaluate(a, LIVE_PROBE)).late.length);
    check(captures() === before, "B's capture changed while the set recorded");
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

  await step("B leaves the room from its menu and enters it again with its recordings and notes, A and B leave and enter again at once, a new room moves B out of it, and B deletes it from the list", async () => {
    const kept = (st) => ({ ids: [st.sets.map((s) => s.id), st.events.map((e) => e.key)].map((x) => x.sort().join()).join(), have: Object.fromEntries(st.sets.flatMap((s) => s.players.map((p) => [`${s.id} ${p.owner}`, p.have]))) });
    const before = kept(await state(b));
    await waitFor("B shows the room header", within(UI_REFRESH_S * 5), () => evaluate(b, `!$("app").hidden && !!st.room`));
    await evaluate(b, `$("roomname").click(); [...$("menu").querySelectorAll("button")].find((x) => x.textContent === t("Leave room")).click(); true`);
    await waitFor("B shows the start screen and still lists the room", within(UI_REFRESH_S * 5), async () => { const st = await state(b); return !st.room && st.rooms.some((r) => r.id === ctx.room) && evaluate(b, `!$("start").hidden && $("app").hidden`); });
    await invoke(b, "enter_room", { id: ctx.room });
    await meshed([a, b], ctx.room, REJOIN_S);
    const after = kept(await state(b));
    check(after.ids === before.ids && Object.entries(before.have).every(([k, n]) => after.have[k] >= n), "B lost recordings or notes by leaving and entering again");
    for (const app of [b, a]) {
      await invoke(app, "close_room", { id: ctx.room });
      await invoke(app, "enter_room", { id: ctx.room });
      await meshed([a, b], ctx.room, REJOIN_S);
    }
    const other = await invoke(b, "create_room", { name: "Other" });
    check((await state(b)).room?.id === other, "B did not move to the room it created");
    await invoke(b, "enter_room", { id: ctx.room });
    await meshed([a, b], ctx.room, REJOIN_S);
    await invoke(b, "remove_room", { id: other });
    await invoke(b, "close_room", { id: ctx.room });
    await waitFor("B lists the room", within(UI_REFRESH_S * 5), () => evaluate(b, `!$("start").hidden && $("roomlist").children.length > 0`));
    await pressAndConfirm(b, `[...$("roomlist").children].find((r) => r.querySelector("b").textContent === st.rooms.find((x) => x.id === ${JSON.stringify(ctx.room)}).name).querySelector(".more").click(); [...$("menu").querySelectorAll("button")].find((x) => x.textContent === t("Delete…")).click()`);
    await waitFor("B no longer lists the room", within(UI_REFRESH_S * 5), async () => { const st = await state(b); return !st.room && !st.rooms.some((r) => r.id === ctx.room); });
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
  await step("A, B and C record the same set, C joining late only through the relay: it sends and gets voice and logs but no picture, and catches up once it comes back on a direct path", async () => {
    [a, b, c] = await gather(ctx, { EZH_RELAY_ONLY: "1" });
    const all = [a, b, c];
    await allSee([a, b], "C's path is the relay", within(DIAL_S), (st) => st.peers.find((p) => p.id === c.id)?.path === "relay");
    for (const x of [a, b]) await waitFor(`${x.key} says C has no picture via relay`, within(UI_REFRESH_S * 10), () => evaluate(x, `memberOf(${JSON.stringify(c.id)}).status.text === t("No picture via relay")`));
    const voiced = segsOf(logOf(a, ctx.room, a.id), ctx.set, "m").length;
    const voice = pieces(logOf(a, ctx.room, a.id), ctx.set, voiced).filter((p) => p.series === "m");
    await waitFor("C holds A's voice", within((SEGMENT_S + SYNC_ROUND_S) * 2), () => !badCopies(c, ctx.room, a.id, voice).length);
    const pictures = (holder, owner) => filesUnder(path.join(roomDir(holder, ctx.room), "recordings", ctx.set, owner.id)).filter((f) => f.split(path.sep).includes("v"));
    check(!pictures(c, a).length && !pictures(a, c).length, `a picture crossed the relay: ${brief([pictures(c, a).length, pictures(a, c).length])}`);
    kill(c);
    await stopped(c);
    start("c", {});
    await attach(c);
    await share(c);
    await meshed(all);
    await allSee([a, b], "C's path is direct", within(DIAL_S), (st) => st.peers.find((p) => p.id === c.id)?.path === "direct");
    const limits = Object.fromEntries(all.map((x) => [x.key, segsOf(logOf(x, ctx.room, x.id), ctx.set, "v").length]));
    await waitFor(`everyone holds what was logged by now (${brief(limits)})`, within((SEGMENT_S + SYNC_ROUND_S) * 6), () => !holdEverything(all, ctx.room, ctx.set, all, limits));
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
    await waitFor("A and B record on while B is cut off", within(OFFLINE_S + SEGMENT_S * 2), () => logged(a) >= before.a + OFFLINE_S / SEGMENT_S && logged(b) >= before.b + OFFLINE_S / SEGMENT_S);
    check((await state(a)).active_set?.id === ctx.set && (await state(b)).active_set?.id === ctx.set, "B or A stopped recording while B was cut off");
    marker(b, "offline", false);
    await meshed(all);
    const limits = Object.fromEntries(all.map((x) => [x.key, logged(x)]));
    await waitFor(`everyone holds what was logged by now (${brief(limits)})`, within((SEGMENT_S + SYNC_ROUND_S) * 4), () => !holdEverything(all, ctx.room, ctx.set, all, limits));
  });

  await step("A's encoder is killed and A keeps recording the set in a new part", async () => {
    const started = logLines(a).map((l) => l.match(/capture (\S+) started: .*, process (\d+)/)).filter(Boolean).pop();
    check(started, "A has no encoder");
    const [, capture, pid] = started;
    spawnSync("taskkill", ["/PID", pid, "/F"], { stdio: "ignore" });
    const parts = () => segsOf(logOf(a, ctx.room, a.id), ctx.set, "v").filter((r) => r.capture !== capture).length;
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

  await step("B restarts during the recording as an update does: the recording goes on and B is back in it with its screen in a new part", async () => {
    const before = segsOf(logOf(b, ctx.room, b.id), ctx.set).map((s) => s.capture);
    const resume = path.join(b.data, "resume.json");
    kill(b);
    await stopped(b);
    fs.writeFileSync(resume, JSON.stringify({ source: SOURCE, muted: false }));
    start("b");
    await attach(b);
    const st = await state(b);
    check(st.capture.source && st.capture.mix.mic.muted === false && !fs.existsSync(resume), "B did not take its screen and microphone back from resume.json");
    await recordsAgain(b, ctx.room, ctx.set, before);
    await allSee([a], "B recording again in the same set", within(SEGMENT_S + LOG_SYNC_S * 2), (st) => st.active_set?.id === ctx.set && setOf(st, ctx.set).players.find((p) => p.owner === b.id).recording);
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
    await meshed(all, ctx.room, REJOIN_S);
    await recordsAgain(b, ctx.room, ctx.set, before);
    await allSee([a], "B recording again", within(SEGMENT_S + LOG_SYNC_S * 2), (st) => setOf(st, ctx.set).players.find((p) => p.owner === b.id).recording);
    kill(c);
    await stopped(c);
    await invoke(a, "stop_recording");
    const lastOf = (app) => Math.max(...segsOf(logOf(app, ctx.room, c.id), ctx.set).map((s) => s.t_us + s.d_us));
    await allSee([a, b], "the set shown ended at once, not before C's last segment", within(LOG_SYNC_S * 2), (st, app) => { const set = setOf(st, ctx.set); return !st.active_set && set && !set.live && set.end_us >= lastOf(app) - set.players.find((p) => p.owner === c.id).offset_us; });
    const ended = setOf(await state(a), ctx.set);
    ctx.durationS = (ended.end_us - ended.start_us) / 1e6;
    const far = Math.max(0, ctx.durationS - FAR_FROM_END_S);
    check(!(await evaluate(a, `watchParty`)), "the app starts in the Watch Party");
    await evaluate(a, `setParty(false); openSession(${JSON.stringify(ctx.set)}); act("seek", ${far}); act("play", ${far}); true`);
    const own = `(() => { const v = play.tiles[st.id].media.video; return v.videoWidth > 0 && !v.paused && v.readyState >= 3 ? v.currentTime : 0; })()`;
    const shown = await waitFor("A's own tile plays right after the stop", within(PLAY_START_S), () => evaluate(a, own));
    await waitFor("A's own tile moves on", within(PLAY_START_S), async () => (await evaluate(a, own)) > shown);
    const mine = ended.players.find((p) => p.owner === a.id);
    await evaluate(a, `act("pause", ${(mine.last_us.v - ended.start_us) / 1e6 - 0.1}); true`);
    await waitFor("A's own tile covers the end", within(SEGMENT_S + PLAY_START_S), () => evaluate(a, `(() => { const s = play.tiles[st.id].media; return !!s.index && covers(s.index, wallOf(nowM()) / 1e6 - s.base - s.shift) && s.video.readyState >= 2; })()`));
    await evaluate(a, `setParty(true); true`);
    start("c", {});
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
    await waitFor(`C has logged ${SHORT_SET_SEGMENTS} segments`, within(SEGMENT_S * (SHORT_SET_SEGMENTS + 1)), () => segsOf(logOf(c, ctx.room, c.id), ctx.set, "v").length >= SHORT_SET_SEGMENTS);
  });

  await step("stopping a recording opens it from the start for everyone in the Watch Party, and leaves a PC already watching that recording where it is", async () => {
    const all = [a, b, c];
    await evaluate(a, `setParty(true); openLive(); selectSet(null); true`);
    await evaluate(b, `setParty(true); openNow(); true`);
    await evaluate(c, `setParty(false); openSession(${JSON.stringify(ctx.set)}); setClock(false, ${STAY_AT_S}); true`);
    for (const app of [a, b]) check(await evaluate(app, `page === "player" && showingNow()`), `${app.key} does not show Live before the stop`);
    await invoke(a, "stop_recording");
    await allSee(all, "the set ended and everyone has it", within((SEGMENT_S + SYNC_ROUND_S) * 3), (st) => !st.active_set && setOf(st, ctx.set) && !setOf(st, ctx.set).live && heldByAll(st, ctx.set, all));
    for (const app of [a, b]) {
      await waitFor(`${app.key} opens the ended recording at 0:00`, within(UI_REFRESH_S * 3), () => evaluate(app, `selected === ${JSON.stringify(ctx.set)} && nowM() === 0 && !clock.playing`));
      await waitFor(`${app.key} is in the Watch Party of the recording`, within(PARTY_REPEAT_S + UI_REFRESH_S * 2), async () => (await state(app)).party.shared.set === ctx.set);
    }
    check(await evaluate(c, `selected === ${JSON.stringify(ctx.set)} && !clock.playing && Math.abs(nowM() - ${STAY_AT_S}) < 0.5`), "C was moved off the place it was watching in the recording");
    await evaluate(c, `setParty(true); true`);
    const ended = setOf(await state(a), ctx.set);
    ctx.durationS = (ended.end_us - ended.start_us) / 1e6;
  });

  await step("each ended recording gets a thumbnail, and the sidebar lists recordings with it", async () => {
    await waitFor("A makes the thumbnail of the set", within(30), async () => setOf(await state(a), ctx.set)?.thumb === true);
    const size = await evaluate(a, `new Promise((done) => { const i = new Image(); i.onload = () => done([i.naturalWidth, i.naturalHeight]); i.onerror = () => done(null); i.src = thumbSrc(setById(${JSON.stringify(ctx.set)})); })`);
    check(size && size[0] > 0 && size[1] > 0, `the page could not load the thumbnail: ${brief(size)}`);
    const file = fs.readFileSync(path.join(roomDir(a, ctx.room), "thumbs", `${ctx.set}.jpg`));
    check(file[0] === 0xff && file[1] === 0xd8 && file.length > 500, "the thumbnail file is not a JPEG");
    await waitFor("A's sidebar lists every recording with a thumbnail", within(UI_REFRESH_S * 5), () => evaluate(a, `!!$("sessions").querySelector(".srow:not(.liverow) .thumb img") && !$("recsopen") && $("sessions").querySelectorAll(".srow:not(.liverow)").length === recordingRows().length`));
    await evaluate(a, `$("sidenav").querySelector("[data-nav=members]").click(); true`);
    await waitFor("A shows the members", within(UI_REFRESH_S * 3), () => evaluate(a, `navActive() === "members" && !$("roomtab").hidden`));
    await screenshot(a, "room-members");
  });

  await step("an Apex matches job for the set is claimed once and finished", async () => {
    const all = [a, b, c];
    const sizes = Object.fromEntries(all.map((owner) => [owner.key, fileSize(logFile(owner, ctx.room, owner.id))]));
    await waitFor("every member pulls every log again", within(REQUEST_TIMEOUT_S + DIAL_S + LOG_SYNC_S), () => all.every((holder) => all.every((owner) => fileSize(logFile(holder, ctx.room, owner.id)) >= sizes[owner.key])));
    await invoke(a, "set_room_plugin", { id: PLUGIN, on: true });
    const seconds = within(JOB_CHECK_S + LOG_SYNC_S + ctx.durationS / KEYFRAME_DECODE_SPEED);
    const analysed = async (app) => setOf(await state(app), ctx.set)?.analysis?.find((x) => x.plugin === PLUGIN)?.state;
    await Promise.all(all.map((app) => waitFor(`${app.key}: the set's analysis done`, seconds, async () => (await analysed(app)) === "done")));
    await sleep(LOG_SYNC_S * 2 * 1000);
    const runs = all.flatMap((owner) => logOf(a, ctx.room, owner.id).filter((r) => r.k === "job" && r.plugin === PLUGIN).map((r) => ({ ...r, by: owner.key })));
    for (const key of new Set(runs.map((r) => r.job))) {
      const claims = runs.filter((r) => r.job === key && r.state === "claim");
      const done = runs.filter((r) => r.job === key && r.state === "done");
      check(claims.length === 1 && done.length === 1 && claims[0].run === done[0].run && claims[0].by === a.key, `job ${key}: ${claims.length} claims, ${done.length} done, the runner is A`);
      check(done[0].events === 0, `job ${key} found ${done[0].events} matches in the test picture`);
    }
    check(new Set(runs.map((r) => r.job)).size === 1, `the set had ${new Set(runs.map((r) => r.job)).size} Apex jobs`);
    ctx.notes.push("one extension job run once, by the member who turned it on");
    const shown = (await state(b)).events.filter((e) => e.plugin);
    check(!shown.length, `extension events shown: ${brief(shown)}`);
  });

  await step("the runner of an extension is chosen and a rerun is processed there", async () => {
    const all = [a, b, c];
    const done = (app) => logOf(a, ctx.room, app.id).filter((r) => r.k === "job" && r.state === "done" && r.plugin === PLUGIN).length;
    await invoke(b, "set_plugin_runner", { id: PLUGIN, runner: b.id });
    for (const app of all) await waitFor(`${app.key} sees B as the runner`, within(LOG_SYNC_S * 3), async () => (await invoke(app, "room_plugins")).find((x) => x.id === PLUGIN).runner === b.id);
    await sleep(JOB_CHECK_S * 2 * 1000);
    check(done(b) === 0 && done(c) === 0, "choosing the runner analysed the recording again");
    const seconds = within(JOB_CHECK_S + LOG_SYNC_S * 2 + ctx.durationS / KEYFRAME_DECODE_SPEED);
    await invoke(a, "retry_job", { room: ctx.room, set: ctx.set, plugin: PLUGIN });
    await waitFor("B processes the rerun", seconds, async () => done(b) > 0);
    for (const app of all) await waitFor(`${app.key} shows the set analysed again`, seconds, async () => setOf(await state(app), ctx.set)?.analysis?.find((x) => x.plugin === PLUGIN)?.state === "done");
    await invoke(b, "set_plugin_runner", { id: PLUGIN, runner: a.id });
    for (const app of all) await waitFor(`${app.key} sees A as the runner again`, within(LOG_SYNC_S * 3), async () => (await invoke(app, "room_plugins")).find((x) => x.id === PLUGIN).runner === a.id);
  });

  await step("the runner of an extension is chosen from the members in its row", async () => {
    await evaluate(b, `openExtensions.add(${JSON.stringify(PLUGIN)}); roomTab = "extensions"; go("room"); pollPlugins(); true`);
    const runner = `[...$("roomtab").querySelectorAll(".plug")].find((p) => p.querySelector(".pt b").textContent === t("Apex matches"))?.querySelector("select[aria-label='" + t("Runner") + "']")`;
    await waitFor("B shows the runner choice", within(UI_REFRESH_S * 5), () => evaluate(b, `!!${runner}`));
    const choices = await evaluate(b, `[...${runner}.options].map((o) => o.textContent).join()`);
    check(choices.split(",").sort().join() === [a.name, b.name, c.name].sort().join(), `the runner choices are ${choices}`);
    await invoke(b, "dismiss_recovery");
    await screenshot(b, "plugins");
    await evaluate(b, `(() => { const s = ${runner}; s.value = ${JSON.stringify(c.id)}; s.dispatchEvent(new Event("change")); return true; })()`);
    await waitFor("B asks before changing who runs it", within(UI_REFRESH_S * 5), () => evaluate(b, `$("confirm").open`));
    await evaluate(b, `$("confirmyes").click(); true`);
    for (const app of [a, b, c]) await waitFor(`${app.key} sees C as the runner`, within(LOG_SYNC_S * 3), async () => (await invoke(app, "room_plugins")).find((x) => x.id === PLUGIN).runner === c.id);
    await invoke(b, "set_plugin_runner", { id: PLUGIN, runner: a.id });
    for (const app of [a, b, c]) await waitFor(`${app.key} sees A as the runner`, within(LOG_SYNC_S * 3), async () => (await invoke(app, "room_plugins")).find((x) => x.id === PLUGIN).runner === a.id);
  });

  await step("the room menu lists the room commands and a rename in its settings reaches everyone", async () => {
    await evaluate(a, `$("sidenav").querySelector("[data-nav=board]").click(); true`);
    await waitFor("A shows the game plan in the main area", within(UI_REFRESH_S * 3), () => evaluate(a, `!$("boardpane").hidden && !$("roompage").hidden && navActive() === "board"`));
    for (const [by, name] of [[b, "Drill room"], [a, "Practice"]]) {
      await invoke(by, "rename_room", { name });
      for (const app of [a, b, c]) await waitFor(`${app.key} shows ${name}`, within(LOG_SYNC_S * 2), async () => (await state(app)).room.name === name);
    }
    await waitFor("the sidebar header shows the room name as a menu", within(UI_REFRESH_S * 5), () => evaluate(a, `$("roomname").querySelector(".nm").textContent === "Practice" && $("roomname").getAttribute("aria-haspopup") === "menu"`));
    await evaluate(a, `$("roomname").click(); true`);
    const labels = await evaluate(a, `[...$("menu").querySelectorAll("button")].map((b) => b.textContent)`);
    check(labels.filter((l) => l !== "Cancel invite").join() === ["Room settings…", "Copy invite code", "AI", "Leave room"].join(), `the room menu lists ${labels}`);
    await evaluate(a, `[...$("menu").querySelectorAll("button")].find((b) => b.textContent === t("Room settings…")).click(); true`);
    await waitFor("A shows the room settings", within(UI_REFRESH_S * 3), () => evaluate(a, `$("roomdlg").open && $("roomnameinput").value === "Practice" && $("roomsave").disabled`));
    await screenshot(a, "room-settings");
    await evaluate(a, `(() => { const i = $("roomnameinput"); i.value = "Scrim room"; i.dispatchEvent(new Event("input")); $("roomsave").click(); return true; })()`);
    for (const app of [a, b, c]) await waitFor(`${app.key} shows the name set in the room settings`, within(LOG_SYNC_S * 2), async () => (await state(app)).room.name === "Scrim room");
    await waitFor("A's header shows the new name", within(UI_REFRESH_S * 3), () => evaluate(a, `$("roomname").querySelector(".nm").textContent === "Scrim room"`));
    await invoke(a, "rename_room", { name: "Practice" });
    for (const app of [a, b, c]) await waitFor(`${app.key} shows Practice again`, within(LOG_SYNC_S * 2), async () => (await state(app)).room.name === "Practice");
  });

  await step("B's chosen colour and built-in avatar and A's room icon reach everyone", async () => {
    const icon = "data:image/webp;base64,UklGRhoAAABXRUJQVlA4TA0AAAAvAAAAEAcQERGIiP4HAA==";
    await invoke(b, "set_profile", { color: 5, avatar: "bolt" });
    await invoke(a, "set_room_icon", { icon });
    for (const app of [a, b, c]) await waitFor(`${app.key} shows B's colour and avatar`, within(LOG_SYNC_S * 2), async () => { const s = await state(app); return s.colors[b.id] === 5 && s.avatars[b.id] === "bolt"; });
    for (const app of [a, b, c]) await waitFor(`${app.key} shows the room icon in the room and in the list`, within(LOG_SYNC_S * 2), async () => { const s = await state(app); return s.room.icon === icon && s.rooms.find((r) => r.id === s.room.id).icon === icon; });
    await waitFor("A's sidebar shows the icon as an image", within(UI_REFRESH_S * 3), () => evaluate(a, `!!$("roomicon").querySelector("img")`));
    await invoke(b, "set_profile", { avatar: "" });
    await invoke(a, "set_room_icon", { icon: "" });
    for (const app of [a, b, c]) await waitFor(`${app.key} shows no avatar for B and no room icon`, within(LOG_SYNC_S * 2), async () => { const s = await state(app); return !s.avatars[b.id] && s.room.icon === ""; });
  });

  await step("the game plan is a board of cards with short Markdown, and a card is added, renamed, moved, resized and deleted by anyone", async () => {
    const text = ["Win the ring", "- first", "- second", `==hold== [Opening](ezh://moment/${encodeURIComponent(ctx.set)}/5/all) [site](https://example.com/x) [bad](javascript:alert(1)) <img src=x onerror=alert(1)>`, "| Who | Job |"].join("\n");
    await evaluate(a, `showRoomTab("board"); true`);
    await invoke(a, "put_board_card", { room: ctx.room, card: "team", text });
    const card = async (app, id) => (await state(app)).board.find((x) => x.card === id);
    for (const app of [b, c]) await waitFor(`${app.key} receives the team card`, within(LOG_SYNC_S * 2), async () => (await card(app, "team"))?.text === text);
    await waitFor("A shows the cards", within(UI_REFRESH_S * 5), () => evaluate(a, `!!$("canvas").querySelector(".bcard.team .chead") && $("canvas").querySelectorAll(".brow > .bcard:not(.team)").length === 3`));
    const dom = await evaluate(a, `(() => { const c = $("canvas").querySelector(".bcard.team"); return { head: c.querySelector(".chead").textContent, items: c.querySelectorAll("li").length, marks: c.querySelectorAll("mark").length, boxes: c.querySelectorAll("input,table,pre").length, links: [...c.querySelectorAll("a")].map((x) => [x.getAttribute("href"), x.title]), moments: c.querySelectorAll("button.link").length, images: c.querySelectorAll("img").length, text: c.textContent, writes: $("canvas").querySelectorAll(".write").length }; })()`);
    check(dom.head === "Win the ring" && dom.items === 2 && dom.marks === 1 && dom.boxes === 0 && dom.moments === 1 && dom.images === 0 && dom.links.length === 1 && dom.links[0][0] === null && dom.links[0][1] === "https://example.com/x", `the team card shows ${brief(dom)}`);
    check(dom.text.includes("javascript:alert(1)") && dom.text.includes("<img src=x onerror=alert(1)>") && dom.text.includes("| Who | Job |"), "the unsafe link, the tag and the table are not shown as plain text");
    check(dom.writes === 3, `the empty member cards show ${dom.writes} Write buttons`);
    await screenshot(a, "board-room");
    const id = await invoke(b, "put_board_card", { room: ctx.room, card: "", title: "Drops", text: "north" });
    for (const app of [a, b, c]) await waitFor(`${app.key} sees the new card after the member cards`, within(LOG_SYNC_S * 2), async () => { const x = await card(app, id); return x?.title === "Drops" && x.text === "north" && (await state(app)).board.at(-1).card === id; });
    await invoke(c, "put_board_card", { room: ctx.room, card: id, title: "Calls" });
    await waitFor("A has C's rename before it moves the card", within(LOG_SYNC_S * 2), async () => (await card(a, id))?.title === "Calls");
    await invoke(a, "put_board_card", { room: ctx.room, card: id, pos: -0.5 });
    for (const app of [a, b, c]) await waitFor(`${app.key} sees the card renamed and moved up with its text`, within(LOG_SYNC_S * 2), async () => { const x = await card(app, id); return x?.title === "Calls" && x.text === "north" && (await state(app)).board[1].card === id; });
    await invoke(c, "put_board_card", { room: ctx.room, card: id, w: 6, h: 3 });
    for (const app of [a, b, c]) await waitFor(`${app.key} sees the card resized with its title, text and place`, within(LOG_SYNC_S * 2), async () => { const x = await card(app, id); return x?.w === 6 && x.h === 3 && x.title === "Calls" && x.text === "north" && (await state(app)).board[1].card === id; });
    await waitFor("A shows the card six columns wide and three rows high", within(UI_REFRESH_S * 5), () => evaluate(a, `(() => { const box = $("canvas").querySelectorAll(".brow > .bcard")[1]; return box.style.gridColumn === "span 6" && box.style.gridRow === "span 3"; })()`));
    await invoke(b, "put_board_card", { room: ctx.room, card: id, gone: true });
    for (const app of [a, b, c]) await waitFor(`${app.key} no longer shows the deleted card`, within(LOG_SYNC_S * 2), async () => !(await card(app, id)));
    check((await invoke(a, "put_board_card", { room: ctx.room, card: "elsewhere", text: "x" }).catch((e) => e.message || "refused")) !== "elsewhere", "an invalid card id was accepted");
    await invoke(a, "set_fullscreen", { on: true, window: "main" });
    await invoke(a, "set_fullscreen", { on: false, window: "main" });
    check(await invoke(a, "set_fullscreen", { on: true, window: "nowhere" }).then(() => false, () => true), "a window that does not exist went full screen");
    await invoke(a, "put_board_card", { room: ctx.room, card: "team", text: "" });
    await waitFor("A shows no text on the cleared team card", within(UI_REFRESH_S * 5), () => evaluate(a, `$("canvas").querySelectorAll(".write").length === 4`));
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
    await waitFor("A's header truncates the long name", within(LOG_SYNC_S * 2), () => evaluate(a, `(() => { const n = $("roomname").querySelector(".nm"); return n.textContent.length === ${ROOM_NAME_CHARS} && n.scrollWidth > n.clientWidth && getComputedStyle(n).textOverflow === "ellipsis" && $("side").scrollWidth <= $("side").clientWidth + 1; })()`));
    await screenshot(a, "sidebar-narrow");
    await invoke(a, "rename_room", { name: "Practice" });
    await evaluate(a, `setSideWidth(${264}); true`);
    const bar = await evaluate(a, `(() => { const r = $("splitter").getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; })()`);
    await mouse(a, "mouseMoved", bar, 0);
    await mouse(a, "mousePressed", bar, 1);
    for (let i = 1; i <= 4; i++) await mouse(a, "mouseMoved", [bar[0] + 15 * i, bar[1]], 1);
    await mouse(a, "mouseReleased", [bar[0] + 60, bar[1]], 0);
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
    check(await evaluate(a, `!$("sessions").querySelector(".liverow") && !$("startbtn").querySelector(".recdot")`), "the idle list holds a live entry or the start box says it records");
    await evaluate(a, `openLive(); true`);
    await waitFor("A shows the live tiles while idle", within(UI_REFRESH_S * 10), () => evaluate(a, `!$("livetiles").hidden && Object.keys(rec.tiles).length === 3 && navActive() === "live"`));
    await screenshot(a, "live-idle");
    const ownText = `(() => { const tl = rec.tiles[st.id]; return tl ? tl.what.textContent + "|" + tl.problem.textContent : null; })()`;
    const before = await evaluate(a, ownText);
    await evaluate(a, `go("room"); true`);
    await evaluate(a, `openLive(); true`);
    await waitFor("A's own live tile says the same after another page", within(UI_REFRESH_S * 10), async () => (await evaluate(a, ownText)) === before);
    check(rows.length === 3 && rows.every((x) => x === "Standby"), `member states ${brief(rows)}`);
  });

  await step("measuring the speed to a member gives up and down in Mbit/s, the round trip and the path", async () => {
    const speed = await invoke(a, "measure_speed", { id: b.id });
    check(speed.up_mbps > 0 && speed.down_mbps > 0 && speed.rtt_ms >= 0 && speed.path === "direct", `speed ${brief(speed)}`);
    await evaluate(a, `roomTab = "members"; drawerFor = ${JSON.stringify(b.id)}; go("room"); true`);
    await waitFor("the member pane has Measure speed", within(UI_REFRESH_S * 5), () => evaluate(a, `[...$("drawer").querySelectorAll("button")].some((x) => x.textContent === t("Measure speed"))`));
    await evaluate(a, `drawerFor = null; renderRoom(); true`);
  });

  await step("a relay server from the settings is tried first and a dead one is skipped", async () => {
    const relayHost = async () => (await invoke(a, "debug_flow")).relay || "";
    const refused = await invoke(a, "set_relays", { urls: ["relay"] }).then(() => null, (e) => String(e.message || e));
    check(refused && refused.includes("Not a relay URL: relay"), `a non-URL was taken: ${brief(refused)}`);
    await invoke(a, "set_relays", { urls: ["https://127.0.0.1:9/", "https://euc1-1.relay.n0.iroh.link./"] });
    await waitFor("A's relay is the second server of its list", within(RELAY_WAIT_S * 3), async () => (await relayHost()).startsWith("euc1-1."));
    check(connected(await state(a), [b.id, c.id]), "A lost a member while it changed relays");
    check((await state(a)).relays.length === 2, "the relay list is not kept");
    await invoke(a, "set_relays", { urls: [] });
    await waitFor("A is back on the public relays", within(RELAY_WAIT_S * 3), async () => logLines(a).some((l) => l.includes("relay: the public relays")));
    check(connected(await state(a), [b.id, c.id]), "A lost a member after the relay list was cleared");
  });

  await step("Watch Party converges on a seek and a play, and a member who joins midway converges too", async () => {
    const all = [a, b, c];
    const seconds = within(PARTY_REPEAT_S + UI_REFRESH_S * 2);
    await Promise.all(all.map((app) => evaluate(app, `openSession(${JSON.stringify(ctx.set)}); true`)));
    for (const app of all) await waitFor(`${app.key} watches the set`, seconds, async () => (await state(app)).party.shared.set === ctx.set);
    await evaluate(c, `setParty(false); true`);
    await waitFor("C leaves the party", seconds, async () => !(await state(c)).party.on);
    await evaluate(a, `act("seek", ${SEEK_TO_S}); true`);
    await follow(ctx, a, [b], false, SEEK_TO_S);
    await evaluate(b, `act("play", nowM()); true`);
    await follow(ctx, b, [a], true, "nowM()");
    check(await evaluate(c, `!clock.playing`), "C moved with a party it had left");
    await evaluate(c, `setParty(true); true`);
    await follow(ctx, b, [c], true, "nowM()");
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

  await step("leaving and joining the Watch Party, going to Live and back, and the starter leaving keep everyone's place", async () => {
    const all = [a, b, c];
    const at = (app) => evaluate(app, `({ set: selected, playing: clock.playing, m: nowM(), layout: [play.mode, play.keys.join()] })`);
    await evaluate(a, `act("seek", ${SEEK_TO_S}); act("play", ${SEEK_TO_S}); true`);
    await follow(ctx, a, [b, c], true, "nowM()");
    const shown = await at(c);
    await evaluate(c, `setParty(false); true`);
    await waitFor("C leaves the party", within(UI_REFRESH_S * 3), async () => !(await state(c)).party.on);
    await sleep(UI_REFRESH_S * 1000);
    const left = await at(c);
    check(left.set === ctx.set && left.playing && left.m >= shown.m && left.layout.join() === shown.layout.join(), `leaving moved C: ${brief([shown, left])}`);
    const paused = await evaluate(c, `act("pause", nowM()); nowM()`);
    await evaluate(c, `openLive(); true`);
    await evaluate(c, `openSession(${JSON.stringify(ctx.set)}); true`);
    const back = await at(c);
    check(back.set === ctx.set && !back.playing && Math.abs(back.m - paused) < JUMP_MIN_S, `C came back to another place: ${brief([paused, back])}`);
    const before = await Promise.all([a, b].map(at));
    await evaluate(c, `setParty(true); true`);
    await follow(ctx, a, [c], true, "nowM()");
    const after = await Promise.all([a, b].map(at));
    check(after.every((x, i) => x.playing && x.m >= before[i].m), `C joining late moved the others: ${brief([before, after])}`);
    await evaluate(a, `setParty(false); true`);
    await sleep(UI_REFRESH_S * 2000);
    const going = await Promise.all([b, c].map(at));
    check(going.every((x) => x.playing && x.m >= after[0].m), `the party stopped when A left: ${brief(going)}`);
    await evaluate(a, `setParty(true); true`);
    await waitFor("A takes the running party again", within(PARTY_REPEAT_S + UI_REFRESH_S * 2), async () => (await state(a)).party.shared.seq > 0);
    await follow(ctx, a, [b, c], true, "nowM()");
    const clocks = await Promise.all(all.map((app) => evaluate(app, "nowM()")));
    check(Math.max(...clocks) - Math.min(...clocks) < JUMP_MIN_S && Math.min(...clocks) >= SEEK_TO_S, `clocks after the rejoins: ${brief(clocks)}`);
    await evaluate(a, `act("pause", nowM()); true`);
  });

  await step("a far seek and an immediate Play start every tile at once, alone and in Watch Party", async () => {
    const far = Math.max(0, ctx.durationS - FAR_FROM_END_S);
    await Promise.all([a, b, c].map((app) => evaluate(app, `openSession(${JSON.stringify(ctx.set)}); true`)));
    await evaluate(a, `act("seek", ${far}); act("play", ${far}); true`);
    await together(a, [far - 1, Infinity]);
    await follow(ctx, a, [b, c], true, "nowM()");
    await Promise.all([b, c].map((app) => together(app, [far - 1, Infinity])));
    await evaluate(b, `act("seek", 1); act("play", 1); true`);
    await follow(ctx, b, [a, c], true, "nowM()");
    await Promise.all([a, c].map((app) => together(app, [0, far / 2])));
    await evaluate(a, `act("pause", nowM()); true`);
  });

  const clickAt = async (app, selector) => {
    const at = await evaluate(app, `(() => { const r = ${selector}.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; })()`);
    await mouse(app, "mouseMoved", at, 0);
    await mouse(app, "mousePressed", at, 1);
    await mouse(app, "mouseReleased", at, 0);
  };
  const press = (app, key, code, keyCode) => send(app, "Input.dispatchKeyEvent", { type: "keyDown", key, code, windowsVirtualKeyCode: keyCode, text: key === " " ? " " : undefined }).then(() => send(app, "Input.dispatchKeyEvent", { type: "keyUp", key, code, windowsVirtualKeyCode: keyCode }));
  const openTiles = async (app) => {
    await evaluate(app, `openSession(${JSON.stringify(ctx.set)}); true`);
    await waitFor(`${app.key} shows the tiles`, within(UI_REFRESH_S * 5), () => evaluate(app, `page === "player" && play.keys.length >= 2 && play.keys.every((k) => play.tiles[k] && play.tiles[k].box.getBoundingClientRect().width > 0)`));
    await evaluate(app, `setView(play, 0); act("pause", nowM()); true`);
  };

  await step("live speaking leaves the player alone and a played voice lights its tile", async () => {
    await openTiles(a);
    await evaluate(a, `play.keys.forEach((k) => setSpeaking(k, true)); true`);
    check(await evaluate(a, `!$("playtiles").querySelector("[data-speak], .speaking")`), "a live speaking message lit a player tile");
    await evaluate(a, `play.keys.forEach((k) => setSpeaking(k, false)); act("seek", 1); act("play", 1); true`);
    const at = (expression) => evaluate(a, `play.keys.map((k) => play.tiles[k]).map((tl) => ${expression})`);
    const from = await at("[tl.media.video.currentTime, tl.mic && tl.mic.video.currentTime]");
    await waitFor("every voice moves with its media", within(UI_REFRESH_S * 5), async () => {
      const now = await at("[tl.media.video.currentTime, tl.mic && tl.mic.video.currentTime]");
      const moved = now.filter(([m], i) => m > from[i][0] + 1);
      return moved.length > 0 && now.every(([m, v], i) => m <= from[i][0] + 1 || v === null || v > from[i][1] + 1);
    });
    await evaluate(a, `act("pause", nowM()); true`);
  });

  await step("clicking a tile, or Hear this in its right-click menu, picks whose app sound plays and clicks keep no focus", async () => {
    await openTiles(a);
    const pick = await evaluate(a, `play.keys.find((k) => k !== soundKey(0, play.keys, st.id, gameSound))`);
    await clickAt(a, `play.tiles[${JSON.stringify(pick)}].box`);
    await waitFor("the clicked tile's sound plays", within(UI_REFRESH_S * 3), () => evaluate(a, `soundKey(0, play.keys, st.id, gameSound) === ${JSON.stringify(pick)} && play.keys.every((k) => (k === ${JSON.stringify(pick)}) === !play.tiles[k].media.video.muted && (k === ${JSON.stringify(pick)}) === !play.tiles[k].snd.hidden) && $("gamesound").title === t("{name} app sound is playing", { name: nameOf(${JSON.stringify(pick)}) })`));
    check(await evaluate(a, `[...$("views").children].every((b, i) => (b.querySelector(".key") === null) === (i > MAX_KEYED) && (i > MAX_KEYED || b.querySelector(".key").textContent === String(i)))`), "the view buttons show their shortcut numbers");
    const next = await evaluate(a, `play.keys[(play.keys.indexOf(${JSON.stringify(pick)}) + 1) % play.keys.length]`);
    await evaluate(a, `play.tiles[${JSON.stringify(next)}].box.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 80, clientY: 120 })); true`);
    await clickAt(a, `[...$("menu").querySelectorAll("button")].find((b) => b.textContent === t("Hear this"))`);
    await waitFor("Hear this in the tile menu picks that member", within(UI_REFRESH_S * 3), () => evaluate(a, `soundKey(0, play.keys, st.id, gameSound) === ${JSON.stringify(next)}`));
    const third = await evaluate(a, `play.keys[(play.keys.indexOf(${JSON.stringify(next)}) + 1) % play.keys.length]`);
    await clickAt(a, `$("gamesound")`);
    await clickAt(a, `[...$("pop").querySelectorAll(".who")][play.keys.indexOf(${JSON.stringify(third)})].querySelector("button")`);
    await waitFor("the app sound popup picks the member whose button was clicked", within(UI_REFRESH_S * 3), () => evaluate(a, `soundKey(0, play.keys, st.id, gameSound) === ${JSON.stringify(third)}`));
    check(await evaluate(a, `$("pop").querySelectorAll(".poprow").length === play.keys.length * 2`), "the app sound popup lacks the per-member volumes");
    await evaluate(a, `closePop(); true`);
    for (const id of ["volume", "play"]) {
      await clickAt(a, `$(${JSON.stringify(id)})`);
      check(await evaluate(a, `document.activeElement === document.body`), `a click on ${id} kept the keyboard focus`);
    }
    await press(a, " ", "Space", 32);
    await waitFor("Space pauses once", within(UI_REFRESH_S * 3), () => evaluate(a, `!clock.playing`));
    const before = await evaluate(a, `nowM()`);
    await press(a, "ArrowRight", "ArrowRight", 39);
    await waitFor("the right arrow moves 5 s", within(UI_REFRESH_S * 3), async () => Math.abs((await evaluate(a, `nowM()`)) - before - 5) < 0.5);
  });

  await step("dragging a name label swaps two tiles", async () => {
    await openTiles(a);
    const before = await evaluate(a, `play.keys.join()`);
    const pts = await evaluate(a, `(() => { const [k0, k1] = play.keys; const l = play.tiles[k0].lab.getBoundingClientRect(); const t = play.tiles[k1].box.getBoundingClientRect(); return { from: [l.left + l.width / 2, l.top + l.height / 2], to: [t.left + t.width / 2, t.top + t.height / 2] }; })()`);
    await mouse(a, "mouseMoved", pts.from, 0);
    await mouse(a, "mousePressed", pts.from, 1);
    for (let i = 1; i <= 5; i++) await mouse(a, "mouseMoved", [pts.from[0] + ((pts.to[0] - pts.from[0]) * i) / 5, pts.from[1] + ((pts.to[1] - pts.from[1]) * i) / 5], 1);
    await mouse(a, "mouseReleased", pts.to, 0);
    const [x0, x1, ...rest] = before.split(",");
    await waitFor("dragging a label swaps the two tiles", within(UI_REFRESH_S * 3), () => evaluate(a, `play.keys.join() === ${JSON.stringify([x1, x0, ...rest].join())}`));
    await screenshot(a, "player-swapped");
    await evaluate(a, `go("room"); true`);
  });

  await step("a session started while watching together records voices and the Watch Party", async () => {
    const all = [a, b, c];
    await allSee(all, "watching the set together", within(PARTY_REPEAT_S + UI_REFRESH_S * 2), (st) => st.party.on && st.party.shared.set === ctx.set);
    await waitFor("A's start button offers the debrief while watching together", within(UI_REFRESH_S * 3), () => evaluate(a, `$("startbtn").textContent === t("Record debrief")`));
    for (const app of [b, c]) await evaluate(app, `window.e2eNotes = []; window.__TAURI__.event.listen("session", (e) => e2eNotes.push(sessionNotice(e.payload.action, e.payload.by))).then(() => true)`);
    await pressAndConfirm(a, "toggleRecording()", false);
    await sleep(UI_REFRESH_S * 1000);
    check(!(await state(a)).active_set, "A started recording although it answered No");
    const asked = await pressAndConfirm(a, "toggleRecording()");
    check(asked === "Record the debrief for everyone?", `A was asked "${asked}"`);
    await waitFor("B is told who started", within(LOG_SYNC_S * 2), () => evaluate(b, `e2eNotes.includes(${JSON.stringify(a.name + " started recording")})`));
    ctx.review = (await state(a)).active_set.id;
    await allSee(all, "recording the review voice only", within(LOG_SYNC_S * 2), (st) => st.active_set?.id === ctx.review && st.active_set.voice === true);
    await waitFor(`A logs ${SEGMENTS_AFTER_RESTART} voice segments of the review`, within(SEGMENT_S * (SEGMENTS_AFTER_RESTART + 2)), () => segsOf(logOf(a, ctx.room, a.id), ctx.review, "m").length >= SEGMENTS_AFTER_RESTART);
    await evaluate(c, `act("seek", ${SEEK_TO_S * 2}); true`);
    await waitFor("C logs its seek as a party record", within(LOG_SYNC_S * 2), () => logOf(c, ctx.room, c.id).some((r) => r.k === "party" && r.set === ctx.review && r.shared.set === ctx.set));
    await evaluate(c, `openLive(); selectSet(null); true`);
    const ending = await pressAndConfirm(a, "toggleRecording()");
    check(ending === "Stop recording for everyone?", `A was asked "${ending}"`);
    await waitFor("B is told who ended", within(LOG_SYNC_S * 2), () => evaluate(b, `e2eNotes.includes(${JSON.stringify(a.name + " stopped recording")})`));
    await allSee(all, "the review ended", within(LOG_SYNC_S * 2), (st) => !st.active_set && setOf(st, ctx.review) && !setOf(st, ctx.review).live);
    await waitFor("C is told who ended", within(LOG_SYNC_S * 2), () => evaluate(c, `e2eNotes.includes(${JSON.stringify(a.name + " stopped recording")})`));
    for (const app of [a, b]) check(await evaluate(app, `selected === ${JSON.stringify(ctx.set)}`), `${app.key} was moved when the debrief ended`);
    check(await evaluate(c, `page === "player" && selected === null`), "C on Live was moved when the debrief ended");
    for (const app of all) {
      const own = logOf(app, ctx.room, app.id);
      check(segsOf(own, ctx.review, "m").length > 0 && segsOf(own, ctx.review, "v").length === 0, `${app.key} recorded ${segsOf(own, ctx.review, "v").length} picture and ${segsOf(own, ctx.review, "m").length} voice segments in the review`);
    }
    check(logOf(a, ctx.room, a.id).some((r) => r.k === "party" && r.set === ctx.review), "A did not log what was watched when the review started");
    for (const app of all) await waitFor(`${app.key} links the review to the recording it was about`, within(LOG_SYNC_S * 2), async () => setOf(await state(app), ctx.review)?.of === ctx.set);
    await waitFor("A lists the recording and not its debrief, which the recording's panel lists", within(UI_REFRESH_S * 3), () => evaluate(a, `recordingRows().every((r) => r.id !== ${JSON.stringify(ctx.review)}) && debriefsOf(${JSON.stringify(ctx.set)}).length === 1`));
    await evaluate(a, `openSession(${JSON.stringify(ctx.set)}); true`);
    await waitFor("A's panel lists the debrief with its start, length and Save under the timeline", within(UI_REFRESH_S * 3), () => evaluate(a, `!$("debriefs").hidden && $("debriefs").querySelectorAll(".drow").length === 1 && $("debriefs").textContent.includes(sessionClock(${JSON.stringify(ctx.review)})) && $("debriefs").textContent.includes(t("Save")) && !$("pchips").textContent.includes(t("Debrief"))`));
    ctx.notes.push(`debrief with ${logOf(a, ctx.room, a.id).filter((r) => r.k === "party" && r.set === ctx.review).length} party records of A linked to its recording`);
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
    const debrief = list.json.recordings.find((r) => r.id === ctx.review);
    check(debrief?.kind === "debrief" && debrief.of === ctx.set && Array.isArray(debrief.watch) && debrief.watch.length >= 2 && debrief.watch.every((w) => w.set === ctx.set && typeof w.at_us === "number") && found.of === undefined && Array.isArray(found.analysis), `the debrief: ${brief(debrief)}`);
    const voice = await api(a, "GET", `${recording(ctx.review)}/voices/${a.id}`, { token });
    check(voice.status === 200 && voice.type?.startsWith("audio/mp4") && voice.raw.subarray(4, 8).toString() === "ftyp", `the debrief voice: ${voice.status} ${voice.type}`);
    check((await api(a, "GET", `${recording(ctx.set)}/voices/${a.id}x`, { token })).status === 404, "a voice that does not exist was served");
    for (const tail of ["events", "chapters", "transcript"]) {
      const r = await api(a, "GET", `${recording(ctx.set)}/${tail}`, { token });
      check(r.status === 200 && Array.isArray(r.json?.[tail === "events" ? "events" : tail === "chapters" ? "chapters" : "lines"]), `${tail}: ${r.status} ${brief(r.text)}`);
    }
    const plugins = await api(a, "GET", `${room}/plugins`, { token });
    check(plugins.status === 200 && plugins.json.plugins.some((p) => p.id === "apex-matches"), `plugins: ${brief(plugins.json)}`);
    check((await api(a, "GET", `${room}/jobs`, { token })).status === 404, "the jobs endpoint is still there");
    const apex = plugins.json.plugins.find((p) => p.id === PLUGIN);
    check(apex.runner === a.id && apex.left === 0 && apex.failed === 0 && found.analysis.some((x) => x.plugin === PLUGIN && x.state === "done"), `the extension: ${brief(apex)} ${brief(found.analysis)}`);
    check((await api(a, "POST", `${recording(ctx.set)}/events`, { token, body: { text: "" } })).status === 400, "an empty note was accepted");
    check((await api(a, "GET", `${recording(ctx.set)}/clips`, { token })).status === 404, "there is an endpoint for clips");
    const text = "Noted through the API #fromapi";
    const made = await api(a, "POST", `${recording(ctx.set)}/events`, { token, body: { text, at_s: SEEK_TO_S, tags: ["api"] } });
    check(made.status === 201 && made.json.at_s === SEEK_TO_S && made.json.tags.join() === "fromapi,api" && made.json.author === a.id, `the note: ${made.status} ${brief(made.text)}`);
    await allSee([a, b, c], "the note written through the API is an AI note", within(LOG_SYNC_S * 2), (st) => st.events.some((e) => e.key === made.json.key && e.text === `${text} #api` && e.kind === "note" && e.source === "ai"));
    const tagged = await api(a, "GET", `${recording(ctx.set)}/events?tag=API`, { token });
    check(tagged.json.events.some((e) => e.key === made.json.key), "the note is not found by its tag");
    await evaluate(a, `openSession(${JSON.stringify(ctx.set)}); true`);
    await waitFor("A lists the note in the player with the AI mark", within(UI_REFRESH_S * 5), () => evaluate(a, `$("reviewlist").textContent.includes("Noted through the API") && !!$("reviewlist").querySelector(".aimark")`));
    const status = await api(a, "GET", "/v1/status", { token });
    check(status.status === 200 && status.json.room === ctx.room && status.json.recording === false && ["idle", "standby"].includes(status.json.state) && status.json.members.length === 3 && status.json.members.some((m) => m.id === a.id && m.state === status.json.state), `status: ${status.status} ${brief(status.json)}`);
    const diag = await api(a, "GET", "/v1/diagnostics?everyone=true", { token });
    const diagText = diag.json && diag.json.text || "";
    check(diag.status === 200 && diagText.includes("=== this PC ===\nez-hanseikai") && diagText.includes("recent log") && diagText.split("=== member ").length === 3, `diagnostics: ${diag.status} ${brief(diagText.slice(0, 200))}`);
    for (const secret of [NAMES.a, NAMES.b, NAMES.c, a.id, b.id]) check(!diagText.includes(secret), `the API report holds ${secret.slice(0, 4)}`);
    const operation = (name, body) => api(a, "POST", `/v1/ops/${name}`, { token, body });
    const opened = await operation("enter_room", { id: ctx.room });
    check(opened.status === 200 && opened.json === null, `enter_room: ${opened.status} ${brief(opened.text)}`);
    check((await operation("enter_room", { id: "nope" })).status === 400 && (await operation("enter_room", {})).json.error === "id is missing or not valid.", "enter_room takes a room that is not in the list");
    check((await operation("nothing", {})).status === 404 && (await api(a, "GET", "/v1/ops/state", { token })).status === 405 && (await operation("state", "[]")).status === 400, "an operation takes any name, method or body");
    check((await operation("state")).json.id === a.id, "an operation without a body is not run as {}");
    const board = "Goal: hold the high ground\n- through the API";
    const put = await api(a, "PUT", `${room}/board`, { token, body: { text: board } });
    check(put.status === 200 && put.json.text === board, `the game plan: ${put.status} ${brief(put.text)}`);
    await allSee([a, b, c], "the game plan written through the API", within(LOG_SYNC_S * 2), (st) => st.events.some((e) => e.kind === "board" && e.text === board));
    check((await api(a, "GET", `${room}/board`, { token })).json.text === board, "the game plan reads back differently");
    await waitFor("A has the game plan as its team card", within(UI_REFRESH_S * 5), () => evaluate(a, `st.board.find((c) => c.card === "team").text === ${JSON.stringify(board)}`));
    await evaluate(a, `$("settings").close(); go("room"); true`);
    await waitFor("A shows the API badge once, beside the connection badge", within(UI_REFRESH_S * 5), () => evaluate(a, `!$("apibadge").hidden && $("apibadge").nextElementSibling === $("netbadge") && document.querySelectorAll(".apibadge").length === 1`));
    await evaluate(a, `$("apibadge").click(); true`);
    check(await evaluate(a, `$("settings").open && settingsPane === "api" && !$("settings").querySelector('section[data-pane="api"]').hidden`), "the badge does not open Settings > API");
    await waitFor("A lists the endpoints", within(UI_REFRESH_S * 5), () => evaluate(a, `$("apiref").querySelectorAll("details").length >= 15 && $("apiref").textContent.includes("/v1/status") && $("apiref").querySelector("pre") !== null`));
    await evaluate(a, `$("settings").close(); go("room"); true`);
    await evaluate(a, `roomMenu($("roomname")); [...$("menu").querySelectorAll("button")].find((b) => b.textContent === t("AI")).click(); true`);
    await waitFor("A opens the AI menu from the room menu", within(UI_REFRESH_S * 5), () => evaluate(a, `!$("menu").hidden && $("menu").querySelectorAll("button").length === 3`));
    await evaluate(a, `$("menu").querySelector("button").click(); true`);
    await waitFor("A shows the prompt before copying", within(UI_REFRESH_S * 5), () => evaluate(a, `$("promptdlg").open && !$("prompttext").readOnly && $("prompttext").value.includes("llms.txt")`));
    check(await evaluate(a, `(() => { const v = $("prompttext").value; return !v.includes(promptToken) && v.includes("Token: ••••••••") && aiCopyText(v, promptToken).includes("Token: " + promptToken) && v.startsWith(promptPurpose.task + "\\n\\n"); })()`), "the prompt box is one text with the masked token and the task first");
    await screenshot(a, "ai-prompt");
    await evaluate(a, `$("promptdlg").close(); true`);
    const prompt = await evaluate(a, `closeMenu(), invoke("api_info").then((i) => aiPrompt(aiPurposes()[0], i, st.room, null))`);
    for (const part of [token, `http://127.0.0.1:${API_PORTS.a}/llms.txt`, ctx.room, "the newest one"]) check(prompt.includes(part), `the prompt lacks ${part === token ? "the token" : part}`);
    await evaluate(a, `openSettings("api"); true`);
    await pressAndConfirm(a, `$("apinew").click()`);
    const fresh = await waitFor("A has a new token", within(UI_REFRESH_S * 5), () => {
      const next = readJson(path.join(a.data, "api.json"))?.token;
      return next !== token && next;
    });
    a.token = fresh;
    check((await api(a, "GET", "/v1", { token })).status === 401 && (await api(a, "GET", "/v1", { token: fresh })).status === 200, "the old token still works or the new one does not");
    check((await invoke(a, "api_info")).token === fresh, "the app keeps the old token");
    await evaluate(a, `$("settings").close(); go("room"); true`);
    const faces = await evaluate(a, `(() => { const was = sideWidth; setSideWidth(clampSide(0), false); const f = $("roomfaces"); const r = { shown: f.querySelectorAll(".av").length, clipped: f.scrollWidth > f.clientWidth }; setSideWidth(was, false); return r; })()`);
    check(faces.shown === 3 && !faces.clipped, `the faces at the narrowest sidebar: ${brief(faces)}`);
    await evaluate(a, `openSettings("api"); true`);
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

  await step("Diagnostics opens on where data goes and copies a masked report", async () => {
    await evaluate(a, `openDiagnostics(); true`);
    await waitFor("Diagnostics shows where data goes", within(UI_REFRESH_S * 5), () => evaluate(a, `!!$("diagbody").querySelector(".flowmap") && $("diagbody").textContent.includes(t("Time check"))`));
    await screenshot(a, "diagnostics");
    const report = await evaluate(a, `diagnosticsText()`);
    check(report.includes("ez-hanseikai") && report.includes("environment check") && report.includes("jobs") && report.includes("recent log") && report.includes("\nhardware: ") && report.includes("\nwhere data goes\n"), "the report lacks its parts");
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

  await step("one member's light export holds only that member's files, and a per-match export of a recording without matches is refused", async () => {
    const out = await invoke(a, "export_folder", { room: ctx.room, set: ctx.set, folder: path.join(a.data, "bundles", `${ctx.set} light`), quality: "light", member: b.id }, EXPORT_FOLDER_S);
    const media = fs.readdirSync(out.folder).filter((f) => /\.(mp4|m4a)$/.test(f)).sort();
    check(out.files === 1 && media.join("|") === `${b.name} voice.m4a|${b.name}.mp4`, `light export: ${brief(out)} ${brief(media)}`);
    const decode = ffmpeg("ffmpeg", ["-v", "error", "-i", path.join(out.folder, `${b.name}.mp4`), "-f", "null", "-"]);
    check(decode.ok && !decode.err, `the light ${b.name}.mp4 does not decode cleanly: ${decode.err.slice(0, 300)}`);
    const folder = path.join(a.data, "bundles", `${ctx.set} matches`);
    check(await invoke(a, "export_folder", { room: ctx.room, set: ctx.set, folder, range: "matches" }, EXPORT_FOLDER_S).then(() => false, () => true) && !fs.existsSync(folder), "a per-match export without matches was accepted");
  });

  const confirmDeletion = async (app, set, form) => {
    await evaluate(app, `confirmDelete([${JSON.stringify(set)}]); true`);
    await waitFor(`${app.key} asks before deleting`, within(UI_REFRESH_S * 5), () => evaluate(app, `$("deletedlg").open`));
    const said = await evaluate(app, `[$("deltitle").textContent, ...[...$("dellist").querySelectorAll(".delcard .small")].map((x) => x.textContent)]`);
    check(said[0] === "Delete this recording?" && said.length === 2 && said[1] === form, `${app.key} says ${brief(said)} instead of "${form}"`);
    await evaluate(app, `$("delyes").click(); true`);
  };
  const mediaLeft = (app, set) => logOf(app, ctx.room, app.id).filter((r) => r.k === "seg" && r.set === set).filter((r) => fs.existsSync(mediaFile(app, ctx.room, set, app.id, r.capture, r.series, segName(r.n)))).length;

  await step("deleting a recording others still hold takes its debrief along and keeps both as not on this PC until Download", async () => {
    const all = [a, b, c];
    const both = [ctx.set, ctx.review];
    const before = await state(b);
    check(both.every((id) => setOf(before, id).shared === true && setOf(before, id).progress.have > 0) && await evaluate(b, `canDelete(setById(${JSON.stringify(ctx.set)}))`), `before deleting: ${brief(both.map((id) => setOf(before, id).progress))}`);
    await evaluate(b, `confirmDelete([${JSON.stringify(ctx.set)}]); true`);
    await waitFor("B asks before deleting", within(UI_REFRESH_S * 5), () => evaluate(b, `$("deletedlg").open`));
    const said = await evaluate(b, `[...$("dellist").querySelectorAll(".delcard .small")].map((x) => x.textContent).join("|") === [t("Others still have it"), t("Debriefs deleted with it: {names}", { names: sessionName(${JSON.stringify(ctx.review)}) })].join("|")`);
    check(said, "B's confirmation does not say others hold it and name the debrief that goes with it");
    await evaluate(b, `$("delyes").click(); true`);
    for (const id of both) {
      await waitFor(`B lists ${id} as not on this PC`, within(UI_REFRESH_S * 5), async () => { const row = setOf(await state(b), id); return row && row.progress.have === 0 && !row.wanted; });
      check(mediaLeft(b, id) === 0 && !fs.existsSync(path.join(roomDir(b, ctx.room), "recordings", id)), `B kept media of ${id} after deleting it`);
      for (const app of [a, c]) check(setOf(await state(app), id).progress.have > 0, `${app.key} lost ${id}`);
    }
    check(!logOf(b, ctx.room, b.id).some((r) => r.k === "set_del"), "B removed a recording for everyone although others hold it");
    await onlyOnDownload([b], a, ctx.set);
    await allSee(all, "everyone still lists both", within(SYNC_ROUND_S), (st) => both.every((id) => !!setOf(st, id)));
    await waitFor("B fetches the recording on request", within((SEGMENT_S + SYNC_ROUND_S) * 3), async () => heldByAll(await state(b), ctx.set, [b]) && !holdEverything([b], ctx.room, ctx.set, all));
    check(setOf(await state(b), ctx.review).progress.have === 0, "B fetched the debrief nobody asked for");
  });

  await step("deleting a recording nobody else holds removes it and its entry everywhere", async () => {
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
    check((await state(a)).sets.find((x) => x.id === bare).shared === false, "A thinks others hold the recording nobody else holds");
    await waitFor("B's page does not offer to delete a recording it does not hold while A does", within(UI_REFRESH_S * 5), () => evaluate(b, `!canDelete(setById(${JSON.stringify(bare)}))`));
    await waitFor("A made the thumbnail", within(30), async () => setOf(await state(a), bare)?.thumb === true);
    await confirmDeletion(a, bare, "This deletes it completely");
    await allSee(all, "the recording nobody else holds gone from the list", within(LOG_SYNC_S * 3), (st) => !setOf(st, bare));
    check(logOf(a, ctx.room, a.id).some((r) => r.k === "set_del" && r.set === bare), "A did not log the removal");
    check(mediaLeft(a, bare) === 0 && !fs.existsSync(path.join(roomDir(a, ctx.room), "recordings", bare)) && !fs.existsSync(path.join(roomDir(a, ctx.room), "thumbs", `${bare}.jpg`)), "A kept media or the thumbnail");
    await waitFor("B's sidebar drops the recording nobody else holds", within(UI_REFRESH_S * 5), () => evaluate(b, `![...document.querySelectorAll("#sessions .srow")].some((r) => r.textContent.includes(sessionName(${JSON.stringify(bare)})))`));
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
    check(title === "Delete old files?", `C asked "${title}"`);
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

import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
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
const JOB_CHECK_S = 2;
const PARTY_REPEAT_S = 4;
const UI_REFRESH_S = 1;
const JUMP_MIN_S = 1;
const SERIES = ["v", "m"];
const PLUGIN = "apex-matches";

const RUNNER_SLOWDOWN = 3;
const COLD_START_S = 60;
const CALL_S = 30;
const POLL_MS = 500;
const SEGMENTS_BEFORE_LATE_JOIN = 2;
const SEGMENTS_AFTER_RESTART = 2;
const LIVE_ROUNDS = 2;
const KEYFRAME_DECODE_SPEED = 10;
const EXPORT_SPEED = 0.5;
const SEEK_TO_S = 5;
const LOG_TAIL_LINES = 15;
const STEADY_S = 20;
const APPROVAL_WAIT_S = LOG_SYNC_S * 3;
const SOURCE = { kind: "display", index: 1 };
const MODEL_BYTES = { base: 147_951_465, medium: 1_533_763_059 };
const WHISPER_BYTES = 8_573_270;
const LOG_LINE_CHARS = 200;
const IMPORT_S = 60;
const EXPORT_FOLDER_S = 120;
const MEDIA_SECONDS = { coach: 8, camera: 6, commentary: 7 };
const PLAN_START = "2026-10-19T12:30:00Z";
const START_TOLERANCE_US = 5e6;

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
  const deadline = Date.now() + seconds * 1000;
  let last;
  for (;;) {
    const gone = exited();
    if (gone) fail(`${gone.key} exited with code ${gone.proc.exitCode} while waiting for: ${label}`);
    try {
      last = await probe();
      if (last) return last;
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

const invoke = (app, cmd, args = {}) => evaluate(app, `window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)})`);
const state = (app) => invoke(app, "state");

async function invokeLong(app, cmd, args, seconds, label) {
  await evaluate(app, `window.e2eLong = null; window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)}).then((r) => { window.e2eLong = { ok: r }; }, (e) => { window.e2eLong = { error: String(e) }; }); true`);
  const done = await waitFor(label, within(seconds), () => evaluate(app, "window.e2eLong"));
  check(!done.error, `${label}: ${done.error}`);
  return done.ok;
}

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
  return {
    "rec-20260101-000000.mp4": "old recording",
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

function seedOld(dir) {
  for (const [name, body] of Object.entries(oldFiles())) {
    fs.mkdirSync(path.dirname(path.join(dir, name)), { recursive: true });
    fs.writeFileSync(path.join(dir, name), body);
  }
  fs.writeFileSync(path.join(dir, "log.old.txt"), "old log");
  fs.writeFileSync(path.join(dir, "settings.json"), JSON.stringify({ limit_mbps: 0, keep_days: 0, source: "display:1", language: "xx", quality: "ultra", obs_port: 4455 }));
  fs.writeFileSync(path.join(dir, "rooms.json"), JSON.stringify({ active: null, list: [{ id: "oldshape", title: "from an older version" }] }));
}

function start(key) {
  const home = path.join(OUT, "apps", key);
  const app = apps[key] || { key, name: NAMES[key], data: path.join(home, "data") };
  if (key === "c" && !apps[key]) seedOld(app.data);
  fs.mkdirSync(app.data, { recursive: true });
  const env = {
    ...process.env,
    EZH_TEST: String(PORTS[key]),
    EZH_API_PORT: String(API_PORTS[key]),
    EZH_DIR: app.data,
    WEBVIEW2_USER_DATA_FOLDER: path.join(home, "webview"),
  };
  app.proc = spawn(EXE, [], { env, stdio: "ignore", windowsHide: true });
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
  app.id = (await state(app)).id;
  return app;
}

async function share(app) {
  check(!(await state(app)).capture.source, `${app.key} starts with a capture source`);
  await invoke(app, "set_source", { source: SOURCE });
}

async function launch(key) {
  const app = await attach(start(key));
  await invoke(app, "set_name", { name: app.name });
  await invoke(app, "set_language", { language: "en" });
  await invoke(app, "set_quality", { id: "light" });
  await share(app);
  say(`${key} is ${app.id.slice(0, 10)}`);
  return app;
}

async function connects(app, peer, since = 0) {
  const view = await invoke(app, "debug_view", { tab: "connections", since, set: null, addresses: false });
  return view.events.filter((e) => e.text.startsWith(peer.id.slice(0, 10)) && e.text.includes(": connected")).length;
}

function api(app, method, route, { token, host, body, connect } = {}) {
  const port = API_PORTS[app.key];
  return new Promise((resolve, reject) => {
    const payload = body === undefined ? null : Buffer.from(typeof body === "string" ? body : JSON.stringify(body));
    const headers = { Host: host || `127.0.0.1:${port}`, ...(token ? { Authorization: `Bearer ${token}` } : {}), ...(payload ? { "Content-Type": "application/json", "Content-Length": payload.length } : {}) };
    const req = http.request({ host: connect || "127.0.0.1", port, method, path: route, headers, agent: false, timeout: CALL_S * 1000 }, (res) => {
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

function kill(app, tree = true) {
  if (!app.proc || app.proc.exitCode !== null) return;
  app.killed = true;
  spawnSync("taskkill", ["/PID", String(app.proc.pid), ...(tree ? ["/T"] : []), "/F"], { stdio: "ignore" });
}

async function stopped(app) {
  await waitFor(`${app.key} exits`, CALL_S, () => app.proc.exitCode !== null);
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
  const kept = path.join(roomDir(app, room), "recordings", set, owner, capture, series, name);
  const live = path.join(roomDir(app, room), owner, capture, series, name);
  return fs.existsSync(kept) || !fs.existsSync(live) ? kept : live;
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
  await sleep(within(SYNC_ROUND_S * 2) * 1000);
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

async function scenario(ctx) {
  let a, b, c;
  await step("start A and B", async () => {
    a = await launch("a");
    b = await launch("b");
  });

  await step("recording alone, the player follows the picture as it grows", async () => {
    ctx.room = await invoke(a, "create_room", { name: "Practice" });
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
    await evaluate(a, "openNow(); share(); true");
    await waitFor("A shares no recording", within(UI_REFRESH_S * 5), async () => !(await state(a)).party.shared.set);
  });

  await step("B joins A's room with approval", async () => {
    const code = await invoke(a, "create_invite", { reuse: false });
    await invoke(b, "join_room", { code });
    await waitFor("A sees B asking to join", within(DIAL_S * 2), async () => (await state(a)).asks.some((x) => x.id === b.id));
    await sleep(APPROVAL_WAIT_S * 1000);
    await invoke(a, "answer_ask", { id: b.id, approve: true });
    await allSee([a, b], "in the room and connected", within(DIAL_S * 2), (st, app) => st.room?.id === ctx.room && !st.room.pending && connected(st, [a, b].filter((x) => x !== app).map((x) => x.id)));
    const names = (await state(b)).names;
    check(names[a.id] === a.name, `B knows A as ${names[a.id]}`);
    const dials = await connects(b, a);
    check(dials === 1, `B connected to A ${dials} times while waiting for approval`);
  });

  await step("a second app on A's data folder exits and leaves A running", async () => {
    const env = { ...process.env, EZH_TEST: String(PORT_BASE + 3), EZH_DIR: a.data, WEBVIEW2_USER_DATA_FOLDER: path.join(OUT, "apps", "a2", "webview") };
    const second = spawn(EXE, [], { env, stdio: "ignore", windowsHide: true });
    const code = await withTimeout(new Promise((done) => second.on("exit", done)), CALL_S, "the second app");
    check(code === 0, `the second app exited with ${code}`);
    const starts = fs.readFileSync(path.join(a.data, "log.txt"), "utf8").split("\n").filter((l) => l.includes(" started, process ")).length;
    check(starts === 1, `A's log shows ${starts} starts`);
    const st = await state(a);
    check(connected(st, [b.id]), "A lost B when a second app started");
  });

  await step("Record on A starts the set on B", async () => {
    await allSee([a, b], "encoding its test source", within(COLD_START_S), (st) => st.capture.live);
    await invoke(a, "start_recording");
    ctx.set = (await state(a)).active_set.id;
    await allSee([b], "recording the same set", within(LOG_SYNC_S * 2), (st) => st.active_set?.id === ctx.set);
  });

  await step("C joins late through B's invite, approved on A", async () => {
    await waitFor(`A has logged ${SEGMENTS_BEFORE_LATE_JOIN} segments`, within(SEGMENT_S * (SEGMENTS_BEFORE_LATE_JOIN + 1)), () => segsOf(logOf(a, ctx.room, a.id), ctx.set, "v").length >= SEGMENTS_BEFORE_LATE_JOIN);
    c = await launch("c");
    const code = await invoke(b, "create_invite", { reuse: false });
    await invoke(c, "join_room", { code });
    await waitFor("A sees C asking B to join", within(DIAL_S * 2), async () => (await state(a)).asks.some((x) => x.id === c.id));
    await invoke(a, "answer_ask", { id: c.id, approve: true });
    const all = [a, b, c];
    await allSee(all, "everyone connected", within(DIAL_S * 2), (st, app) => st.room?.id === ctx.room && !st.room.pending && connected(st, all.filter((x) => x !== app).map((x) => x.id)));
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
    await allSee([b, c], "A's live picture available", within(SEGMENT_S + LOG_SYNC_S), (st) => st.peers.find((p) => p.id === a.id)?.live);
  });

  await step("connections stay up without redialing", async () => {
    const all = [a, b, c];
    const since = Date.now() * 1000;
    await sleep(STEADY_S * 1000);
    for (const app of all) {
      for (const peer of all.filter((x) => x !== app)) {
        const n = await connects(app, peer, since);
        check(n === 0, `${app.key} connected to ${peer.key} ${n} times in ${STEADY_S} s`);
      }
      check(connected(await state(app), all.filter((x) => x !== app).map((x) => x.id)), `${app.key} lost a peer`);
    }
  });

  await step("while recording the start box says Stop recording with the time and the list gets the running recording", async () => {
    check(await evaluate(a, `$("startbtn").textContent.includes(t("Stop recording")) && !!$("startbtn").querySelector("#livetimer") && !!$("startbtn").querySelector(".recdot") && $("standby").hidden && !!$("sessions").querySelector(".liverow .thumb.live")`), "the start box or the running recording's row is wrong");
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

  await step("B crashes mid-recording and recovers its open segment", async () => {
    const capture = segsOf(logOf(b, ctx.room, b.id), ctx.set, "v").at(-1).capture;
    const dir = path.join(roomDir(b, ctx.room), b.id, capture);
    const openPart = (series) => {
      const open = readJson(path.join(dir, series, "open.json"));
      return open && { series, n: open.n, set: open.set, whole: wholeFragments(path.join(dir, series, `${segName(open.n)}.part`)) };
    };
    await waitFor(`B's open segment has ${CRASH_AFTER_FRAGMENTS} whole fragments`, within(SEGMENT_S * 2), () => {
      const v = openPart("v");
      return v && v.set === ctx.set && v.whole >= CRASH_AFTER_FRAGMENTS;
    });
    kill(b, false);
    await stopped(b);
    const open = SERIES.map(openPart).filter((o) => o && o.set === ctx.set && o.whole > 0);
    check(open.some((o) => o.series === "v"), "no open picture segment at the crash");
    ctx.notes.push(`killed B with open segments ${brief(open)}`);
    start("b");
    await attach(b);
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
    await allSee(all, "everyone connected again", within(DIAL_S * 2), (st, app) => connected(st, all.filter((x) => x !== app).map((x) => x.id)));
    const logged = (x) => segsOf(logOf(x, ctx.room, x.id), ctx.set, "v").length;
    const target = logged(a) + SEGMENTS_AFTER_RESTART;
    await waitFor(`A logs ${SEGMENTS_AFTER_RESTART} more segments`, within(SEGMENT_S * (SEGMENTS_AFTER_RESTART + 1)), () => logged(a) >= target);
    const limits = { a: target, c: logged(c) };
    await waitFor("B holds them after its restart", within((SEGMENT_S + LOG_SYNC_S) * 2), () => !holdEverything([b], ctx.room, ctx.set, [a, c], limits));
    await waitFor("B records the set again in a new capture", within(SEGMENT_S * 2), () => {
      const own = logOf(b, ctx.room, b.id);
      const ended = own.findLastIndex((r) => r.k === "rec_end" && r.set === ctx.set);
      return own.findLastIndex((r) => r.k === "rec" && r.set === ctx.set) > ended && segsOf(own, ctx.set, "v").some((s) => s.capture !== capture);
    });
  });

  await step("A's encoder is killed and A keeps recording the set in a new part", async () => {
    const sink = (await invoke(a, "debug_view", { tab: "recording", since: 0, set: null, addresses: false })).engine.sink;
    check(sink && sink.pid, "A has no encoder");
    spawnSync("taskkill", ["/PID", String(sink.pid), "/F"], { stdio: "ignore" });
    const parts = () => segsOf(logOf(a, ctx.room, a.id), ctx.set, "v").filter((r) => r.capture !== sink.capture).length;
    await waitFor(`A logs ${SEGMENTS_AFTER_RESTART} segments of the set from a new encoder`, within(SEGMENT_S * (SEGMENTS_AFTER_RESTART + 2)), () => parts() >= SEGMENTS_AFTER_RESTART);
    const st = await state(a);
    check(st.active_set?.id === ctx.set && st.capture.working, `A is not recording the set again: ${brief(st.capture.status)}`);
  });

  await step("Stop on C ends the set everywhere and everyone has it", async () => {
    const all = [a, b, c];
    await invoke(c, "stop_recording");
    await allSee(all, "the set ended", within(LOG_SYNC_S * 2), (st) => !st.active_set && setOf(st, ctx.set) && !setOf(st, ctx.set).live);
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
    ctx.durationS = (setOf(st, ctx.set).end_us - setOf(st, ctx.set).start_us) / 1e6;
    ctx.notes.push(`verified ${count} pieces by SHA-256 on each of ${all.length} members, set of ${ctx.durationS.toFixed(1)} s`);
  });

  await step("each ended recording gets a thumbnail, and the room page lists recordings as cards", async () => {
    await waitFor("A makes the thumbnail of the set", within(30), async () => setOf(await state(a), ctx.set)?.thumb === true);
    const size = await evaluate(a, `new Promise((done) => { const i = new Image(); i.onload = () => done([i.naturalWidth, i.naturalHeight]); i.onerror = () => done(null); i.src = thumbSrc(setById(${JSON.stringify(ctx.set)})); })`);
    check(size && size[0] > 0 && size[1] > 0, `the page could not load the thumbnail: ${brief(size)}`);
    const file = fs.readFileSync(path.join(roomDir(a, ctx.room), "thumbs", `${ctx.set}.jpg`));
    check(file[0] === 0xff && file[1] === 0xd8 && file.length > 500, "the thumbnail file is not a JPEG");
    await evaluate(a, `roomTab = "recordings"; delete sigs.roomtab; go("room"); true`);
    await waitFor("A lists the set as a card with a picture", within(UI_REFRESH_S * 5), () => evaluate(a, `[...$("roomtab").querySelectorAll(".reccard")].some((c) => c.querySelector("img") && c.querySelectorAll(".acts button").length === 2)`));
    check(await evaluate(a, `!!$("sessions").querySelector(".srow:not(.liverow) .thumb img") && !!$("allrecs")`), "the sidebar row has no thumbnail");
    await evaluate(a, `$("recfilter").value = "zzzz"; $("recfilter").dispatchEvent(new Event("input")); true`);
    check(await evaluate(a, `$("roomtab").querySelectorAll(".reccard").length === 0`), "the filter did not hide the cards");
    await evaluate(a, `$("recfilter").value = ""; $("recfilter").dispatchEvent(new Event("input")); true`);
    await waitFor("the cards come back", within(UI_REFRESH_S * 3), () => evaluate(a, `$("roomtab").querySelectorAll(".reccard").length > 0`));
    await screenshot(a, "recordings-tab");
    await evaluate(a, `roomTab = "members"; delete sigs.roomtab; go("room"); true`);
    await screenshot(a, "room-members");
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
    await allSee(all, "everyone connected again", within(DIAL_S * 2), (st, app) => connected(st, all.filter((x) => x !== app).map((x) => x.id)));
    const id = await invoke(c, "add_event", { kind: "note", text: "After the restore", target: {}, values: null });
    await allSee([a, b], "C's note written after the restore", within(REQUEST_TIMEOUT_S + DIAL_S + LOG_SYNC_S), (st) => st.events.some((e) => e.key === `${c.id}/${id}`));
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
    await invoke(c, "set_process_plugin", { id: "transcribe", on: true });
    await invoke(b, "set_room_plugin", { id: "transcribe", on: false, settings: {} });
  });

  await step("the game plan is a tab of the room page and a rename reaches everyone", async () => {
    await evaluate(a, `showBoard(); true`);
    await waitFor("A shows the game plan inside the room page", within(UI_REFRESH_S * 3), () => evaluate(a, `!$("boardpane").hidden && !$("roompage").hidden`));
    check(await evaluate(a, `!$("navboard") && $("roomname").querySelector(".nm").textContent === t("Room")`), "the sidebar still has a game plan entry, or the room button shows a name");
    for (const [by, name] of [[b, "Drill room"], [a, "Practice"]]) {
      await invoke(by, "rename_room", { name });
      for (const app of [a, b, c]) await waitFor(`${app.key} shows ${name}`, within(LOG_SYNC_S * 2), async () => (await state(app)).room.name === name);
    }
    await evaluate(a, `roomTab = "settings"; delete sigs.roomtab; go("room"); true`);
    await waitFor("A shows the room settings", within(UI_REFRESH_S * 3), () => evaluate(a, `!!$("roomnameinput") && $("roomnameinput").value === "Practice"`));
    check(await evaluate(a, `[...document.querySelectorAll("#roompage .ptabs button")].map((x) => x.dataset.tab).join() === "members,plugins,jobs,board,recordings,settings" && !$("invitebtn") && !$("leavebtn")`), "the room page tabs or header are not as designed");
    await screenshot(a, "room-settings");
    await evaluate(a, `(() => { const i = $("roomnameinput"); i.value = "Scrim room"; i.dispatchEvent(new Event("input")); i.parentElement.querySelector("button.primary").click(); return true; })()`);
    for (const app of [a, b, c]) await waitFor(`${app.key} shows the name set in the room settings`, within(LOG_SYNC_S * 2), async () => (await state(app)).room.name === "Scrim room");
    await invoke(a, "rename_room", { name: "Practice" });
    for (const app of [a, b, c]) await waitFor(`${app.key} shows Practice again`, within(LOG_SYNC_S * 2), async () => (await state(app)).room.name === "Practice");
    await evaluate(a, `roomTab = "members"; go("room"); true`);
  });

  await step("each member row says one short state", async () => {
    const rows = await evaluate(a, `people().map((id) => memberOf(id).status.text)`);
    check(await evaluate(a, `(() => { const e = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 60, clientY: 60 }); document.body.dispatchEvent(e); return e.defaultPrevented && $("menu").hidden; })()`), "the browser's context menu is not blocked");
    await evaluate(a, `openSettings("general"); true`);
    check(await evaluate(a, `(() => { $("name").dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 300, clientY: 300 })); const labels = [...$("menu").querySelectorAll("button")].map((b) => b.textContent).join(); closeMenu(); $("settings").close(); return labels === [t("Cut"), t("Copy"), t("Paste"), t("Select all")].join(); })()`), "a text field has no edit menu");
    check(await evaluate(a, `!$("sessions").querySelector(".liverow") && !$("standby").hidden && $("standby").textContent.includes(t("Standby"))`), "the idle list holds a live entry or the start box lacks Standby");
    check(rows.length === 3 && rows.every((x) => x === "Standby" || /^Receiving \d+%$/.test(x)), `member states ${brief(rows)}`);
  });

  await step("Watch Party converges on a seek and a play", async () => {
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
    await evaluate(a, `openSet(${JSON.stringify(ctx.set)}); act("seek", ${SEEK_TO_S}); true`);
    await follow(a, [b, c], false, SEEK_TO_S);
    await evaluate(c, `act("play", nowM()); true`);
    await follow(c, [a, b], true, "nowM()");
    const clocks = await Promise.all(all.map((app) => evaluate(app, "nowM()")));
    check(Math.max(...clocks) - Math.min(...clocks) < JUMP_MIN_S, `clocks differ: ${brief(clocks)}`);
    check(Math.min(...clocks) >= SEEK_TO_S, `played from before the seek: ${brief(clocks)}`);
    await evaluate(a, `act("pause", nowM()); true`);
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

  await step("the local API is off until the page switches it on, then reads recordings and takes a note and a game plan that everyone sees", async () => {
    const refused = await api(a, "GET", "/llms.txt").then(() => false, () => true);
    check(refused && (await state(a)).api.on === false, "the API answers before it was switched on");
    await evaluate(a, `openSettings("api"); true`);
    await evaluate(a, `$("apion").click(); true`);
    await waitFor("A switches the API on", within(UI_REFRESH_S * 5), () => evaluate(a, `st.api.on && !$("apirows").hidden && $("apiaddr").textContent === "http://127.0.0.1:${API_PORTS.a}"`));
    const { token } = await invoke(a, "api_info");
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
    check(diag.status === 200 && diagText.includes("=== this PC ===\nez-hanseikai") && diagText.includes("recent log") && diagText.split("=== member ").length === 3, `diagnostics: ${diag.status} ${brief(diagText.slice(0, 200))}`);
    for (const secret of [NAMES.a, NAMES.b, NAMES.c, a.id, b.id]) check(!diagText.includes(secret), `the API report holds ${secret.slice(0, 4)}`);
    const opened = await api(a, "POST", `${room}/open`, { token });
    check(opened.status === 200 && opened.json.room === ctx.room, `open: ${opened.status} ${brief(opened.text)}`);
    check((await api(a, "POST", "/v1/rooms/nope/open", { token })).status === 404 && (await api(a, "GET", `${room}/open`, { token })).status === 405, "open takes any room or method");
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
    await waitFor("A lists the endpoints", within(UI_REFRESH_S * 5), () => evaluate(a, `$("apiref").querySelectorAll("details").length > 15 && $("apiref").textContent.includes("/v1/status") && $("apiref").querySelector("pre") !== null`));
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
    const fresh = await waitFor("A has a new token", within(UI_REFRESH_S * 5), async () => {
      const next = (await invoke(a, "api_info")).token;
      return next !== token && next;
    });
    check((await api(a, "GET", "/v1", { token })).status === 401 && (await api(a, "GET", "/v1", { token: fresh })).status === 200, "the old token still works or the new one does not");
    check(readJson(path.join(a.data, "api.json"))?.token === fresh, "api.json keeps the old token");
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
    await evaluate(a, `openSettings("api"); document.querySelector('#apiaccess [data-lan="0"]').click(); true`);
    await waitFor("A limits the API to this PC", within(UI_REFRESH_S * 5), () => evaluate(a, `!st.api.lan && $("apilanbox").hidden`));
    if (own) check(await api(a, "GET", "/v1", { token: fresh, host: own, connect: "127.0.0.1" }).then((r) => r.status === 403), "this PC's own address is accepted again for this PC only");
    await evaluate(a, `$("apion").click(); true`);
    await waitFor("A switches the API off", within(UI_REFRESH_S * 5), () => evaluate(a, `!st.api.on && $("apirows").hidden`));
    check(await api(a, "GET", "/llms.txt").then(() => false, () => true), "the API still answers after it was switched off");
    check(readJson(path.join(a.data, "settings.json")).api === false, "settings.json does not keep the switch");
    check(!fs.existsSync(path.join(a.data, "api.json")), "api.json stays after the API stopped");
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
    check(report.includes("ez-hanseikai") && report.includes("jobs") && report.includes("recent log") && report.includes("\nhardware\n") && report.includes("\nclocks\n") && report.includes("\nwhere data goes\n"), "the report lacks its parts");
    for (const secret of [NAMES.a, NAMES.b, NAMES.c, a.id, b.id]) check(!report.includes(secret), `the report holds ${secret.slice(0, 4)}`);
    check(!/[A-Za-z]:\\/.test(report) && !/\b\d{1,3}(\.\d{1,3}){3}\b/.test(report), "the report holds a path or an address");
    await evaluate(a, `$("diagdlg").close(); true`);
  });

  await step("export the set on A to a playable file", async () => {
    await evaluate(a, `window.e2eExport = null; window.__TAURI__.event.listen("export", (e) => { if (e.payload.done) window.e2eExport = e.payload; }).then(() => true)`);
    const started = await invoke(a, "export_set", { room: ctx.room, set: ctx.set });
    check(started && started.skipped.length === 0, `export skipped ${brief(started)}`);
    const done = await waitFor("the export finished", within(ctx.durationS / EXPORT_SPEED), () => evaluate(a, "window.e2eExport"));
    check(!done.error, `export failed: ${done.error}`);
    const file = path.join(a.data, `${ctx.set}.mp4`);
    const probe = ffmpeg("ffprobe", ["-v", "error", "-show_entries", "format=duration:stream=codec_type", "-of", "json", file]);
    check(probe.ok, `ffprobe failed: ${probe.err}`);
    const info = JSON.parse(probe.out);
    const kinds = info.streams.map((s) => s.codec_type);
    check(kinds.filter((k) => k === "video").length === 1 && kinds.filter((k) => k === "audio").length === 4, `streams ${brief(kinds)}`);
    const duration = Number(info.format.duration);
    check(Math.abs(duration - ctx.durationS) <= SEGMENT_S, `export lasts ${duration} s, the set ${ctx.durationS} s`);
    const decode = ffmpeg("ffmpeg", ["-v", "error", "-i", file, "-map", "0", "-f", "null", "-"]);
    check(decode.ok && !decode.err, `export does not decode cleanly: ${decode.err.slice(0, 400)}`);
    ctx.notes.push(`export of ${duration.toFixed(1)} s with streams ${kinds.join(" ")}`);
    ffmpeg("ffmpeg", ["-v", "error", "-y", "-ss", String(duration / 2), "-i", file, "-frames:v", "1", path.join(OUT, "export-frame.png")]);
  });

  await step("the set exports to a folder of plain files and imports back as a recording everyone receives", async () => {
    const all = [a, b, c];
    const names = all.map((x) => x.name).sort().join();
    const raw = await invokeLong(a, "export_folder", { room: ctx.room, set: ctx.set, aligned: false }, EXPORT_FOLDER_S, "the folder export");
    check(raw.files === 3 && raw.skipped.length === 0, `folder export: ${brief(raw)}`);
    const session = readJson(path.join(raw.folder, "session.json"));
    check(session?.id === ctx.set && session.start && session.views?.length === 3 && session.views.map((v) => v.name).sort().join() === names, `session.json: ${brief(session)}`);
    const offsets = session.views.map((v) => v.offset_ms);
    check(Math.min(...offsets) === 0 && Math.max(...offsets) >= SEGMENT_S * 1000, `raw offsets ${brief(offsets)}`);
    const plain = (file, kinds) => {
      const probe = ffmpeg("ffprobe", ["-v", "error", "-show_entries", "format=duration:stream=codec_type", "-of", "json", file]);
      check(probe.ok, `ffprobe ${path.basename(file)}: ${probe.err}`);
      const info = JSON.parse(probe.out);
      const got = info.streams.map((x) => x.codec_type).sort().join();
      check(got === kinds, `${path.basename(file)} has streams ${got}, not ${kinds}`);
      const decode = ffmpeg("ffmpeg", ["-v", "error", "-i", file, "-f", "null", "-"]);
      check(decode.ok && !decode.err, `${path.basename(file)} does not decode cleanly: ${decode.err.slice(0, 300)}`);
      return Number(info.format.duration);
    };
    for (const v of session.views) {
      check(v.file.endsWith(".mp4") && v.voice?.endsWith(".m4a"), `view ${brief(v)}`);
      plain(path.join(raw.folder, v.file), "audio,video");
      plain(path.join(raw.folder, v.voice), "audio");
    }
    check(fs.readFileSync(path.join(raw.folder, "notes.csv"), "utf8").includes("Noted through the API"), "notes.csv lacks the note written through the API");
    const aligned = await invokeLong(a, "export_folder", { room: ctx.room, set: ctx.set, aligned: true }, EXPORT_FOLDER_S, "the aligned folder export");
    const second = readJson(path.join(aligned.folder, "session.json"));
    check(second.views.every((v) => v.offset_ms === 0), `aligned offsets ${brief(second.views.map((v) => v.offset_ms))}`);
    for (const v of second.views) {
      const seconds = plain(path.join(aligned.folder, v.file), "audio,video");
      check(Math.abs(seconds - ctx.durationS) <= SEGMENT_S, `${v.file} lasts ${seconds} s in the aligned export, the set ${ctx.durationS} s`);
    }

    const made = await invokeLong(a, "import_recording", { spec: { folder: raw.folder } }, IMPORT_S, "the import of the exported folder");
    ctx.imported = made.id;
    check(made.views === 3 && made.id !== ctx.set, `import: ${brief(made)}`);
    await allSee(all, "the imported recording", within(LOG_SYNC_S * 3), (st) => setOf(st, made.id)?.imported_by === a.name);
    const owners = setOf(await state(a), made.id).players.map((p) => p.owner);
    for (const app of all) {
      const st = await state(app);
      const set = setOf(st, made.id);
      check(set.title === path.basename(raw.folder) && set.players.length === 3 && set.players.every((p) => p.imported), `${app.key} lists the import as ${brief({ title: set.title, players: set.players.map((p) => p.imported) })}`);
      check(owners.every((o) => !all.some((x) => x.id === o) && names.split(",").includes(st.names[o])), `${app.key} names the imported views ${brief(owners.map((o) => st.names[o]))}`);
      check(st.rooms.find((r) => r.id === ctx.room).members.length === 3 && st.peers.length === 2, `${app.key} counts ${st.peers.length} peers and ${st.rooms.find((r) => r.id === ctx.room).members.length} members`);
    }
    await onlyOnDownload([b, c], a, made.id);
    await allSee(all, "everyone has the imported recording", within((SEGMENT_S + SYNC_ROUND_S) * 3), (st) => heldByAll(st, made.id, all));
    for (const owner of owners) {
      const list = pieces(logOf(a, ctx.room, owner), made.id);
      check(list.length > 2, `the imported view has ${list.length} pieces`);
      for (const holder of [a, b, c]) check(!badCopies(holder, ctx.room, owner, list).length, `${holder.key} lacks pieces of an imported view`);
    }
    await allSee([b, c], "the note of the imported folder", within(LOG_SYNC_S * 2), (st) => st.events.some((e) => e.set === made.id && e.text.includes("Noted through the API")));
    await evaluate(a, `openSession(${JSON.stringify(made.id)}); true`);
    await waitFor("A plays the three imported views", within(UI_REFRESH_S * 10), () => evaluate(a, `play.keys.length === 3 && play.keys.filter((k) => play.tiles[k].media.video.videoWidth > 0).length === 3`));
    check(await evaluate(a, `$("pchips").textContent.includes("Imported by")`), "the player header does not say the recording is imported");
    check(await evaluate(a, `play.keys.every((k) => !play.tiles[k].imp.hidden)`), "a tile of an imported view has no Imported mark");
    await screenshot(a, "imported-player");
    ctx.notes.push(`folder export of ${session.views.length} views imported back and received by SHA-256 on 3 members`);
  });

  await step("a folder made by hand, with a transcoded file and a sound-only file, imports on B and plays for everyone", async () => {
    const all = [a, b, c];
    const media = path.join(OUT, "media", "Walkthrough");
    fs.mkdirSync(media, { recursive: true });
    const make = (inputs, out) => {
      const r = ffmpeg("ffmpeg", ["-v", "error", "-y", ...inputs.flatMap((i) => ["-f", "lavfi", "-i", i]), ...out]);
      check(r.ok, `ffmpeg ${out.at(-1)}: ${r.err}`);
    };
    make([`testsrc2=size=320x180:rate=15:duration=${MEDIA_SECONDS.coach}`, `sine=frequency=440:duration=${MEDIA_SECONDS.coach}`], ["-c:v", "libx264", "-pix_fmt", "yuv420p", "-g", "15", "-c:a", "aac", "-shortest", path.join(media, "Coach.mp4")]);
    make([`testsrc2=size=160x120:rate=10:duration=${MEDIA_SECONDS.camera}`, `sine=frequency=660:duration=${MEDIA_SECONDS.camera}`], ["-c:v", "mpeg4", "-c:a", "pcm_s16le", "-shortest", path.join(media, "Old camera.avi")]);
    make([`sine=frequency=880:duration=${MEDIA_SECONDS.commentary}`], ["-c:a", "aac", path.join(media, "Commentary.m4a")]);
    fs.writeFileSync(path.join(media, "session.json"), JSON.stringify({
      title: "Walkthrough", start: PLAN_START,
      views: [{ name: "Coach", file: "Coach.mp4" }, { name: "Old camera", file: "Old camera.avi", offset_ms: 2000 }, { name: "Commentary", file: "Commentary.m4a" }],
      events: [{ at_s: 3, text: "The ring closes here #callout" }, { at_s: 0, to_s: 4, text: "Intro" }],
    }));
    const plan = await invoke(b, "import_plan", { spec: { folder: media } });
    check(plan.title === "Walkthrough" && plan.views.length === 3 && plan.notes === 2 && plan.start === "2026-10-19T12:30:00.000Z", `plan ${brief(plan)}`);
    const broken = path.join(OUT, "media", "Broken");
    fs.mkdirSync(broken, { recursive: true });
    fs.writeFileSync(path.join(broken, "session.json"), "{oops");
    const refused = await invoke(b, "import_plan", { spec: { folder: broken } }).then(() => "", (e) => String(e));
    check(refused.includes("is not valid"), `a broken session.json was answered with "${refused}"`);
    const before = (await state(b)).sets.length;
    check((await invoke(b, "import_recording", { spec: { folder: broken } }).then(() => "", (e) => String(e))).includes("is not valid") && (await state(b)).sets.length === before, "a refused import changed the recordings");
    await evaluate(b, `openImport({ folder: ${JSON.stringify(media)} }); true`);
    await waitFor("B's import dialog lists the three views", within(UI_REFRESH_S * 5), () => evaluate(b, `$("importdlg").open && $("importviews").children.length === 3 && !$("importgo").disabled`));
    check(await evaluate(b, `$("importtitle").value === "Walkthrough" && [...$("importviews").querySelectorAll("input")].map((i) => i.value).join() === "Coach,Old camera,Commentary"`), "the import dialog does not show the title and the view names");
    await screenshot(b, "import-dialog");
    await evaluate(b, `$("importdlg").close(); true`);
    const made = await invokeLong(b, "import_recording", { spec: { folder: media } }, IMPORT_S, "the import of the hand-made folder");
    check(made.views === 3 && made.notes === 2, `import: ${brief(made)}`);
    await allSee(all, "the walkthrough", within(LOG_SYNC_S * 3), (st) => setOf(st, made.id)?.imported_by === b.name && setOf(st, made.id).title === "Walkthrough");
    await onlyOnDownload([a, c], b, made.id);
    await allSee(all, "everyone has the walkthrough", within((SEGMENT_S + SYNC_ROUND_S) * 3), (st) => heldByAll(st, made.id, all));
    await waitFor("B makes the walkthrough's thumbnail", within(30), async () => setOf(await state(b), made.id)?.thumb === true);
    const seen = await state(a);
    const set = setOf(seen, made.id);
    const view = (name) => set.players.find((p) => seen.names[p.owner] === name);
    const [coach, camera, commentary] = ["Coach", "Old camera", "Commentary"].map(view);
    check(coach && camera && commentary, `views ${brief(set.players.map((p) => seen.names[p.owner]))}`);
    check(Math.abs(set.start_us - Date.parse(PLAN_START) * 1000) <= START_TOLERANCE_US, `start ${set.start_us}`);
    check(Math.abs(Math.abs(camera.start_us - coach.start_us) - 2e6) <= 1, `the second view starts ${camera.start_us - coach.start_us} us after the first`);
    check(coach.parts.every((part) => part.v && !part.m) && camera.parts.every((part) => part.v) && commentary.parts.every((part) => !part.v && part.m), "the views have the wrong series");
    const codecs = [coach, camera].map((p) => logOf(b, ctx.room, p.owner).find((r) => r.k === "cap")?.codec);
    check(codecs.join() === "h264,h264", `the stored codecs are ${codecs}`);
    await waitFor("A shows the notes and the chapter", within(LOG_SYNC_S * 2), async () => {
      const st = await state(a);
      const callout = st.events.find((e) => e.set === made.id && e.text === "The ring closes here #callout");
      const intro = st.events.find((e) => e.set === made.id && e.text === "Intro");
      return callout?.tags.includes("callout") && intro?.end_us > intro?.local_us;
    });
    await evaluate(c, `openSession(${JSON.stringify(made.id)}); true`);
    await waitFor("C plays the two pictures", within(UI_REFRESH_S * 10), () => evaluate(c, `play.keys.length === 3 && play.keys.filter((k) => play.tiles[k].media.video.videoWidth > 0).length === 2`));
    await waitFor("the walkthrough has one finished Apex job", within(JOB_CHECK_S + LOG_SYNC_S + 60), async () => {
      const jobs = (await invoke(a, "jobs")).jobs.filter((j) => j.plugin === PLUGIN && j.set === made.id);
      check(!jobs.some((j) => j.state === "failed"), `the Apex job failed: ${brief(jobs)}`);
      return jobs.length === 1 && jobs[0].state === "done";
    });
    await evaluate(c, `openSession(${JSON.stringify(ctx.set)}); true`);
    ctx.notes.push("hand-made folder with a copied, a transcoded and a sound-only file imported on B and played on C");
  });

  await step("the local API imports a bare folder of videos and exports a recording to a folder", async () => {
    const bare = path.join(OUT, "media", "Review night");
    fs.mkdirSync(bare, { recursive: true });
    for (const [name, hz] of [["Ann POV", 440], ["Ben POV", 660]]) {
      const r = ffmpeg("ffmpeg", ["-v", "error", "-y", "-f", "lavfi", "-i", "testsrc2=size=160x90:rate=10:duration=4", "-f", "lavfi", "-i", `sine=frequency=${hz}:duration=4`, "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac", "-shortest", path.join(bare, `${name}.mp4`)]);
      check(r.ok, `ffmpeg ${name}: ${r.err}`);
    }
    await invoke(a, "set_api", { on: true });
    await waitFor("the API answers", within(UI_REFRESH_S * 5), () => api(a, "GET", "/llms.txt").then((r) => r.status === 200, () => false));
    const llms = await api(a, "GET", "/llms.txt");
    check(llms.text.includes("recordings/import") && llms.text.includes("/export"), "llms.txt does not describe import and export");
    const { token } = await invoke(a, "api_info");
    const room = `/v1/rooms/${ctx.room}`;
    check((await api(a, "POST", `${room}/recordings/import`, { token, body: { folder: path.join(OUT, "media", "Nothing here") } })).status === 400, "a missing folder was not refused");
    check((await api(a, "POST", `${room}/recordings/import`, { token, body: { files: "x" } })).status === 400, "a bad body was not refused");
    const made = await api(a, "POST", `${room}/recordings/import`, { token, body: { folder: bare } });
    check(made.status === 201 && made.json.views === 2, `import: ${made.status} ${brief(made.text)}`);
    ctx.bare = made.json.id;
    const one = await api(a, "GET", `${room}/recordings/${encodeURIComponent(made.json.id)}`, { token });
    check(one.status === 200 && one.json.title === "Review night" && one.json.imported_by?.id === a.id && one.json.members.length === 2 && one.json.members.every((m) => m.imported) && one.json.members.map((m) => m.name).sort().join() === "Ann POV,Ben POV", `recording: ${brief(one.json)}`);
    await allSee([b, c], "the bare folder's recording", within(LOG_SYNC_S * 3), (st) => setOf(st, made.json.id)?.imported_by === a.name);
    const out = path.join(OUT, "api-export");
    check((await api(a, "POST", `${room}/recordings/${encodeURIComponent(made.json.id)}/export`, { token, body: {} })).status === 400, "an export without a folder was accepted");
    const exported = await api(a, "POST", `${room}/recordings/${encodeURIComponent(made.json.id)}/export`, { token, body: { folder: out, aligned: true } });
    check(exported.status === 201 && exported.json.files === 2, `export: ${exported.status} ${brief(exported.text)}`);
    const session = readJson(path.join(out, "session.json"));
    check(session.views.map((v) => v.file).sort().join() === "Ann POV.mp4,Ben POV.mp4" && session.views.every((v) => v.offset_ms === 0), `exported session.json ${brief(session)}`);
    for (const v of session.views) {
      const probe = ffmpeg("ffprobe", ["-v", "error", "-show_entries", "format=duration", "-of", "default=nw=1:nk=1", path.join(out, v.file)]);
      check(probe.ok && Math.abs(Number(probe.out) - 4) <= 1.5, `${v.file} lasts ${probe.out.trim()} s`);
    }
    check((await api(a, "POST", `${room}/recordings/${encodeURIComponent(made.json.id)}/export`, { token, body: { folder: out } })).status === 400, "an export into a folder that is not empty was accepted");
    await invoke(a, "set_api", { on: false });
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
    const bare = ctx.bare;
    await allSee(all, "the bare recording listed", within(LOG_SYNC_S * 3), (st) => !!setOf(st, bare));
    const rowsOf = async (app) => (await invoke(app, "jobs")).jobs.filter((j) => j.set === bare).length;
    await waitFor("A and B list a job row for the bare recording", within(JOB_CHECK_S + LOG_SYNC_S * 3), async () => (await rowsOf(a)) > 0 && (await rowsOf(b)) > 0);
    check((await state(a)).sets.find((x) => x.id === bare).shared === false, "A thinks others hold the bare recording");
    check(await evaluate(b, `!canDelete(setById(${JSON.stringify(bare)}))`), "B may delete a recording it does not hold while A does");
    await waitFor("A made the thumbnail", within(30), async () => setOf(await state(a), bare)?.thumb === true);
    await confirmDeletion(a, bare, "This deletes it completely");
    await allSee(all, "the bare recording gone from the list", within(LOG_SYNC_S * 3), (st) => !setOf(st, bare));
    for (const app of all) await waitFor(`${app.key} has no job row for it`, within(JOB_CHECK_S + LOG_SYNC_S), async () => (await rowsOf(app)) === 0);
    check(logOf(a, ctx.room, a.id).some((r) => r.k === "set_del" && r.set === bare), "A did not log the removal");
    check(mediaLeft(a, bare) === 0 && !fs.existsSync(path.join(roomDir(a, ctx.room), "recordings", bare)) && !fs.existsSync(path.join(roomDir(a, ctx.room), "thumbs", `${bare}.jpg`)), "A kept media or the thumbnail");
    await waitFor("B's sidebar drops the bare recording", within(UI_REFRESH_S * 5), () => evaluate(b, `![...document.querySelectorAll("#sessions .srow")].some((r) => r.textContent.includes(sessionName(${JSON.stringify(bare)})))`));
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

  await step("the update channel is kept in the settings", async () => {
    check((await state(b)).channel === "stable", "the channel does not start as stable");
    await invoke(b, "set_channel", { channel: "beta" });
    check(readJson(path.join(b.data, "settings.json")).channel === "beta", "settings.json does not keep the beta channel");
    check(await invoke(b, "check_update").then(() => true, (e) => String(e).length > 0), "checking the beta channel failed");
    await invoke(b, "set_channel", { channel: "stable" });
  });

  await step("C kept every file of an older version it found in its data folder", async () => {
    for (const [name, body] of Object.entries(oldFiles())) {
      const file = path.join(apps.c.data, name);
      check(fs.existsSync(file) && fs.readFileSync(file).equals(Buffer.from(body)), `${name} was changed or removed`);
    }
  });

  await step("removing C for the whole room stops it fetching", async () => {
    await invoke(a, "remove_peer", { id: c.id });
    await allSee([a, b], "C no longer a peer", within(LOG_SYNC_S * 2), (st) => !st.peers.some((p) => p.id === c.id));
    await sleep(within(LOG_SYNC_S) * 1000);
    const before = fileSize(logFile(c, ctx.room, a.id));
    const id = await invoke(a, "add_event", { kind: "note", text: "After the removal", target: {}, values: null });
    const key = `${a.id}/${id}`;
    await allSee([b], "the new note", within(LOG_SYNC_S * 2), (st) => st.events.some((e) => e.key === key));
    await sleep(within(DIAL_S + LOG_SYNC_S) * 1000);
    check(fileSize(logFile(c, ctx.room, a.id)) === before, "C still received A's log after its removal");
    const st = await state(c);
    check(!st.events.some((e) => e.key === key), "C shows a note written after its removal");
  });

  await step("B leaves the room, keeps it and enters it again", async () => {
    await invoke(b, "leave_room", { id: ctx.room });
    const left = await state(b);
    check(!left.room && left.rooms.some((r) => r.id === ctx.room), `B's rooms after leaving: ${brief(left.rooms)}`);
    check(fs.existsSync(logFile(b, ctx.room, b.id)), "B lost its log by leaving");
    await invoke(b, "switch_room", { id: ctx.room });
    await allSee([a, b], "A and B connected again", within(DIAL_S * 2), (st, app) => st.room?.id === ctx.room && connected(st, [a, b].filter((x) => x !== app).map((x) => x.id)));
  });
}

async function main() {
  fs.rmSync(OUT, { recursive: true, force: true });
  fs.mkdirSync(OUT, { recursive: true });
  check(fs.existsSync(EXE), `no app at ${EXE}`);
  check(fs.existsSync(path.join(FFMPEG_DIR, "ffprobe.exe")), `no ffprobe in ${FFMPEG_DIR}`);
  const ctx = { notes: [] };
  const started = Date.now();
  let failed = null;
  try {
    await scenario(ctx);
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

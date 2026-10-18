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

const DIAL_S = DIAL_RETRY_S + CONNECT_TIMEOUT_S;
const SYNC_ROUND_S = LOG_SYNC_S * HELLO_EVERY;
const FRAGMENT_S = SEGMENT_S / FRAGMENTS_PER_SEGMENT;
const CRASH_AFTER_FRAGMENTS = FRAGMENTS_PER_SEGMENT / 2;
const PORT_BASE = Number(process.env.E2E_PORT_BASE || 9311);
const PORTS = { a: PORT_BASE, b: PORT_BASE + 1, c: PORT_BASE + 2 };
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

const invoke = (app, cmd, args = {}) => evaluate(app, `window.__TAURI__.core.invoke(${JSON.stringify(cmd)}, ${JSON.stringify(args)})`);
const state = (app) => invoke(app, "state");

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
    return list.find((t) => t.type === "page" && t.url.startsWith("http") && !t.url.includes("debug.html") && t.webSocketDebuggerUrl);
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
  await invoke(app, "set_quality", { id: "light" });
  await share(app);
  say(`${key} is ${app.id.slice(0, 10)}`);
  return app;
}

async function connects(app, peer, since = 0) {
  const view = await invoke(app, "debug_view", { tab: "connections", since, set: null, addresses: false });
  return view.events.filter((e) => e.text.startsWith(peer.id.slice(0, 10)) && e.text.includes(": connected")).length;
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

function pieces(owned, set, limit = Infinity) {
  const caps = Object.fromEntries(owned.filter((r) => r.k === "cap").map((r) => [r.capture, r]));
  const segs = SERIES.flatMap((series) => segsOf(owned, set, series).slice(0, limit));
  const runs = [...new Set(segs.map((s) => `${s.capture}/${s.series}`))].map((k) => k.split("/"));
  const inits = runs.map(([capture, series]) => ({ capture, series, name: "init.mp4", ...(caps[capture]?.init?.[series] || {}) }));
  return [...inits, ...segs.map((s) => ({ capture: s.capture, series: s.series, name: segName(s.n), size: s.size, sha256: s.sha256 }))];
}

function badCopies(holder, room, owner, list) {
  return list
    .filter((p) => {
      const file = path.join(roomDir(holder, room), owner, p.capture, p.series, p.name);
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
  const dir = path.join(roomDir(app, room), app.id, last.capture, "v");
  const joined = path.join(OUT, `${name}.mp4`);
  fs.writeFileSync(joined, Buffer.concat([fs.readFileSync(path.join(dir, "init.mp4")), fs.readFileSync(path.join(dir, segName(last.n)))]));
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

  await step("create a room on A; B joins with approval", async () => {
    ctx.room = await invoke(a, "create_room", { name: "Practice" });
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
    check(recovery === "The app restarted after a problem.", `B says ${brief(recovery)} after the crash`);
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
    await allSee(all, "Everyone has it", within((SEGMENT_S + SYNC_ROUND_S) * 2), (st) => setOf(st, ctx.set)?.all_verified && setOf(st, ctx.set).status === "Everyone has it");
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

  await step("the transcription download follows the chosen model", async () => {
    await evaluate(b, `roomTab = "plugins"; go("room"); pollPlugins(); true`);
    for (const [model, size] of Object.entries(MODEL_BYTES)) {
      const shown = `[...$("roomtab").querySelectorAll(".plug")].find((p) => p.querySelector("b").textContent === t("Transcription"))`;
      await waitFor("B shows the transcription card", within(UI_REFRESH_S * 5), () => evaluate(b, `!!${shown}`));
      await evaluate(b, `(() => { const s = ${shown}.querySelector(".field select"); s.value = ${JSON.stringify(model)}; s.dispatchEvent(new Event("change")); return true; })()`);
      const text = bytes(size + WHISPER_BYTES);
      await waitFor(`B's card offers ${text}`, within(LOG_SYNC_S * 2), () => evaluate(b, `${shown}.querySelector(".here").textContent.includes(${JSON.stringify(text)})`));
      for (const app of [b, a]) {
        await waitFor(`${app.key} offers the ${model} model`, within(LOG_SYNC_S * 2), async () => {
          const p = (await invoke(app, "room_plugins")).find((x) => x.id === "transcribe");
          return p.settings.model === model && p.download_bytes === size + WHISPER_BYTES;
        });
      }
    }
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

  await step("a session started while watching together records voices and the Watch Party", async () => {
    const all = [a, b, c];
    await allSee(all, "watching the set together", within(PARTY_REPEAT_S + UI_REFRESH_S * 2), (st) => st.party.on && st.party.shared.set === ctx.set);
    check(await evaluate(a, `startState() === "review"`), "A's start button does not offer to record the review");
    await invoke(a, "start_recording");
    ctx.review = (await state(a)).active_set.id;
    await allSee(all, "recording the review voice only", within(LOG_SYNC_S * 2), (st) => st.active_set?.id === ctx.review && st.active_set.voice === true);
    await waitFor(`A logs ${SEGMENTS_AFTER_RESTART} voice segments of the review`, within(SEGMENT_S * (SEGMENTS_AFTER_RESTART + 2)), () => segsOf(logOf(a, ctx.room, a.id), ctx.review, "m").length >= SEGMENTS_AFTER_RESTART);
    await evaluate(c, `act("seek", ${SEEK_TO_S * 2}); true`);
    await waitFor("C logs its seek as a party record", within(LOG_SYNC_S * 2), () => logOf(c, ctx.room, c.id).some((r) => r.k === "party" && r.set === ctx.review && r.shared.set === ctx.set));
    await invoke(a, "stop_recording");
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
    await evaluate(a, `act("seek", reviewTotal()); true`);
    await waitFor("A replays the watched session at the seek", within(UI_REFRESH_S * 3), () => evaluate(a, `review?.id === ${JSON.stringify(ctx.review)} && selected === ${JSON.stringify(ctx.set)} && Math.abs(nowM() - ${SEEK_TO_S * 2}) < ${JUMP_MIN_S}`));
    await evaluate(a, `endReview(); openSession(${JSON.stringify(ctx.set)}); true`);
    ctx.notes.push(`review session with ${(await state(a)).sets.find((s) => s.id === ctx.review).party.length} party records replayed`);
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

  await step("a deleted session stays listed and comes back with Download", async () => {
    const preview = await evaluate(b, `JSON.stringify(deletePreview([setById(${JSON.stringify(ctx.review)})], st.id))`);
    const parts = JSON.parse(preview)[0];
    check(parts.players.length === 3 && parts.players.every((p) => p.bytes > 0), `the delete preview lists ${brief(parts)}`);
    await invoke(b, "delete_set", { room: ctx.room, set: ctx.review });
    await waitFor("B lists the review as not on this PC", within(UI_REFRESH_S * 3), async () => { const st = await state(b); return !setOf(st, ctx.review) && st.gone.some((g) => g.id === ctx.review); });
    const gone = logOf(a, ctx.room, b.id).filter((r) => r.k === "seg" && r.set === ctx.review);
    check(gone.every((r) => !fs.existsSync(path.join(roomDir(b, ctx.room), b.id, r.capture, r.series, segName(r.n)))), "B kept its own review segments after deleting them");
    await invoke(b, "restore_set", { room: ctx.room, set: ctx.review });
    await waitFor("B fetches the review again", within((SEGMENT_S + SYNC_ROUND_S) * 3), async () => setOf(await state(b), ctx.review)?.all_verified);
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

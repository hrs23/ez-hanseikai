// Stability numbers per app version, read from the app's own logs.
// node scripts/stability.mjs [--json] <data folder | log file | log .gz> ...
// A data folder is read as log.older.txt, log.old.txt, log.txt and crash.txt, oldest first.
// Every rate is per hour the app ran (or per recording hour), so versions used for different times compare.
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const LINE = /^(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3})\s+(TRACE|DEBUG|INFO|WARN|ERROR)\s+(?:\S+?:\s)?(.*)$/;
const STARTED = /^.*? (\d+\.\d+\.\d+\S*) started, process/;
// A run that sees no line for this long is counted as stopped there (the PC slept or the app was closed).
const GAP_MS = 30 * 60 * 1000;

const COUNTS = {
  // the app
  unclean: /the app did not exit cleanly last time/,
  panic: /\bpanic(ked)?\b/,
  // capture and recording
  captureBehind: /capture is falling behind after/,
  captureDropped: /capture rate fell from/,
  duplicated: /More than 1000 frames duplicated/,
  encoderStalled: /produced nothing for \d+ s, stopping it/,
  // sync between members
  connected: /: connected \(\+/,
  connectFailed: / connect failed: /,
  failedInRow: /connect attempts failed in a row, then reachable/,
  syncStopped: /: sync stopped: /,
  helloLost: /: hello not answered/,
  protocolDiffers: /: protocol differs from ours/,
  fetchStopped: /: fetch from \S+ stopped: /,
  liveBehind: /live picture fell behind, restarting/,
  qualityLowered: /quality lowered to /,
  timeCheckFailed: /time check failed/,
};

function read(file) {
  const data = fs.readFileSync(file);
  return (file.endsWith(".gz") || (data[0] === 0x1f && data[1] === 0x8b) ? zlib.gunzipSync(data) : data).toString("utf8");
}

function inputs(args) {
  const files = [];
  for (const arg of args) {
    if (fs.statSync(arg).isDirectory()) {
      for (const name of ["log.older.txt", "log.old.txt", "log.txt", "log.gz"]) if (fs.existsSync(path.join(arg, name))) files.push(path.join(arg, name));
      if (fs.existsSync(path.join(arg, "crash.txt"))) files.push(path.join(arg, "crash.txt"));
    } else files.push(arg);
  }
  return files;
}

function blank() {
  const s = { runs: 0, ms: 0, recordings: 0, recordMs: 0, fpsSum: 0, warn: 0, error: 0, shortConnections: 0, closed: 0, relayClosed: 0, maxOffsetMs: 0, crashFile: 0 };
  for (const k of Object.keys(COUNTS)) s[k] = 0;
  return s;
}

export function measure(texts) {
  const by = new Map();
  const get = (v) => by.get(v) ?? by.set(v, blank()).get(v);
  let version = "unknown", last = null, prevVersion = null;
  const seen = new Set();
  for (const text of texts) {
    for (const raw of text.split(/\r?\n/)) {
      // The same lines can come from two copies of one log (a report and the folder); count each once.
      const key = raw.slice(0, 200);
      if (seen.has(key)) continue;
      seen.add(key);
      // crash.txt: "<time> version X panic: ..."
      const crash = raw.match(/^\S+ version (\S+) panic: /);
      if (crash) { get(crash[1]).crashFile++; continue; }
      const m = raw.match(LINE);
      if (!m) continue;
      const [, time, level, msg] = m;
      const t = Date.parse(time + "Z");
      const start = msg.match(STARTED);
      if (start) {
        prevVersion = version;
        version = start[1];
        last = null;
        get(version).runs++;
      }
      const s = get(version);
      if (last !== null && t > last && t - last < GAP_MS) s.ms += t - last;
      last = t;
      if (level === "WARN") s.warn++;
      if (level === "ERROR") s.error++;
      for (const [k, re] of Object.entries(COUNTS)) {
        if (!re.test(msg)) continue;
        // "did not exit cleanly" is written by the next run, about the run before it.
        if (k === "unclean") get(prevVersion ?? version).unclean++;
        else s[k]++;
      }
      const rec = msg.match(/recording of (\d+) s ended: captured ([\d.]+) fps/);
      if (rec) { s.recordings++; s.recordMs += Number(rec[1]) * 1000; s.fpsSum += Number(rec[2]) * Number(rec[1]); }
      const closed = msg.match(/: closed.* on the (\S+) path after (\d+) s/);
      if (closed) {
        s.closed++;
        if (/relay/.test(closed[1])) s.relayClosed++;
        if (Number(closed[2]) < 60) s.shortConnections++;
      }
      const off = msg.match(/time check: this PC is (-?\d+) ms off/);
      if (off) s.maxOffsetMs = Math.max(s.maxOffsetMs, Math.abs(Number(off[1])));
    }
  }
  return by;
}

function compare(a, b) {
  const pa = a.split(/[.-]/).map(Number), pb = b.split(/[.-]/).map(Number);
  for (let i = 0; i < 3; i++) if ((pa[i] || 0) !== (pb[i] || 0)) return (pa[i] || 0) - (pb[i] || 0);
  return a.localeCompare(b);
}

export function table(by) {
  const rows = [];
  for (const v of [...by.keys()].sort(compare)) {
    const s = by.get(v), h = s.ms / 3.6e6, rh = s.recordMs / 3.6e6;
    const per = (n, hours) => (hours > 0 ? (n / hours).toFixed(2) : "-");
    const stalls = s.captureBehind + s.duplicated + s.encoderStalled;
    rows.push({
      version: v,
      "app h": h.toFixed(1),
      runs: s.runs,
      "unclean exits": s.unclean,
      panics: s.crashFile,
      "rec h": rh.toFixed(1),
      recordings: s.recordings,
      "avg fps": s.recordMs ? (s.fpsSum / (s.recordMs / 1000)).toFixed(1) : "-",
      "capture stalls / rec h": per(stalls, rh),
      connections: s.connected,
      "drops <60s": s.shortConnections,
      "relay share": s.closed ? `${Math.round((100 * s.relayClosed) / s.closed)}%` : "-",
      "connect fails / h": per(s.connectFailed, h),
      "sync stops": s.syncStopped + s.helloLost + s.fetchStopped,
      "live behind": s.liveBehind,
      "quality down": s.qualityLowered,
      "clock max off ms": s.maxOffsetMs,
      "warn / h": per(s.warn, h),
      "error / h": per(s.error, h),
    });
  }
  return rows;
}

// Metrics as rows and versions as columns, so a new version is one more column.
export function columns(rows) {
  const out = {};
  for (const row of rows) for (const [k, v] of Object.entries(row)) if (k !== "version") (out[k] ??= {})[row.version] = v;
  return out;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2);
  const json = args[0] === "--json";
  const files = inputs(json ? args.slice(1) : args);
  if (!files.length) {
    console.error("usage: node scripts/stability.mjs [--json] <data folder | log file | log .gz> ...");
    process.exit(2);
  }
  const by = measure(files.map(read));
  if (json) console.log(JSON.stringify(Object.fromEntries(by), null, 1));
  else console.table(columns(table(by)));
}

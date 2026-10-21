import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const ui = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "ui");
const read = (f) => fs.readFileSync(path.join(ui, f), "utf8");
const source = ["util.js", "i18n.js", "live.js", "media.js", "layout.js", "dock.js", "events.js", "markdown.js", "members.js", "recordings.js", "extensions.js", "ai.js", "diag.js"].map(read).join("\n");
const context = vm.createContext({ window: { __TAURI__: { core: {} } }, document: { querySelectorAll: () => [], documentElement: {} }, console, Intl, Date, URL, nameOf: (id) => id, location: { search: "" } });
const ui$ = vm.runInContext(`${source}\n({ matchEnds, matchChapters, stepTarget, matchTargets, groupDrawings, dayLabel, keysOf, memberStatus, viewLine, viewText, tileProblem, livePlayers, ordered, extensionLine, analysisReason, recordingAnalysis, cardParts, dropPos, deletePreview, oldSessions, onThisPc, sizeOf, canDelete, deleteForm, noPicture, soundKey, talking, speaking, seekSum, SPEAK_HANG_MS, downloadStats, sessionNotice, aiPurposes, aiPrompt, aiMenuItems, aiPreview, aiDebrief, apiUrl, apiBadge, apiReferenceBlocks, rateOf, liveEdge, netState, summaryLines, leftText, pcCards, clockSummary, flowRows, skewText, liveStep, liveFollow, liveAvail, plan, covers, endOf, waiting, AHEAD_S, HOLD_MAX_MS, RETRY_MS, LIVE_LAG_MAX_S, LIVE_HEADROOM_S, LIVE_SHRINK_GAP_S, LIVE_CATCH_UP, JA, t, sessionName, mdParse, mdInline, mdVouched, mdHooks, tagName, clampSide, captureStates, micLevel, meterPos, SIDE_MIN, SIDE_MAX, SIDE_DEFAULT, setTitles: (x) => { titles = x; }, setStorage: (x) => { st = x; }, setLanguage: (l) => { language = l; } })`, context);
const plain = (v) => JSON.parse(JSON.stringify(v));

const moment = (m, extra = {}) => ({ role: "moment", m, plugin: { id: "apex-matches" }, tags: ["match"], ...extra });

test("match ends come only from plugin moments tagged match", () => {
  const list = [moment(300), moment(100), moment(200, { tags: ["other"] }), moment(250, { plugin: null }), { role: "chapter", m: 50, tags: ["match"], plugin: {} }];
  assert.deepEqual([...ui$.matchEnds(list)], [100, 300]);
});

test("matches run between consecutive ends, with the open one only while recording", () => {
  assert.deepEqual(plain(ui$.matchChapters([100, 300], 500, false)), [{ m: 0, end: 100, n: 1 }, { m: 100, end: 300, n: 2 }]);
  assert.deepEqual(plain(ui$.matchChapters([100, 300], 500, true)).at(-1), { m: 300, end: 500, n: 3 });
  assert.equal(ui$.matchChapters([], 500, true).length, 0);
});

test("A and D land a minute before the previous or next match end", () => {
  const targets = ui$.matchTargets([30, 400, 900]);
  assert.deepEqual([...targets], [0, 340, 840]);
  assert.equal(ui$.stepTarget(targets, 340, 1, 0.5), 840);
  assert.equal(ui$.stepTarget(targets, 340, -1, 0.5), 0);
  assert.equal(ui$.stepTarget(targets, 900, 1, 0.5), undefined);
});

test("strokes drawn at one moment by one person are one row", () => {
  const d = (m, author) => ({ role: "drawing", m, author, setId: "s" });
  const rows = ui$.groupDrawings([d(10, "a"), d(10.5, "a"), d(10.6, "b"), { role: "moment", m: 11 }, d(30, "a")]);
  assert.deepEqual([...rows.map((r) => r.count ?? 0)], [2, 1, 0, 1]);
});

test("days read as today, yesterday or a date", () => {
  const now = new Date(2026, 9, 18, 21, 0);
  ui$.setLanguage("en");
  assert.equal(ui$.dayLabel("2026-10-18", now, "en"), "Today");
  assert.equal(ui$.dayLabel("2026-10-17", now, "en"), "Yesterday");
  assert.equal(ui$.dayLabel("2026-10-15", now, "en"), "Oct 15");
  assert.equal(ui$.dayLabel("2025-12-31", now, "en"), "Dec 31, 2025");
  ui$.setLanguage("ja");
  assert.equal(ui$.dayLabel("2026-10-17", now, "ja"), "昨日");
  assert.equal(ui$.dayLabel("2026-10-15", now, "ja"), "10月15日");
  ui$.setLanguage("en");
});

test("shortcuts need Ctrl or Alt and read like the native menu", () => {
  const key = (code, mods = {}) => ui$.keysOf({ code, key: "", ctrlKey: false, altKey: false, shiftKey: false, ...mods });
  assert.equal(key("KeyM", { ctrlKey: true, altKey: true }), "Ctrl+Alt+M");
  assert.equal(key("F9", { altKey: true, shiftKey: true }), "Alt+Shift+F9");
  assert.equal(key("Digit5", { ctrlKey: true }), "Ctrl+5");
  assert.equal(key("KeyM", { shiftKey: true }), null);
  assert.equal(key("ControlLeft", { ctrlKey: true }), null);
});


test("the delete preview lists every part that is here and says whether the recording goes completely", () => {
  const set = { id: "s1", shared: true, progress: { have: 30 }, players: [{ owner: "me", have: 10 }, { owner: "b", have: 20 }, { owner: "c", have: 0 }], debriefs: ["d1"] };
  assert.deepEqual(plain(ui$.deletePreview([set])), [{ id: "s1", bytes: 30, form: "Others still have it", players: [{ owner: "me", bytes: 10 }, { owner: "b", bytes: 20 }], debriefs: ["d1"] }]);
  assert.equal(ui$.deletePreview([{ ...set, shared: false }])[0].form, "This deletes it completely");
  const sets = ["s5", "s4", "s3", "s2", "s1"].map((id) => ({ id, live: false, progress: { have: id === "s2" ? 0 : 1 } }));
  assert.deepEqual([...ui$.oldSessions(sets, 2)], ["s3", "s1"]);
});

test("a row is on this PC or not, always shows a size, and deleting needs media here or nobody else", () => {
  const set = (extra) => ({ id: "s1", live: false, shared: false, wanted: false, progress: { have: 0, total: 900 }, ...extra });
  assert.equal(ui$.onThisPc(set()), false);
  assert.equal(ui$.sizeOf(set()), 900);
  assert.equal(ui$.onThisPc(set({ progress: { have: 400, total: 900 } })), true);
  assert.equal(ui$.sizeOf(set({ progress: { have: 400, total: 900 } })), 400);
  assert.equal(ui$.onThisPc(set({ wanted: true })), true);
  assert.equal(ui$.canDelete(set({ shared: true })), false);
  assert.equal(ui$.canDelete(set({ shared: false })), true);
  assert.equal(ui$.canDelete(set({ shared: true, progress: { have: 5, total: 9 } })), true);
  assert.equal(ui$.canDelete(set({ live: true, progress: { have: 5, total: 9 } })), false);
});

test("the two delete confirmations read the same in both languages", () => {
  for (const [form, en, ja] of [[true, "Others still have it", "ほかの人は持っています"], [false, "This deletes it completely", "完全に消えます"]]) {
    const text = ui$.deleteForm({ shared: form });
    assert.equal(text, en);
    ui$.setLanguage("ja");
    assert.equal(ui$.t(text), ja);
    ui$.setLanguage("en");
  }
});

test("receiving speed and the live edge", () => {
  assert.equal(ui$.rateOf([{ t: 0, have: 0 }, { t: 1000, have: 10 }], 1000), null);
  assert.equal(ui$.rateOf([{ t: 0, have: 0 }, { t: 2000, have: 4000 }], 2000), 2000);
  assert.equal(ui$.rateOf([{ t: 0, have: 0 }, { t: 2000, have: 4000 }], 60000), null);
  assert.deepEqual(plain(ui$.liveEdge(100, 98)), { behind: 2, at: true });
  assert.deepEqual(plain(ui$.liveEdge(100, 50)), { behind: 50, at: false });
  assert.equal(ui$.leftText(30), "30 s left");
  assert.equal(ui$.leftText(150), "3 min left");
});

test("the connection badge shows a relay when any member uses one", () => {
  assert.equal(ui$.netState([{ connected: false }]), null);
  assert.equal(ui$.netState([{ connected: true, path: "direct" }]), "direct");
  assert.equal(ui$.netState([{ connected: true, path: "direct" }, { connected: true, path: "relay" }]), "relay");
});

test("the diagnostics overview marks problems", () => {
  const st = {
    version: "0.9.0", names: { a: "Aki" }, sets: [{ progress: { have: 1, total: 2, stuck: true } }],
    peers: [{ id: "a", connected: true, path: "relay", rtt_ms: 80, version: "0.8.2" }],
    capture: { source: null, source_lost: false, status: "", mix: { mic: { muted: true } }, quality: { fps: 30, codec: "h264" } },
    disk: { free: 1e9, hours_left: 2, low: true, full: false },
  };
  const levels = Object.fromEntries(ui$.summaryLines(st, 1).map((l) => [l.key, l.level]));
  assert.deepEqual(levels, { Connections: "warn", Capture: "info", Microphone: "info", Transfers: "warn", Storage: "warn", Versions: "warn", Clocks: "info", Extensions: "fail" });
});

const hw = { gpu: "Test Graphics 9000", driver: "31.0.15.1234", encoder: "NVENC", codec: "hevc", cpu: "Fake CPU 3.5GHz", ram_mb: 16384, display: [2560, 1440], os: "Test Windows 10.0.99999", fps: 29.9, dropped: 2 };
const peerOf = (id, extra) => ({ id, connected: true, path: "direct", ...extra });

test("each PC lists its hardware and its clock, this PC against true time and members against this PC", () => {
  ui$.setLanguage("en");
  const st = { id: "me", hw, clock: { offset_us: 8_200_000 }, peers: [peerOf("a", { hw, offset_us: 12_000 }), peerOf("b", { offset_us: -4_500_000 }), peerOf("c", { connected: false, hw })] };
  const [me, a, b, c] = plain(ui$.pcCards(st));
  const row = (card, key) => card.rows.find((r) => r.key === key);
  assert.deepEqual(row(me, "Graphics"), { key: "Graphics", level: "info", text: "Test Graphics 9000 · 31.0.15.1234" });
  assert.equal(row(me, "Encoder").text, "NVENC · HEVC");
  assert.equal(row(me, "Memory").text, "16 GB");
  assert.equal(row(me, "Display").text, "2560 × 1440");
  assert.equal(row(me, "Capture").text, "29.9 fps · 2 dropped");
  assert.equal(row(me, "Capture").level, "ok");
  assert.deepEqual(row(me, "Clock"), { key: "Clock", level: "warn", text: "8.2 s behind" });
  assert.deepEqual(row(ui$.pcCards({ ...st, clock: { offset_us: 20_000 } })[0], "Clock").text, "In sync");
  assert.deepEqual(plain(row(ui$.pcCards({ ...st, clock: { offset_us: null } })[0], "Clock")), { key: "Clock", level: "warn", text: "Unreachable" });
  assert.deepEqual(row(a, "Clock"), { key: "Clock", level: "ok", text: "In sync" });
  assert.equal(row(a, "CPU").text, "Fake CPU 3.5GHz");
  assert.deepEqual(b.rows.map((r) => r.key), ["Hardware", "Clock"]);
  assert.equal(row(b, "Hardware").text, "Not shared");
  assert.deepEqual(row(b, "Clock"), { key: "Clock", level: "warn", text: "4.5 s behind" });
  assert.equal(c.dim, true);
  assert.equal(row(c, "Clock"), undefined);
});

test("dropped frames are a warning", () => {
  const many = ui$.pcCards({ id: "me", hw: { ...hw, dropped: 120 }, peers: [] })[0];
  assert.equal(many.rows.find((r) => r.key === "Capture").level, "warn");
  assert.equal(ui$.pcCards({ id: "me", hw: null, peers: [] })[0].rows[0].text, "Not shared");
});

test("clock differences read in milliseconds or seconds", () => {
  assert.equal(ui$.skewText(10_000), "In sync");
  assert.equal(ui$.skewText(250_000), "250 ms ahead");
  assert.equal(ui$.skewText(-1_250_000), "1.3 s behind");
});

test("the clock summary warns about a big spread between members", () => {
  const ok = { peers: [peerOf("a", { offset_us: 200_000 })] };
  assert.deepEqual(plain(ui$.clockSummary(ok)), { level: "ok", text: "Largest difference 200 ms" });
  assert.equal(ui$.clockSummary({ peers: [peerOf("a", { offset_us: 5_000_000 })] }).level, "warn");
  assert.equal(ui$.clockSummary({ peers: [peerOf("a", { offset_us: 2_000_000 }), peerOf("b", { offset_us: -2_000_000 })] }).level, "warn");
  assert.deepEqual(plain(ui$.clockSummary({ peers: [peerOf("a", {})] })), { level: "info", text: "Measuring…" });
  assert.equal(ui$.clockSummary({ peers: [] }).text, "No one else is online");
});

test("live tiles wait for the slowest member, never go back, stop waiting for one too far behind, mark it late once passed, and advance while nobody holds", () => {
  const { liveStep, LIVE_LAG_MAX_S: MAX, LIVE_HEADROOM_S: ROOM, LIVE_SHRINK_GAP_S: GAP, LIVE_CATCH_UP: UP } = ui$;
  const DT = 0.15;
  let now = 1000;
  const at = (key, avail, ended = false) => ({ key, avail, ended });
  let s = liveStep({ pos: null }, [at("a", now), at("b", now), at("slow", now - 6)], now, 0);
  assert.equal(s.pos, now - 6 - ROOM - GAP);
  const stalled = now - 6;
  let held = false;
  while (now < stalled + MAX - DT) {
    now += DT;
    const next = liveStep(s, [at("a", now), at("b", now), at("slow", stalled)], now, DT);
    assert.ok(next.pos >= s.pos && next.pos <= stalled - ROOM + 1e-9);
    assert.deepEqual(plain(next.late), []);
    held ||= next.held;
    s = next;
  }
  assert.ok(held && s.pos > stalled - ROOM - 1e-9);
  now = stalled + MAX + DT;
  s = liveStep(s, [at("a", now), at("b", now), at("slow", stalled)], now, DT);
  assert.ok(s.pos > stalled - ROOM && !s.held && s.rate === UP && !s.late.length);
  s = liveStep(s, [at("a", now), at("b", now), at("slow", stalled)], now, ROOM);
  assert.deepEqual(plain(s.late), ["slow"]);
  const alone = liveStep(s, [at("slow", stalled)], now, DT);
  assert.deepEqual(plain(alone), { pos: s.pos + DT, rate: 1, held: false, late: ["slow"] });
  assert.deepEqual(plain(liveStep(alone, [at("slow", alone.pos + DT)], now, DT).late), []);
});

test("the live delay shrinks slowly only while the slack is above the gap, and a tile with no start follows its own edge", () => {
  const { liveStep, LIVE_HEADROOM_S: ROOM, LIVE_SHRINK_GAP_S: GAP, LIVE_CATCH_UP: UP } = ui$;
  const now = 1000;
  const far = liveStep({ pos: now - ROOM - GAP - 2 }, [{ key: "a", avail: now, ended: false }], now, 1);
  assert.equal(far.rate, UP);
  assert.ok(Math.abs(far.pos - (now - ROOM - GAP - 2 + UP)) < 1e-9);
  const near = liveStep({ pos: now - ROOM - GAP }, [{ key: "a", avail: now, ended: false }], now, 1);
  assert.equal(near.rate, 1);
  const held = liveStep({ pos: now - ROOM - 0.1 }, [{ key: "a", avail: now, ended: false }], now, 1);
  assert.deepEqual([held.held, held.pos], [true, now - ROOM]);
  const others = liveStep({ pos: 900 }, [{ key: "a", avail: now, ended: false }, { key: "gone", avail: 100, ended: true }, { key: "none", avail: null, ended: false }], now, 1);
  assert.deepEqual(plain(others.late), []);
  assert.deepEqual(plain(liveStep({ pos: 5 }, [{ key: "none", avail: null, ended: false }], now, 1)), { pos: 6, rate: 1, held: false, late: [] });
  assert.equal(liveStep({ pos: null }, [], now, 1).pos, null);
  assert.deepEqual([ui$.liveFollow(20, 19.5), ui$.liveFollow(20, 14), ui$.liveFollow(20, 12), ui$.liveFollow(20, 20).playing].map(plain), [{ target: 19.5, playing: true }, { target: 14, playing: true }, { target: 18, playing: true }, false]);
  const ending = (end) => ({ buffered: { length: end === null ? 0 : 1, end: () => end } });
  assert.equal(ui$.liveAvail([[100, ending(5)], [90, ending(8)], [null, ending(30)], [80, ending(null)]]), 98);
  assert.equal(ui$.liveAvail([[null, ending(5)]]), null);
});

const piece = (n, len = 100) => ({ n, len, t_us: (n - 1) * 4e6 });
const index = (count, open = false) => ({ segs: Array.from({ length: count }, (_, i) => piece(i + 1)), open });

test("the player appends the segment that holds a far target, then the next ones up to the look-ahead, and nothing more", () => {
  const { plan, AHEAD_S } = ui$;
  const everything = index(1000);
  const buffered = [];
  const got = new Map();
  const appended = [];
  for (let next = plan(everything, buffered, got, 0, 2000.5, 0); next; next = plan(everything, buffered, got, next.n, 2000.5, 0)) {
    assert.equal(next.from, 0);
    appended.push(next.n);
    got.set(next.n, 100);
    buffered.push([(next.n - 1) * 4 + 0.05, next.n * 4]);
  }
  assert.deepEqual(appended, [501, 502, 503]);
  assert.ok((appended.at(-1) - 1) * 4 < 2000.5 + AHEAD_S);
  assert.deepEqual(plain(plan(everything, [], new Map(), 0, -3, 0)), { n: 1, from: 0 });
  assert.deepEqual(plain(plan(everything, [], new Map(), 0, 8.5, 5)), { n: 1, from: 0 });
});

test("a grown tail continues from the byte appended, and a tail that shrank or lost its place is appended whole again", () => {
  const { plan } = ui$;
  const grown = { segs: [piece(1), piece(2), { ...piece(3), len: 80 }], open: true };
  const here = [[0.05, 9]];
  const got = new Map([[1, 100], [2, 100], [3, 50]]);
  assert.deepEqual(plain(plan(grown, here, got, 3, 9, 0)), { n: 3, from: 50 });
  assert.deepEqual(plain(plan(grown, here, got, 2, 9, 0)), { n: 3, from: 0 });
  assert.equal(plan({ ...grown, segs: [piece(1), piece(2), { ...piece(3), len: 30 }] }, here, got, 3, 9, 0).from, 0);
  assert.equal(plan(grown, here, new Map([[1, 100], [2, 100], [3, 80]]), 3, 9, 0), null);
  assert.deepEqual(plain(plan(grown, [[0.05, 8]], new Map([[1, 100], [2, 100], [3, 80]]), 3, 9, 0)), { n: 3, from: 0 });
});

test("a last piece shorter than the probe is not asked for again, and following the edge first finishes the tail that has since grown", () => {
  const { plan } = ui$;
  const short = { segs: [piece(1), piece(2)], open: false };
  assert.equal(plan(short, [[0.05, 4.02]], new Map([[1, 100], [2, 100]]), 2, 5, 0), null);
  assert.deepEqual(plain(plan(short, [[0.05, 4]], new Map([[1, 100], [2, 100]]), 2, 5, 0)), { n: 2, from: 0 });
  const edge = { segs: [{ ...piece(1), len: 150 }, piece(2)], open: true };
  assert.deepEqual(plain(plan(edge, [[0.05, 4]], new Map([[1, 100]]), 1, Infinity, 0)), { n: 1, from: 100 });
  assert.deepEqual(plain(plan(edge, [[0.05, 8]], new Map([[1, 150]]), 1, Infinity, 0)), { n: 2, from: 0 });
});

test("a growing tail adds nothing to what is known to be there, and a lone piece lasts a fragment", () => {
  const { covers, endOf } = ui$;
  assert.deepEqual([endOf(index(0)), endOf(index(1)), endOf(index(3)), endOf(index(3, true))], [0, 1, 12, 8]);
  assert.deepEqual([covers(index(1), 0.5), covers(index(1), 1), covers(index(3), 11.9), covers(index(3), 12), covers(index(3, true), 7.9), covers(index(3, true), 8), covers(index(3), -0.1), covers(index(0), 0)], [true, false, true, false, true, false, false, false]);
});

test("a slot holds the others only while its data is here, it is not ready, and for a limited time", () => {
  const { waiting, HOLD_MAX_MS, RETRY_MS } = ui$;
  const slot = (over = {}) => ({ video: { paused: true, seeking: false, error: null, buffered: { length: 0 } }, got: new Map(), want: { part: { path: "p" } }, part: { path: "p" }, index: index(3), shift: 0, failedAt: 0, waitSince: 0, ...over });
  const now = 100 * RETRY_MS;
  assert.equal(waiting(slot(), false, 5, now), true);
  assert.equal(waiting(slot(), true, 5, now), false);
  assert.equal(waiting(slot(), false, 12, now), false);
  assert.equal(waiting(slot({ index: index(3, true) }), false, 9, now), false);
  assert.equal(waiting(slot({ index: null }), false, 5, now), true);
  assert.equal(waiting(slot({ part: { path: "q" } }), false, 5, now), true);
  const video = (over) => ({ paused: true, seeking: false, error: null, buffered: { length: 0 }, ...over });
  assert.equal(waiting(slot({ video: video({ paused: false }) }), false, 5, now), false);
  assert.equal(waiting(slot({ video: video({ paused: false, seeking: true }) }), false, 5, now), true);
  assert.equal(waiting(slot({ video: video({ error: {} }) }), false, 5, now), false);
  const ended = { got: new Map([[3, 100]]), video: video({ buffered: { length: 1, end: () => 10.5 } }) };
  assert.deepEqual([waiting(slot(ended), false, 11, now), waiting(slot(ended), false, 10, now), waiting(slot({ ...ended, index: index(3, true) }), false, 7, now)], [false, true, true]);
  assert.equal(waiting(slot({ failedAt: now - 1 }), false, 5, now), false);
  assert.equal(waiting(slot({ want: null }), false, 5, now), false);
  const long = slot();
  assert.deepEqual([waiting(long, false, 5, now), waiting(long, false, 5, now + HOLD_MAX_MS - 1), waiting(long, false, 5, now + HOLD_MAX_MS), waiting(long, true, 5, now + HOLD_MAX_MS), waiting(long, false, 5, now + HOLD_MAX_MS + 1)], [true, true, false, false, true]);
});

test("where data goes says when a relay or a server is involved", () => {
  ui$.setLanguage("en");
  const flow = { relay: "relay.example.net", downloads: [], update: { host: "github.com", channel: "beta" }, time: { host: "time.cloudflare.com", ok: true }, api: { on: false, lan: false } };
  const label = (k) => k;
  const names = { a: "Aki", b: "Ren" };
  const direct = ui$.flowRows({ names, peers: [peerOf("a")] }, flow, label);
  assert.deepEqual(plain(direct[0]), { key: "Members", level: "ok", text: "All direct" });
  assert.deepEqual(plain(direct.map((r) => r.key)), ["Members", "Address lookup", "Update check", "Time check", "Downloads"]);
  assert.deepEqual(plain(direct[3]), { key: "Time check", level: "info", text: "time.cloudflare.com · at start and every hour" });
  assert.deepEqual(plain(ui$.flowRows({ names, peers: [] }, { ...flow, time: { host: "time.cloudflare.com", ok: false } }, label)[3]), { key: "Time check", level: "warn", text: "time.cloudflare.com · Unreachable" });
  assert.equal(direct[1].text, "relay.example.net · Publishes only the relay address");
  assert.ok(direct[2].text.includes("github.com") && direct[2].text.includes("Beta"));
  const relayed = ui$.flowRows({ names, peers: [peerOf("a"), peerOf("b", { path: "relay", relay: "relay2.example.net" })] }, { ...flow, downloads: [{ key: "ffmpeg", hosts: ["files.example.org"] }], api: { on: true, lan: true } }, label);
  assert.deepEqual(plain(relayed[0]), { key: "Members", level: "warn", text: "Through a relay server: Ren" });
  assert.ok(relayed[1].text.startsWith("relay.example.net, relay2.example.net"));
  assert.deepEqual(plain(relayed[4]), { key: "Downloads", level: "warn", text: "Downloading ffmpeg from files.example.org" });
  assert.equal(relayed[5].level, "warn");
  assert.equal(ui$.flowRows({ names, peers: [] }, null, label)[0].text, "No one else is online");
});

test("the new Diagnostics texts have Japanese translations", () => {
  for (const word of ["PCs", "Hardware", "Graphics", "Encoder", "CPU", "Memory", "Display", "Windows version", "Capture", "Clock", "Clocks", "Members", "Address lookup", "Update check", "Time check", "Downloads", "Local API"]) assert.ok(word in ui$.JA, word);
});

test("every fixed text has a Japanese translation", () => {
  const used = new Set();
  for (const f of ["app.js", "side.js", "pages.js", "board.js", "extensions.js", "events.js", "util.js", "draw.js", "ai.js", "members.js", "recordings.js", "diag.js", "markdown.js"]) {
    const s = read(f);
    for (const m of s.matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"/g)) used.add(JSON.parse(`"${m[1]}"`));
    for (const m of s.matchAll(/\bt\([^)]*\?\s*"((?:[^"\\]|\\.)*)"\s*:\s*"((?:[^"\\]|\\.)*)"/g)) { used.add(JSON.parse(`"${m[1]}"`)); used.add(JSON.parse(`"${m[2]}"`)); }
  }
  for (const m of read("index.html").matchAll(/data-t(?:-title|-placeholder|-label)?="([^"]*)"/g)) used.add(m[1]);
  const missing = [...used].filter((k) => !(k in ui$.JA));
  assert.deepEqual(missing, []);
});

test("a focused player plays its own sound; otherwise the chosen player, else the viewer's own, else the first", () => {
  assert.equal(ui$.soundKey(3, ["a", "b", "c"], "a", "b"), "c");
  assert.equal(ui$.soundKey(0, ["a", "b", "c"], "b", "c"), "c");
  assert.equal(ui$.soundKey(0, ["a", "b", "c"], "b", null), "b");
  assert.equal(ui$.soundKey(0, ["a", "b"], "z", "y"), "a");
  assert.equal(ui$.soundKey(0, [], "a", "a"), null);
});

test("a voice counts as talking while loud and for the hang after", () => {
  const last = ui$.talking(-Infinity, 0.5, 1000);
  assert.equal(last, 1000);
  assert.equal(ui$.talking(last, 0.001, 1100), 1000);
  assert.equal(ui$.speaking(last, 1000 + ui$.SPEAK_HANG_MS - 1), true);
  assert.equal(ui$.speaking(last, 1000 + ui$.SPEAK_HANG_MS), false);
  assert.equal(ui$.talking(last, 0.5, 1500), 1500);
});

test("repeated seeks in one direction add up and a change of direction starts over", () => {
  assert.equal(ui$.seekSum(null, -5), -5);
  assert.equal(ui$.seekSum(-5, -5), -10);
  assert.equal(ui$.seekSum(-10, 5), 5);
  assert.equal(ui$.seekSum(5, 30), 35);
});

test("a download shows its percent, speed and time left", () => {
  assert.equal(ui$.downloadStats({ done: 500, total: 1000, speed: 250, left_s: 2 }).text, "50% · 250 B/s · 2 s left");
  assert.equal(ui$.downloadStats({ done: 0, total: 0, speed: 0, left_s: null }).text, "0%");
  assert.equal(ui$.downloadStats({ done: 2000, total: 1000, speed: 0, left_s: 0 }).n, 100);
});

test("everyone is told who started or stopped recording", () => {
  ui$.setLanguage("en");
  assert.equal(ui$.sessionNotice("start", "Aki"), "Aki started recording");
  assert.equal(ui$.sessionNotice("end", "Ren"), "Ren stopped recording");
  assert.equal(ui$.sessionNotice("end", ""), "Recording stopped");
  ui$.setLanguage("ja");
  assert.equal(ui$.sessionNotice("start", "Aki"), "Aki が録画を開始しました");
  assert.equal(ui$.sessionNotice("end", ""), "録画が終わりました");
  ui$.setLanguage("en");
});

test("the pages call a recording a recording, never a session", () => {
  const texts = [...Object.keys(ui$.JA), ...Object.values(ui$.JA)];
  assert.deepEqual(texts.filter((x) => /session|セッション/i.test(x)), []);
  for (const f of ["app.js", "side.js", "pages.js", "board.js", "extensions.js", "events.js", "util.js", "ai.js", "markdown.js", "index.html"]) {
    const shown = [...read(f).matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"|data-t(?:-title|-placeholder|-label)?="([^"]*)"/g)].map((m) => m[1] || m[2]);
    assert.deepEqual(shown.filter((x) => /session/i.test(x)), [], f);
  }
  for (const [en, ja] of [["Start recording", "録画開始"], ["Stop recording", "録画終了"], ["Recordings", "録画一覧"], ["Recording", "録画中"], ["Standby", "待機中"], ["Jump to latest", "最新に戻る"]]) assert.equal(ui$.JA[en], ja);
});

test("every English phrase in the pages has a Japanese one", () => {
  const files = fs.readdirSync(ui).filter((f) => f.endsWith(".js") && f !== "i18n.js");
  const used = new Set();
  for (const f of files) for (const m of read(f).matchAll(/\bt\("((?:[^"\\]|\\.)*)"/g)) used.add(JSON.parse(`"${m[1]}"`));
  for (const m of read("index.html").matchAll(/data-t(?:-title|-placeholder|-label)?="([^"]*)"/g)) used.add(m[1].replaceAll("&amp;", "&"));
  const missing = [...used].filter((k) => !(k in ui$.JA));
  assert.deepEqual(missing, []);
});

const info = { on: true, address: "127.0.0.1:47625", token: "0123456789abcdef" };
const room = { id: "ab12", name: "Practice" };

test("the AI menu offers a few prompts, or the switch when the API is off", () => {
  ui$.setLanguage("en");
  const calls = [];
  const actions = { enable: () => calls.push("enable"), show: (title, text, token) => calls.push({ title, text, token }) };
  const items = ui$.aiMenuItems(info, room, "2026-10-19 21-30-00", actions);
  assert.deepEqual(plain(items.map((i) => i.label)), ["Write notes from the transcript", "Summarize a recording into the game plan", "Find the mistakes in a match"]);
  assert.ok(items.length >= 3 && items.length <= 4);
  items[0].run();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].title, items[0].label);
  assert.match(calls[0].text, /Authorization: Bearer/);
  assert.ok(calls[0].text.includes(info.token));
  const preview = ui$.aiPreview(calls[0].text, calls[0].token);
  assert.ok(!preview.includes(info.token) && preview.includes("••••••••") && preview.includes("Authorization: Bearer"));
  const off = ui$.aiMenuItems({ ...info, on: false, token: "" }, room, null, actions);
  assert.deepEqual(plain(off.map((i) => i.label)), ["Turn on the local API"]);
  off[0].run();
  assert.equal(calls[1], "enable");
  assert.ok("Copy prompt" in ui$.JA);
});

test("a prompt names the API address, the token, llms.txt, the room and the recording", () => {
  ui$.setLanguage("en");
  const [notes, plan, mistakes] = ui$.aiPurposes();
  const text = ui$.aiPrompt(notes, info, room, "2026-10-19 21-30-00");
  for (const part of ["http://127.0.0.1:47625/llms.txt", "http://127.0.0.1:47625", info.token, "ab12", "Practice", "2026-10-19 21-30-00", "transcript"]) assert.ok(text.includes(part), part);
  assert.ok(ui$.aiPrompt(plan, info, room, null).includes("newest one in the room"));
  assert.ok(ui$.aiPrompt(mistakes, info, room, "x").includes("#mistake"));
  ui$.setLanguage("ja");
  const ja = ui$.aiPrompt(ui$.aiPurposes()[0], info, room, "2026-10-19 21-30-00");
  assert.ok(ja.includes(info.token) && ja.includes("http://127.0.0.1:47625/llms.txt") && ja.includes("ab12"));
  assert.match(ja, /文字起こし/);
  assert.doesNotMatch(ja, /Task:|Help my team/);
  assert.deepEqual(plain(ui$.aiPurposes().map((p) => p.label)), ["文字起こしからメモを書く", "録画を作戦ボードにまとめる", "試合のミスを探す"]);
  ui$.setLanguage("en");
  assert.equal(ui$.apiUrl("127.0.0.1:1"), "http://127.0.0.1:1");
});

test("the badge shows only while the API runs and warns when other devices can reach it", () => {
  ui$.setLanguage("en");
  assert.equal(ui$.apiBadge({ on: false, lan: true }), null);
  assert.deepEqual(plain(ui$.apiBadge({ on: true, lan: false })), { lan: false, title: "The local API is on" });
  assert.deepEqual(plain(ui$.apiBadge({ on: true, lan: true })), { lan: true, title: "Other devices on your network can control the app" });
  ui$.setLanguage("ja");
  assert.equal(ui$.apiBadge({ on: true, lan: true }).title, "ネットワーク上のほかの端末からアプリを操作できます");
  ui$.setLanguage("en");
});

test("endpoint examples that are JSON are shown as indented code and the rest as text", () => {
  const blocks = plain(ui$.apiReferenceBlocks(['{"a": 1, "b": [2]}', "Newest first.", '{"x": 1}   or   {"y": 2}', "42"]));
  assert.deepEqual(blocks.map((b) => b.code), [true, false, false, false]);
  assert.equal(blocks[0].text, '{\n  "a": 1,\n  "b": [\n    2\n  ]\n}');
  assert.equal(blocks[1].text, "Newest first.");
});

test("an imported recording is named by its title and the others by their time", () => {
  ui$.setLanguage("en");
  ui$.setTitles({ "2026-10-19 12-30-00": "Scrim vs Team B" });
  assert.equal(ui$.sessionName("2026-10-19 12-30-00"), "Scrim vs Team B");
  assert.match(ui$.sessionName("2026-10-19 13-00-00", new Date(2026, 9, 19)), /^Today 13:00$/);
  ui$.setTitles({});
});

test("the folder export action, the old files button and their phrases exist", () => {
  assert.match(read("index.html"), /id="oldfiles"/);
  assert.match(read("pages.js"), /"export-folder":/);
  for (const word of ["Imported", "Imported by {name}", "Export to folder…", "Delete old files ({size})", "Delete old files?"]) assert.ok(word in ui$.JA, word);
});

test("every fixed message the export code can send has a Japanese one", () => {
  const source = fs.readFileSync(path.join(ui, "..", "src-tauri", "src", "export.rs"), "utf8").split("#[cfg(test)]")[0];
  const used = new Set();
  for (const m of source.matchAll(/(?:Err\(|ok_or\(|ok_or_else\(\|\| |return Err\()"((?:[^"\\]|\\.)*)"/g)) used.add(JSON.parse(`"${m[1]}"`));
  for (const m of source.matchAll(/"((?:[A-Z][^"{}\\]*)\.)"(?:\.into\(\)|\.to_string\(\))/g)) used.add(JSON.parse(`"${m[1]}"`));
  const fixed = [...used].filter((x) => /^[A-Z]/.test(x) && x.includes(" ") && !x.includes("{"));
  assert.ok(fixed.length > 5, `found ${fixed.length} messages`);
  assert.deepEqual(fixed.filter((x) => !(x in ui$.JA)), []);
});

test("a member row says one short state", () => {
  const at = new Date(2026, 9, 19, 12, 0);
  ui$.setLanguage("en");
  const say = (m) => ui$.memberStatus({ connected: true, sharing: false, ...m }, at).text;
  assert.equal(say({}), "Not sharing");
  assert.equal(say({ sharing: true }), "Standby");
  assert.equal(say({ connected: false }), "Offline");
  assert.equal(say({ connected: false, seen_us: new Date(2026, 9, 19, 9, 5).getTime() * 1000 }), "Offline · last seen Today 09:05");
  ui$.setLanguage("ja");
  assert.equal(say({}), "共有していません");
  assert.equal(say({ sharing: true }), "待機中");
  assert.match(say({ connected: false }), /^オフライン/);
  ui$.setLanguage("en");
});
test("a recording is hidden from the list when it is a debrief of another, and debriefs are found by the recording they name", () => {
  const sets = [{ id: "s1", live: false, of: null }, { id: "d1", live: false, of: "s1" }, { id: "d2", live: false, of: "s1" }, { id: "s2", live: false, of: null }, { id: "live", live: true, of: null }];
  ui$.setStorage({ sets });
  const { recordingRows, debriefsOf } = vm.runInContext("({ recordingRows, debriefsOf })", context);
  assert.deepEqual(plain(recordingRows().map((r) => r.id)), ["s2", "s1"]);
  assert.deepEqual(plain(debriefsOf("s1").map((d) => d.id)), ["d1", "d2"]);
  assert.deepEqual(debriefsOf("s2"), []);
  assert.equal(ui$.noPicture({ players: [{ parts: [{ m: {} }] }] }), true);
  assert.equal(ui$.noPicture({ players: [{ parts: [{ v: {}, m: {} }] }] }), false);
  assert.equal(ui$.noPicture({ players: [] }), false);
});
test("the sidebar width stays between limits and the splitter takes the arrow keys", () => {
  assert.equal(ui$.clampSide(100), ui$.SIDE_MIN);
  assert.equal(ui$.clampSide(900), ui$.SIDE_MAX);
  assert.equal(ui$.clampSide(300.4), 300);
  assert.equal(ui$.clampSide("junk"), ui$.SIDE_DEFAULT);
  assert.equal(ui$.clampSide(null), ui$.SIDE_MIN);
  assert.ok(ui$.SIDE_MIN < ui$.SIDE_DEFAULT && ui$.SIDE_DEFAULT < ui$.SIDE_MAX);
  const focused = (a) => { context.document.activeElement = a; return vm.runInContext("nativeKey", context); };
  const key = (k) => ({ key: k });
  for (const k of ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"]) assert.equal(focused({ getAttribute: (n) => (n === "role" ? "separator" : null) })(key(k)), true, k);
  assert.equal(focused({ getAttribute: () => null, tagName: "DIV" })(key("ArrowLeft")), false);
  delete context.document.activeElement;
});

const blocks = (text) => plain(ui$.mdParse(text));
const kinds = (text) => blocks(text).map((b) => b.type);

test("markdown reads headings, lists and paragraphs only", () => {
  assert.deepEqual(kinds("# A\ntext\n\n- a\n- b\n\n1. x\n2. y"), ["heading", "paragraph", "list", "list"]);
  const list = blocks("- one\n- two\n  - nested\n- plain")[0];
  assert.deepEqual(list.items.map((i) => i.inline[0].text), ["one", "two", "plain"]);
  assert.equal(list.items[1].blocks[0].type, "list");
  assert.equal(blocks("1. a\n2. b")[0].ordered, true);
  assert.deepEqual(blocks("1. Drop\n2. Loot\n\nThen regroup.\n\n3. Rotate\n\nok\n\n0. zero").filter((b) => b.type === "list").map((b) => b.start), [1, 3, 0]);
  assert.equal(blocks("- a")[0].start, null);
  assert.deepEqual(kinds("| a | b |\n|---|---|\n| 1 | 2 |"), ["paragraph"]);
  assert.deepEqual(kinds("```\ncode\n```"), ["paragraph"]);
  assert.deepEqual(kinds("> quote\n\n---"), ["paragraph", "paragraph"]);
  assert.equal(blocks("- [x] done")[0].items[0].inline[0].text, "[x] done");
  assert.deepEqual(kinds("- a\n1. b"), ["list", "list"]);
});
test("markdown inline reads emphasis, code, links and tags, and only safe links become links", () => {
  const node = (text) => plain(ui$.mdInline(text));
  assert.deepEqual(node("**b** *i* ~~s~~ ==m== `c`").filter((n) => !n.text).map((n) => Object.keys(n)[0]), ["strong", "em", "strike", "mark", "code"]);
  assert.deepEqual(node("snake_case_name 2 * 3 * 4"), [{ text: "snake_case_name 2 * 3 * 4" }]);
  assert.equal(node("[site](https://example.com/a)")[0].link.url, "https://example.com/a");
  assert.equal(node("[wiki](https://ja.wikipedia.org/wiki/戦術)")[0].link.url, "https://ja.wikipedia.org/wiki/%E6%88%A6%E8%A1%93");
  assert.equal(node("https://例え.jp/a?q=作戦")[0].link.url, "https://xn--r8jz45g.jp/a?q=%E4%BD%9C%E6%88%A6");
  assert.equal(node("<https://www.google.com/maps/@48.85,2.35,15z>")[0].link.url, "https://www.google.com/maps/@48.85,2.35,15z");
  assert.equal(node("see https://example.com/a_b_c.")[1].link.url, "https://example.com/a_b_c");
  const wiki = "https://en.wikipedia.org/wiki/Apex_(video_game)";
  assert.equal(node(`see ${wiki} now`)[1].link.url, wiki);
  assert.deepEqual(node(`(${wiki}).`).map((n) => n.text ?? n.link.url), ["(", wiki, ")."]);
  assert.deepEqual(node(`[wiki](${wiki})`).map((n) => n.link.url), [wiki]);
  assert.deepEqual(node("https://example.com/a)").map((n) => n.text ?? n.link.url), ["https://example.com/a", ")"]);
  assert.equal(node("[x](ezh://moment/s1/12.5/all)")[0].link.link.m, 12.5);
  for (const bad of ["[x](javascript:alert(1))", "[x](file:///C:/Windows/notepad.exe)", "[x](data:text/html;base64,AAAA)", "![img](https://example.com/a.png)", "[x](ezh://moment/s1/-3/all)", "[x](//evil.example/a)"]) assert.ok(node(bad).every((n) => !n.link), bad);
  assert.equal(node("#plan and C#")[0].tag, "plan");
  assert.equal(node("\\#plan")[0].text, "#");
  assert.deepEqual(node("a <b>x</b> & <script>"), [{ text: "a <b>x</b> & <script>" }]);
  assert.deepEqual(node("```a``"), [{ text: "```a``" }]);
  assert.deepEqual(node("``a `b` c``"), [{ code: "a `b` c" }]);
  const started = performance.now();
  assert.deepEqual(node("ok " + "`".repeat(3990)), [{ text: "ok " + "`".repeat(3990) }]);
  assert.ok(performance.now() - started < 100);
});

test("a card is a bold first line over short markdown, with marked text, scene links and tags", () => {
  const head = (text) => plain(ui$.cardParts(text).head);
  assert.deepEqual(head("Win the first fight\n- a\n- b").map((n) => n.text), ["Win the first fight"]);
  assert.deepEqual(plain(ui$.cardParts("Goal\n- a\n- b").body).map((b) => b.type), ["list"]);
  assert.deepEqual(head("# Plan ==now==").map((n) => Object.keys(n)[0]), ["text", "mark"]);
  assert.equal(ui$.cardParts("").head.length, 0);
  assert.equal(ui$.cardParts("only a line").body.length, 0);
  const node = plain(ui$.mdInline("go [push](ezh://moment/s1/12.5/all) #rotate ==hold=="));
  assert.deepEqual(node.filter((n) => !n.text).map((n) => (n.link ? "link" : n.tag ? "tag" : "mark")), ["link", "tag", "mark"]);
});

test("dropping a card between two others takes the middle position, and at the ends one step beyond", () => {
  assert.equal(ui$.dropPos([1, 2, 3], 0), 0);
  assert.equal(ui$.dropPos([1, 2, 3], 1), 1.5);
  assert.equal(ui$.dropPos([1, 2, 3], 3), 4);
  assert.equal(ui$.dropPos([], 0), 0);
});
class Text {
  constructor(text) { this.text = text; }
  get textContent() { return this.text; }
}
class Fake {
  constructor(tag) { this.tag = tag; this.children = []; this.dataset = {}; this.style = {}; this.attributes = {}; this.classList = { toggle() {}, add() {}, remove() {}, contains: () => false }; }
  append(...items) { for (const x of items) this.children.push(typeof x === "object" && x !== null ? x : new Text(String(x))); }
  replaceChildren(...items) { this.children = []; this.append(...items); }
  setAttribute(k, v) { this.attributes[k] = v; }
  set textContent(v) { this.children = [new Text(String(v))]; }
  get textContent() { return this.children.map((c) => c.textContent).join(""); }
  find(tag) { return this.children.flatMap((c) => (c.tag === tag ? [c] : []).concat(c.find ? c.find(tag) : [])); }
}
function page(files, globals = {}) {
  const boxes = {};
  const document = { createElement: (tag) => new Fake(tag), createElementNS: (ns, tag) => new Fake(tag), getElementById: (id) => (boxes[id] ||= new Fake("div")), querySelectorAll: () => [], documentElement: {} };
  const ctx = vm.createContext({ window: { __TAURI__: { core: {} } }, document, console, Intl, Date, URL, location: { search: "" }, ...globals });
  vm.runInContext(files.map(read).join("\n"), ctx);
  return { ctx, boxes };
}

test("a card shows nothing a note can write as running or navigating", () => {
  const { ctx } = page(["util.js", "icons.js", "i18n.js", "layout.js", "events.js", "markdown.js"], { st: { palette: [] }, filterTag() {}, openLink() {} });
  const text = ["Plan", "[bad](javascript:alert(1)) <img src=x onerror=alert(1)> [ok](https://example.com/p) [m](ezh://moment/s1/5/all)", "- [ ] not a task", "| A | B |"].join("\n");
  const card = vm.runInContext(`cardDom(${JSON.stringify(text)}, mdHooks({ onOpen: () => {} }))`, ctx);
  assert.equal(card.find("table").length + card.find("pre").length + card.find("input").length, 0);
  assert.deepEqual(card.find("a").map((a) => a.title), ["https://example.com/p"]);
  assert.ok(card.find("a").every((a) => a.href === undefined));
  assert.equal(card.find("img").length + card.find("script").length, 0);
  assert.match(card.textContent, /javascript:alert\(1\)/);
  assert.match(card.textContent, /<img src=x onerror=alert\(1\)>/);
  assert.equal(card.find("button").filter((b) => b.className === "link").length, 1);
  const labels = [];
  ctx.onOpen = (url, label) => labels.push(label);
  const links = vm.runInContext(`cardDom("x\\nhttps://a.example/x <https://a.example@evil.example/y> [b](https://b.example/z)", mdHooks({ onOpen }))`, ctx).find("a");
  for (const a of links) a.onclick({ preventDefault() {}, stopPropagation() {} });
  assert.deepEqual(labels, ["https://a.example/x", "https://a.example@evil.example/y", "b"]);
  const dom = (t) => vm.runInContext(`cardDom(${JSON.stringify(t)}, mdHooks())`, ctx);
  assert.deepEqual(dom("Lead\n5. five\n6. six\n\n- x").find("ol").map((o) => o.start), [5]);
  assert.equal(dom("Lead\nbody").children[0].className, "chead");
});
test("a moment link keeps a formatted label, and deep lists render without running out of stack", () => {
  const { ctx } = page(["util.js", "icons.js", "i18n.js", "layout.js", "events.js", "markdown.js"], { st: { palette: [] }, filterTag() {}, openLink() {} });
  const label = (text) => vm.runInContext(`cardDom(${JSON.stringify("x\n" + text)}, mdHooks())`, ctx).find("button").filter((b) => b.className === "link")[0].textContent;
  assert.equal(label("[**Opening** push](ezh://moment/s1/5/all)"), "Opening push 0:05");
  assert.equal(label("[Rotate `B`](ezh://moment/s1/5/all)"), "Rotate B 0:05");
  assert.equal(label("ezh://moment/s1/5/all"), "Moment 0:05");
  const text = "- x\n" + Array.from({ length: 80 }, (_, k) => " ".repeat(2 * k + 2) + "- x").join("\n");
  const card = vm.runInContext(`cardDom(${JSON.stringify(text)}, mdHooks())`, ctx);
  assert.ok(card.find("ul").length <= 9);
});
test("a link's text vouches only for the host it names", () => {
  const hides = (label, url) => !ui$.mdVouched(new URL(url).href, label);
  for (const [label, url] of [["g\u043e\u043egle.com", "https://g\u043e\u043egle.com/login"], ["https://g\u043e\u043egle.com/login", "https://g\u043e\u043egle.com/login"], ["xn--ggle-55da.com", "https://xn--ggle-55da.com"], ["dropbox.com", "https://x.com/a"], ["paypal.com", "https://l.com"], ["google.com/search?q=evil.io", "https://evil.io/x"], ["https://www.google.com/search?q=evil.io", "https://evil.io/x"], ["google.com@evil.io", "https://evil.io"], ["Open the plan", "https://example.com"]]) assert.ok(hides(label, url), label);
  for (const [label, url] of [["example.com", "https://example.com/p"], ["https://www.example.com/a", "https://example.com/b"], ["the example.com page", "https://www.example.com"]]) assert.ok(!hides(label, url), label);
  assert.ok("Open {host}?" in ui$.JA);
});

test("the screen and microphone buttons share one state: off, on or a warning", () => {
  const cap = (source, muted) => ({ source, mix: { mic: { muted } } });
  assert.deepEqual(plain(ui$.captureStates(cap(null, true), "")), { screen: "off", mic: "off" });
  assert.deepEqual(plain(ui$.captureStates(cap({ kind: "display" }, false), "")), { screen: "on", mic: "on" });
  assert.deepEqual(plain(ui$.captureStates(cap({ kind: "display" }, true), "")), { screen: "on", mic: "off" });
  assert.equal(ui$.captureStates(cap({ kind: "display" }, false), "Capture stopped").screen, "warn");
  assert.equal(ui$.captureStates(cap(null, false), "").screen, "off");
});
test("the microphone button shows a live level that sits flat while muted", () => {
  assert.equal(ui$.micLevel(0, false), 0);
  assert.equal(ui$.micLevel(1, false), 1);
  assert.equal(ui$.micLevel(0.5, true), 0);
  const quiet = ui$.micLevel(0.01, false);
  const loud = ui$.micLevel(0.3, false);
  assert.ok(quiet > 0 && quiet < loud && loud < 1);
  assert.equal(ui$.meterPos(-60), 0);
});

const ctx = (extra = {}) => ({ me: "me", name: (id) => ({ me: "Aki", ren: "Ren", koya: "Koya" })[id] || id, online: (id) => id !== "koya", left: () => null, ...extra });
const view = (owner, extra = {}) => ({ owner, recording: false, end_us: 9, have: 5, total: 10, verified_by: ["me"], parts: [{ v: {}, m: {} }], ...extra });
const finishedSet = (players, extra = {}) => ({ id: "s1", live: false, wanted: false, players, ...extra });

test("the view line names the first member who blocks the playback, in one sentence", () => {
  ui$.setLanguage("en");
  const line = (set, c = ctx(), key) => ui$.viewLine(set, c, key);
  assert.equal(line(finishedSet([view("me"), view("ren")])), null);
  const missing = line(finishedSet([view("me"), view("ren", { verified_by: ["ren"], have: 0 })]));
  assert.deepEqual(plain({ text: missing.text, action: missing.action }), { text: "Ren's video missing", action: "Download" });
  assert.equal(ui$.viewText(missing), "Ren's video missing · Download");
  const receiving = (left) => line(finishedSet([view("me"), view("ren", { verified_by: ["ren"] })]), ctx({ left: () => left })).text;
  assert.equal(receiving(150), "Receiving Ren's video · 3 min left");
  assert.equal(receiving(null), "Receiving Ren's video");
  assert.equal(line(finishedSet([view("me"), view("ren", { verified_by: ["ren"], have: 0 })], { wanted: true })).text, "Receiving Ren's video");
  assert.equal(line(finishedSet([view("koya", { verified_by: ["koya"] })])).text, "Koya's video is only on Koya's PC");
  assert.equal(line(finishedSet([view("koya", { verified_by: ["ren", "koya"] })]), ctx({ online: () => false })).text, "Koya's video is only on Ren's PC");
  const silent = finishedSet([view("me", { parts: [{ m: {} }] })]);
  assert.equal(line(silent), null);
  assert.equal(line(silent, ctx(), "me").text, "Aki didn't record a screen");
  assert.equal(line(finishedSet([view("ren", { verified_by: ["ren"], recording: true })])), null);
  assert.equal(line({ ...finishedSet([view("ren", { verified_by: ["ren"] })]), live: true }), null);
  assert.equal(line(finishedSet([view("ren", { verified_by: ["ren"], have: 0 }), view("koya", { verified_by: ["koya"] })])).text, "Ren's video missing");
  ui$.setLanguage("ja");
  assert.equal(line(finishedSet([view("koya", { verified_by: ["koya"] })])).text, "Koya の映像は Koya のPCにしかありません");
  ui$.setLanguage("en");
});

test("a Live tile tells at most one problem, only while this PC records", () => {
  ui$.setLanguage("en");
  const now = 1000 * 1e6;
  const m = (extra) => ({ recording: true, sharing: true, now, start: now - 600e6, voiceEnd: now - 2e6, pictureEnd: now - 5e6, ...extra });
  assert.equal(ui$.tileProblem(m({})), null);
  assert.equal(ui$.tileProblem(m({ recording: false, sharing: false })), null);
  assert.equal(ui$.tileProblem(m({ sharing: false })), "Not recording screen");
  assert.equal(ui$.tileProblem(m({ voiceEnd: now - 61e6 })), "No voice for 1 min");
  assert.equal(ui$.tileProblem(m({ voiceEnd: now - 60e6 })), null);
  assert.equal(ui$.tileProblem(m({ pictureEnd: now - 15e6 })), "15 s behind");
  assert.equal(ui$.tileProblem(m({ sharing: false, voiceEnd: now - 90e6 })), "Not recording screen");
  assert.equal(ui$.tileProblem(m({ voiceEnd: null, start: null, pictureEnd: undefined })), null);
  assert.equal(ui$.tileProblem(m({ voiceEnd: null, start: now - 90e6 })), "No voice for 1 min");
  ui$.setLanguage("ja");
  assert.equal(ui$.tileProblem(m({ voiceEnd: now - 61e6 })), "1分間声がありません");
  ui$.setLanguage("en");
});

test("Live lists every room member in the saved order and then in join order, whatever their connection", () => {
  const state = (extra = {}) => ({
    id: "me", layout: { order: ["koya", "me"] }, capture: { source: null, live: null, working: false },
    peers: [{ id: "ren", connected: true, sharing: true, live: { size: 5, path: "p" } }, { id: "koya", connected: false, sharing: false }, { id: "mia", connected: true, sharing: false }],
    sets: [], ...extra,
  });
  const list = ui$.livePlayers(state());
  assert.deepEqual(plain(list.map((p) => p.key)), ["koya", "me", "ren", "mia"]);
  assert.deepEqual(plain(list.map((p) => p.online)), [false, true, true, true]);
  assert.deepEqual(plain(list.map((p) => p.sharing)), [false, false, true, false]);
  assert.equal(list[2].file.size, 5);
  assert.equal(ui$.livePlayers(state({ layout: null })).map((p) => p.key).join(), "me,ren,koya,mia");
  const recording = ui$.livePlayers(state({ capture: { source: {}, live: { size: 9 }, working: true }, sets: [{ live: true, players: [{ owner: "me", recording: true, last_us: { v: 3, m: 4 }, start_us: 10, offset_us: 2 }, { owner: "ren", recording: true }] }] }));
  const me = recording.find((p) => p.key === "me");
  assert.deepEqual(plain({ r: me.recording, s: me.sharing, last: me.last, start: me.start }), { r: true, s: true, last: { v: 3, m: 4 }, start: 8 });
  assert.equal(ui$.livePlayers(state({ capture: { source: {}, live: null, working: false }, sets: [{ live: true, players: [{ owner: "me", recording: true }] }] })).find((p) => p.key === "me").recording, false);
});

test("an extension row says one thing: no runner, what failed, who it waits for, or what is left", () => {
  ui$.setLanguage("en");
  const name = (id) => ({ a: "Ren", b: "Koya" })[id];
  const line = (p) => ui$.extensionLine({ on: true, runner: "a", left: 0, failed: 0, waiting_for: null, ...p }, name);
  assert.equal(line({}), "");
  assert.equal(line({ left: 3 }), "3 left · Ren's PC");
  assert.equal(line({ failed: 1, left: 3 }), "1 failed · Retry");
  assert.equal(line({ left: 2, waiting_for: "b" }), "Waiting for Koya's PC");
  assert.equal(line({ runner: null, left: 2, failed: 1 }), "No runner");
  assert.equal(line({ on: false, runner: null }), "");
  assert.equal(ui$.analysisReason({ state: "left", reason: "waiting", waiting_for: "a" }, name), "Waiting for Ren's PC");
  assert.equal(ui$.analysisReason({ state: "left", reason: "receiving" }, name), "Receiving");
  assert.equal(ui$.analysisReason({ state: "left", reason: "queue" }, name), "Waiting");
  assert.equal(ui$.analysisReason({ state: "failed" }, name), "Failed");
  assert.equal(ui$.analysisReason({ state: "failed", reason: "missing" }, name), "File missing");
  assert.equal(ui$.analysisReason({ state: "running", progress: 0.424 }, name), "Analyzing 42%");
  assert.equal(ui$.recordingAnalysis({ analysis: [{ state: "done" }, { state: "running", progress: 0.42 }] }).text, "Analyzing 42%");
  assert.equal(ui$.analysisReason({ state: "running", progress: null }, name), "Analyzing");
  assert.equal(ui$.recordingAnalysis({ analysis: [{ state: "running" }] }).text, "Analyzing");
  assert.equal(ui$.recordingAnalysis({ analysis: [{ state: "running", progress: 0.1 }, { state: "failed" }] }).text, "Analysis failed");
  assert.equal(ui$.recordingAnalysis({ analysis: [{ state: "done" }, { state: "left" }] }), null);
  ui$.setLanguage("ja");
  assert.equal(line({ left: 3 }), "残り 3 件 · Ren のPC");
  ui$.setLanguage("en");
});

test("every end-to-end step is named in docs/test-cases.md", () => {
  const cases = fs.readFileSync(path.join(ui, "..", "docs", "test-cases.md"), "utf8");
  const steps = [...fs.readFileSync(path.join(ui, "..", "scripts", "e2e.mjs"), "utf8").matchAll(/await step\("((?:[^"\\]|\\.)*)"/g)].map((m) => JSON.parse(`"${m[1]}"`));
  assert.deepEqual(steps.filter((s) => !cases.includes(s)), []);
});

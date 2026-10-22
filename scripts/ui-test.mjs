import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const ui = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "ui");
const read = (f) => fs.readFileSync(path.join(ui, f), "utf8");
const source = ["util.js", "i18n.js", "live.js", "media.js", "layout.js", "dock.js", "events.js", "markdown.js", "members.js", "recordings.js", "extensions.js", "ai.js", "diag.js"].map(read).join("\n");
const memory = new Map();
const localStorage = { getItem: (k) => memory.get(k) ?? null, setItem: (k, v) => memory.set(k, v) };
const context = vm.createContext({ localStorage, window: { __TAURI__: { core: {} } }, document: { querySelectorAll: () => [], documentElement: {} }, console, Intl, Date, URL, nameOf: (id) => id, location: { search: "" } });
const ui$ = vm.runInContext(`${source}\n({ LIMITS, cleanText, single, changed, matchEnds, noteMoments, progressText, matchChapters, stepTarget, matchTargets, groupDrawings, dayLabel, keysOf, hotkeyRows, takenKeys, memberStatus, sendsPicture, viewLine, tileProblem, livePlayers, noScreen, ordered, extensionLine, analysisReason, analysisState, cardParts, dropPos, dropSlot, boardSpan, boardFont, cellSpan, fitFont, deletePreview, exportChoice, exportArgs, oldSessions, onThisPc, sizeOf, canDelete, deleteForm, noPicture, soundKey, talking, stillTalking, seekSum, SPEAK_HANG_MS, downloadStats, sessionNotice, aiPurposes, aiPrompt, aiFacts, aiTask, aiTaskKey, aiMenuItems, aiPreview, aiCopyText, aiTaskOf, aiDebrief, apiUrl, apiReferenceBlocks, rateOf, liveEdge, netState, leftText, flowRows, liveStep, liveFollow, liveAvail, partyMove, sameParty, plan, grownIndex, covers, endOf, waiting, AHEAD_S, HOLD_MAX_MS, RETRY_MS, LIVE_LAG_MAX_S, LIVE_HEADROOM_S, LIVE_SHRINK_GAP_S, LIVE_CATCH_UP, JA, t, sessionName, mdParse, mdInline, mdVouched, mdHooks, tagName, mentionParts, chipMatches, nextChip, mentionWho, hintAt, applyHint, memberNames, clampSide, captureStates, micLevel, meterPos, SIDE_MIN, SIDE_MAX, SIDE_DEFAULT, setTitles: (x) => { titles = x; }, setStorage: (x) => { st = x; }, setLanguage: (l) => { language = l; } })`, context);
test("a changed() check against a tile's own store reports again on a rebuilt tile", () => {
  const first = {};
  assert.equal(ui$.changed("what", [1], first), true);
  assert.equal(ui$.changed("what", [1], first), false);
  assert.equal(ui$.changed("what", [1], {}), true);
});

const plain = (v) => JSON.parse(JSON.stringify(v));

const moment = (m, extra = {}) => ({ role: "moment", m, plugin: { id: "apex-matches" }, tags: ["match"], ...extra });

test("the page scripts together declare no name twice", () => {
  const files = [...read("index.html").matchAll(/<script src="([^"]+)"/g)].map((m) => m[1]);
  assert.doesNotThrow(() => new vm.Script(files.map(read).join("\n")));
});

test("match ends come only from plugin moments tagged match", () => {
  const list = [moment(300), moment(100), moment(200, { tags: ["other"] }), moment(250, { plugin: null }), { role: "chapter", m: 50, tags: ["match"], plugin: {} }];
  assert.deepEqual([...ui$.matchEnds(list)], [100, 300]);
});

test("Q and E step through notes and marks that have a time, in order", () => {
  const list = [moment(300, { kind: "note" }), moment(100, { kind: "mark" }), moment(200, { kind: "event" }), moment(250, { kind: "note", m: null }), moment(150, { kind: "note", role: "chapter" })];
  assert.deepEqual(ui$.noteMoments(list).map((n) => n.m), [100, 300]);
});

test("progress shows percent, speed and time left, and drops what is unknown", () => {
  assert.equal(ui$.progressText({ n: 42, speed: 3100000, left: 150 }), "42% · 3.1 MB/s · 3 min left");
  assert.equal(ui$.progressText({ n: 42, speed: null, left: null }), "42%");
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

test("each global shortcut row is named and can be reset only when it differs from its default", () => {
  const rows = ui$.hotkeyRows([{ action: "mark", keys: "Ctrl+Alt+K", default: "Ctrl+Alt+K" }, { action: "mic", keys: "Alt+F9", default: "Ctrl+Shift+M" }, { action: "screen", keys: "", default: "Ctrl+Shift+O" }]);
  assert.deepEqual(rows.map((r) => [r.label, r.changed]), [["Mark this moment", false], ["Microphone on / off", true], ["Screen on / off", true]]);
  for (const r of rows) assert.ok(ui$.JA[r.label], r.label);
});

test("a global shortcut another app holds shows its row in a warning state and its toast names the keys and action", () => {
  const hotkeys = [{ action: "mark", keys: "Ctrl+Alt+K", default: "Ctrl+Alt+K", taken: true }, { action: "mic", keys: "Ctrl+Shift+M", default: "Ctrl+Shift+M", taken: false }, { action: "screen", keys: "Ctrl+Shift+O", default: "Ctrl+Shift+O", taken: false }];
  assert.deepEqual(ui$.hotkeyRows(hotkeys).map((r) => r.warning), ["In use by another app", "", ""]);
  assert.ok(ui$.JA["In use by another app"]);
  assert.equal(ui$.takenKeys(hotkeys), "Ctrl+Alt+K (Mark this moment)");
  assert.equal(ui$.takenKeys(hotkeys.map((h) => ({ ...h, taken: false }))), "");
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

test("a party state moves the player only when it differs, quietly on joining, and never by a repeat of what was applied", () => {
  const here = { base: 1e6, now: 50e6, rate: 1, m: 30, playing: true, quiet: false };
  const p = { seq: 4, stamp: 9, origin: "b", wall_us: 11e6, at_us: 50e6, playing: true };
  assert.deepEqual(plain(ui$.partyMove(p, here)), { cmd: "seek", m: 10, quiet: false });
  assert.deepEqual(plain(ui$.partyMove(p, { ...here, quiet: true })), { cmd: "seek", m: 10, quiet: true });
  assert.deepEqual(plain(ui$.partyMove({ ...p, playing: false }, here)), { cmd: "pause", m: 10, quiet: false });
  assert.deepEqual(plain(ui$.partyMove({ ...p, wall_us: 31.5e6 }, here)), { m: 30.5, quiet: true });
  assert.equal(ui$.partyMove({ ...p, wall_us: 31.5e6 }, { ...here, quiet: true }), null);
  assert.deepEqual(plain(ui$.partyMove({ ...p, at_us: 40e6 }, here)), { cmd: "seek", m: 20, quiet: false });
  assert.equal(ui$.sameParty(p, { ...p }), true);
  assert.equal(ui$.sameParty({ ...p, seq: 0, stamp: -5 }, p), false);
  assert.equal(ui$.sameParty(p, null), false);
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

const peerOf = (id, extra) => ({ id, connected: true, path: "direct", ...extra });

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

test("a growing recording's index is fetched from its last piece on, and a tail that no longer starts there asks for the whole index again", () => {
  const { grownIndex } = ui$;
  const known = { segs: [piece(1), piece(2), { ...piece(3), len: 50 }], open: true };
  assert.deepEqual(plain(grownIndex(known, { segs: [piece(3), piece(4)], open: false })), { segs: [piece(1), piece(2), piece(3), piece(4)], open: false });
  assert.equal(grownIndex(known, { segs: [], open: false }), null);
  assert.equal(grownIndex(known, { segs: [piece(4)], open: true }), null);
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
  const flow = { relay: "relay.example.net", download: null, update: { host: "github.com" }, time: { host: "time.cloudflare.com", ok: true }, api: { on: false } };
  const names = { a: "Aki", b: "Ren" };
  const direct = ui$.flowRows({ names, peers: [peerOf("a")] }, flow);
  assert.deepEqual(plain(direct[0]), { key: "Members", level: "ok", text: "All direct" });
  assert.deepEqual(plain(direct.map((r) => r.key)), ["Members", "Address lookup", "Update check", "Time check", "Downloads"]);
  assert.deepEqual(plain(direct[3]), { key: "Time check", level: "info", text: "time.cloudflare.com · at start and every hour" });
  assert.deepEqual(plain(ui$.flowRows({ names, peers: [] }, { ...flow, time: { host: "time.cloudflare.com", ok: false } })[3]), { key: "Time check", level: "warn", text: "time.cloudflare.com · Unreachable" });
  assert.equal(direct[1].text, "relay.example.net · Publishes only the relay address");
  assert.equal(direct[2].text, "github.com · at start and every 4 h");
  const relayed = ui$.flowRows({ names, peers: [peerOf("a"), peerOf("b", { path: "relay", relay: "relay2.example.net" })] }, { ...flow, download: "files.example.org", api: { on: true } });
  assert.deepEqual(plain(relayed[0]), { key: "Members", level: "warn", text: "Through a relay server: Ren" });
  assert.ok(relayed[1].text.startsWith("relay.example.net, relay2.example.net"));
  assert.deepEqual(plain(relayed[4]), { key: "Downloads", level: "warn", text: "Downloading Video tools from files.example.org" });
  assert.deepEqual(plain(relayed[5]), { key: "Local API", level: "info", text: "This PC only" });
  assert.equal(ui$.flowRows({ names, peers: [] }, null)[0].text, "No one else is online");
});

test("the new Diagnostics texts have Japanese translations", () => {
  for (const word of ["Members", "Address lookup", "Update check", "Time check", "Downloads", "Local API"]) assert.ok(word in ui$.JA, word);
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
  assert.equal(ui$.stillTalking(last, 1000 + ui$.SPEAK_HANG_MS - 1), true);
  assert.equal(ui$.stillTalking(last, 1000 + ui$.SPEAK_HANG_MS), false);
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
  for (const f of ["app.js", "faces.js", "side.js", "pages.js", "board.js", "extensions.js", "events.js", "util.js", "ai.js", "markdown.js", "index.html"]) {
    const shown = [...read(f).matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"|data-t(?:-title|-placeholder|-label)?="([^"]*)"/g)].map((m) => m[1] || m[2]);
    assert.deepEqual(shown.filter((x) => /session/i.test(x)), [], f);
  }
  for (const [en, ja] of [["Start recording", "録画開始"], ["Stop recording", "録画終了"], ["Recordings", "録画一覧"], ["Recording", "録画中"], ["Standby", "待機中"], ["Jump to latest", "最新に戻る"]]) assert.equal(ui$.JA[en], ja);
});

const sourceFiles = (dir, ext) => fs.readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? sourceFiles(path.join(dir, e.name), ext) : e.name.endsWith(ext) ? [path.join(dir, e.name)] : []));
const literals = (text) => [...text.matchAll(/"((?:[^"\\\n]|\\.)*)"/g)].map((m) => m[1]);

test("every English phrase in the pages has a Japanese one, and every Japanese entry is used", () => {
  const files = fs.readdirSync(ui).filter((f) => f.endsWith(".js") && f !== "i18n.js");
  const html = read("index.html");
  const rust = sourceFiles(path.join(ui, "..", "src-tauri", "src"), ".rs").map((f) => fs.readFileSync(f, "utf8").split("#[cfg(test)]")[0]);
  const text = (m) => JSON.parse(`"${m}"`);
  const marked = [...html.matchAll(/data-t(?:-title|-placeholder|-label)?="([^"]*)"/g)].map((m) => m[1].replaceAll("&amp;", "&"));
  const phrases = new Set(marked);
  for (const s of files.map(read)) {
    for (const m of s.matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"/g)) phrases.add(text(m[1]));
    for (const m of s.matchAll(/\bt\([^)]*\?\s*"((?:[^"\\]|\\.)*)"\s*:\s*"((?:[^"\\]|\\.)*)"/g)) [m[1], m[2]].forEach((x) => phrases.add(text(x)));
  }
  const messages = new Set();
  for (const s of rust) {
    for (const m of s.matchAll(/(?:Err\(|ok_or\(|ok_or_else\(\|\| |return Err\()"((?:[^"\\]|\\.)*)"/g)) messages.add(text(m[1]));
    for (const m of s.matchAll(/"((?:[A-Z][^"{}\\]*)\.)"(?:\.into\(\)|\.to_string\(\))/g)) messages.add(text(m[1]));
  }
  const fixed = [...messages].filter((x) => /^[A-Z]/.test(x) && x.includes(" ") && !x.includes("{"));
  assert.ok(fixed.length > 5, `found ${fixed.length} messages`);
  assert.deepEqual([...phrases, ...fixed].filter((x) => !(x in ui$.JA)), []);
  const strings = new Set([...files.flatMap((f) => literals(read(f))), ...marked, ...rust.flatMap(literals)].map((x) => x.replaceAll("_", " ").toLowerCase()));
  assert.deepEqual(Object.keys(ui$.JA).filter((k) => !strings.has(k.toLowerCase())), []);
});

test("the folder export is a menu action of the page", () => {
  assert.match(read("pages.js"), /"export-folder":/);
});

test("the export options fall back to the defaults and to everyone for a member not in the recording, and send no member for everyone", () => {
  const defaults = { range: "whole", quality: "standard", member: "" };
  assert.deepEqual(plain(ui$.exportChoice(null, ["a"])), defaults);
  assert.deepEqual(plain(ui$.exportChoice({ range: "x", quality: 3, member: "gone" }, ["a"])), defaults);
  assert.deepEqual(plain(ui$.exportChoice({ range: "matches", quality: "light", member: "a" }, ["a", "b"])), { range: "matches", quality: "light", member: "a" });
  assert.deepEqual(plain(ui$.exportArgs(defaults)), { range: "whole", quality: "standard", member: null });
  assert.deepEqual(plain(ui$.exportArgs({ range: "matches", quality: "original", member: "b" })), { range: "matches", quality: "original", member: "b" });
});

const info = { on: true, address: "127.0.0.1:47625", token: "0123456789abcdef" };
const room = { id: "ab12", name: "Practice" };

test("the AI menu offers a few prompts, or the switch when the API is off", () => {
  ui$.setLanguage("en");
  const calls = [];
  const actions = { enable: () => calls.push("enable"), show: (purpose, text, token) => calls.push({ title: purpose.label, text, token }) };
  const items = ui$.aiMenuItems(info, room, "2026-10-19 21-30-00", actions);
  assert.deepEqual(plain(items.map((i) => i.label)), ["Write notes from the voices", "Summarize a recording into the game plan", "Find the mistakes in a match"]);
  assert.ok(items.length >= 3 && items.length <= 4);
  items[0].run();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].title, items[0].label);
  assert.ok(calls[0].text.includes(info.token));
  const preview = ui$.aiPreview(calls[0].text, calls[0].token);
  assert.ok(!preview.includes(info.token) && preview.includes("••••••••") && preview.includes("llms.txt"));
  const off = ui$.aiMenuItems({ ...info, on: false, token: "" }, room, null, actions);
  assert.deepEqual(plain(off.map((i) => i.label)), ["Turn on the local API"]);
  off[0].run();
  assert.equal(calls[1], "enable");
  assert.ok("Copy prompt" in ui$.JA);
  assert.ok("This app does not run the AI. Copy the prompt and give it to your own AI agent (such as Claude Code); it works through the local API." in ui$.JA);
});

test("a prompt names the API address, the token, llms.txt, the room and the recording", () => {
  ui$.setLanguage("en");
  const [notes, plan, mistakes] = ui$.aiPurposes();
  const text = ui$.aiPrompt(notes, info, room, "2026-10-19 21-30-00");
  for (const part of ["http://127.0.0.1:47625/llms.txt", "http://127.0.0.1:47625", info.token, "ab12", "Practice", "2026-10-19 21-30-00", "Transcribe the voices"]) assert.ok(text.includes(part), part);
  for (const p of [plan, mistakes, ui$.aiDebrief()]) assert.ok(ui$.aiPrompt(p, info, room, "x").includes(p.task));
  assert.ok(ui$.aiPrompt(plan, info, room, null).includes("the newest one"));
  assert.ok(ui$.aiPrompt(mistakes, info, room, "x").includes("#mistake"));
  ui$.setLanguage("ja");
  const ja = ui$.aiPrompt(ui$.aiPurposes()[0], info, room, "2026-10-19 21-30-00");
  assert.ok(ja.includes(info.token) && ja.includes("http://127.0.0.1:47625/llms.txt") && ja.includes("ab12"));
  assert.match(ja, /文字に起こし/);
  assert.doesNotMatch(ja, /Transcribe|API: .*\n.*Token:/);
  assert.deepEqual(plain(ui$.aiPurposes().map((p) => p.label)), ["声からメモを書く", "録画を作戦ボードにまとめる", "試合のミスを探す"]);
  ui$.setLanguage("en");
  assert.equal(ui$.apiUrl("127.0.0.1:1"), "http://127.0.0.1:1");
});

test("an edited task is used for its purpose and the default comes back when it is cleared", () => {
  ui$.setLanguage("en");
  const [notes, plan] = ui$.aiPurposes();
  assert.equal(ui$.aiTask(notes), notes.task);
  memory.set(ui$.aiTaskKey(notes), JSON.stringify("Only list the callouts."));
  assert.equal(ui$.aiTask(notes), "Only list the callouts.");
  assert.equal(ui$.aiTask(plan), plan.task);
  memory.set(ui$.aiTaskKey(notes), "null");
  assert.equal(ui$.aiTask(notes), notes.task);
});

test("the whole prompt is one text: copy swaps the mask for the token and only the task before the first blank line is kept", () => {
  ui$.setLanguage("en");
  const [notes] = ui$.aiPurposes();
  const shown = ui$.aiPreview(ui$.aiPrompt(notes, info, room, "x"), info.token);
  assert.ok(!shown.includes(info.token) && shown.includes("••••••••"));
  assert.equal(ui$.aiCopyText(shown, info.token), ui$.aiPrompt(notes, info, room, "x"));
  const noMask = shown.replaceAll("••••••••", "");
  assert.equal(ui$.aiCopyText(noMask, info.token), noMask);
  assert.equal(ui$.aiTaskOf(`Only list the callouts.\n\n${shown}`), "Only list the callouts.");
  assert.equal(ui$.aiTaskOf(shown), notes.task);
  assert.equal(ui$.aiTaskOf("\n\nfacts"), "");
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

test("a member row says one short state", () => {
  const at = new Date(2026, 9, 19, 12, 0);
  ui$.setLanguage("en");
  const say = (m) => ui$.memberStatus({ connected: true, sharing: false, ...m }, at).text;
  assert.equal(say({}), "Not sharing");
  assert.equal(say({ sharing: true }), "Standby");
  assert.equal(say({ sharing: true, relayed: true }), "No picture via relay");
  assert.equal(say({ relayed: true }), "Not sharing");
  assert.equal(say({ connected: false }), "Offline");
  assert.equal(say({ connected: false, seen_us: new Date(2026, 9, 19, 9, 5).getTime() * 1000 }), "Offline · last seen Today 09:05");
  ui$.setLanguage("ja");
  assert.equal(say({}), "共有していません");
  assert.equal(say({ sharing: true }), "待機中");
  assert.equal(say({ sharing: true, relayed: true }), "中継のため映像なし");
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
test("a player member gets a voice chip instead of a tile only when every part it is known to have is voice", () => {
  assert.equal(ui$.noScreen({ parts: [{ m: {} }, { m: {} }] }), true);
  assert.equal(ui$.noScreen({ parts: [{ m: {} }, { v: {}, m: {} }] }), false);
  assert.equal(ui$.noScreen({ parts: [] }), false);
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

test("a resized card snaps to whole grid cells within the limits", () => {
  assert.deepEqual(plain(ui$.boardSpan({ w: 4, h: 5 }, 130, -70, 100, 64)), { w: 5, h: 4 });
  assert.deepEqual(plain(ui$.boardSpan({ w: 4, h: 5 }, -900, 2000, 100, 64)), { w: 2, h: 16 });
  assert.deepEqual(plain(ui$.boardSpan({ w: 10, h: 2 }, 900, -200, 100, 64)), { w: 12, h: 2 });
});

test("board text starts near the body size, grows only a little on wide boards, and shrinks in half pixels to the largest size that fits, never below the floor", () => {
  assert.equal(ui$.boardFont(13, 820), 13);
  assert.equal(ui$.boardFont(13, 1500), 13);
  assert.equal(ui$.boardFont(13, 1980), 14.5);
  assert.equal(ui$.boardFont(13, 3000), 15);
  assert.equal(ui$.fitFont(() => true, 14), 14);
  assert.equal(ui$.fitFont((f) => f <= 11.5, 14), 11.5);
  assert.equal(ui$.fitFont(() => false, 14), 10);
  assert.equal(ui$.fitFont(() => true, 9), 10);
  assert.equal(ui$.cellSpan(100, 4, 12), 28);
  assert.equal(ui$.cellSpan(0, 4, 0), 1);
});

test("a dragged card lands before the card under the pointer's left half, after its right half, and in reading order between cards", () => {
  const rects = [{ left: 0, right: 100, top: 0, bottom: 100 }, { left: 120, right: 220, top: 0, bottom: 200 }, { left: 0, right: 100, top: 120, bottom: 200 }];
  assert.equal(ui$.dropSlot(rects, 140, 50), 1);
  assert.equal(ui$.dropSlot(rects, 200, 150), 2);
  assert.equal(ui$.dropSlot(rects, 110, 50), 1);
  assert.equal(ui$.dropSlot(rects, 300, 300), 3);
  assert.equal(ui$.dropSlot(rects, -5, -5), 0);
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

test("an avatar shows the image, a built-in shape or the initial; an unknown shape shows the initial; a room without an icon shows a preset picked from its id, and a room with no id its initial", () => {
  const image = "data:image/webp;base64,AAAA";
  const state = { id: "me", name: "Aki", names: { ren: "Ren", koya: "Koya", mia: "Mia" }, colors: { me: 1, ren: 2, koya: 3, mia: 4 }, avatars: { me: "tiger", ren: image, koya: "gone", mia: "data:image/svg+xml;base64,AAAA" } };
  const { ctx } = page(["util.js", "faces.js"], { st: state, nameOf: (id) => id, colorIndex: (id) => state.colors[id] || 0 });
  const draw = (code) => vm.runInContext(code, ctx);
  const tiger = draw(`avatar("me", "sm")`);
  assert.deepEqual([tiger.find("path").length, tiger.textContent, tiger.style.background], [1, "", "var(--member-sky)"]);
  assert.equal(draw(`avatar("me", "sm")`).find("path")[0].attributes.d, draw(`FACES.tiger`));
  assert.equal(draw(`avatar("ren")`).find("img")[0].src, image);
  for (const id of ["koya", "mia"]) assert.deepEqual([draw(`avatar("${id}")`).textContent, draw(`avatar("${id}")`).find("img").length], [id === "koya" ? "K" : "M", 0]);
  const room = draw(`roomIcon({ name: "Practice", icon: "" }, "sm")`);
  assert.deepEqual([room.textContent, room.className], ["P", "av sm room"]);
  const preset = draw(`roomIcon({ id: "ab12", name: "Practice", icon: "" }, "sm")`);
  assert.deepEqual([preset.textContent, preset.find("path").length, preset.className], ["", 1, "av sm room"]);
  const shape = (icon) => draw(`roomIcon({ id: "ab12", name: "Practice", icon: "${icon}" }, "sm")`).find("path")[0]?.attributes.d;
  assert.deepEqual([draw(`ROOM_PRESETS.length`), draw(`ROOM_PRESETS.some((n) => FACES[n])`)], [16, false]);
  assert.equal(shape("bolt"), draw(`ROOM_FACES.bolt`));
  assert.equal(shape("cat"), draw(`FACES.cat`));
  assert.equal(shape("unknown"), undefined);
  assert.ok(draw(`ROOM_PRESETS`).includes(draw(`defaultPreset("ab12")`)));
  assert.equal(draw(`roomIcon({ id: "ab12", name: "Other", icon: "" }, "sm")`).style.background, preset.style.background);
  assert.equal(draw(`roomIcon({ name: "Practice", icon: "${image}" }, "sm")`).find("img").length, 1);
  assert.deepEqual(plain(draw(`look("ren")`)), [2, image.length, image.slice(-16)]);
});

test("a card shows nothing a note can write as running or navigating", () => {
  const { ctx } = page(["util.js", "icons.js", "i18n.js", "layout.js", "events.js", "markdown.js"], { st: {}, openLink() {} });
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
test("every rendered #tag and @mention is a chip that sends its kind, value and ids to the one app-wide pick", () => {
  const picks = [];
  const { ctx } = page(["util.js", "icons.js", "i18n.js", "layout.js", "events.js", "markdown.js"], { st: { id: "me", name: "Mia", names: { a: "Jo Smith" } }, openLink() {}, pickChip: (...a) => picks.push(a) });
  const chips = (code) => vm.runInContext(code, ctx).find("button").filter((b) => b.className === "tag");
  const card = chips(`cardDom("Plan @jo smith\\n- push #Rotate with @Mia", mdHooks())`);
  assert.deepEqual(card.map((b) => [b.dataset.kind, b.dataset.value, b.textContent]), [["mention", "Jo Smith", "@Jo Smith"], ["tag", "Rotate", "#Rotate"], ["mention", "Mia", "@Mia"]]);
  for (const b of card) b.onclick({ stopPropagation() {} });
  assert.deepEqual(plain(picks), [["mention", "Jo Smith", ["a"]], ["tag", "Rotate", []], ["mention", "Mia", ["me"]]]);
  const note = chips(`el("p", {}, ...inline("go @Jo #push", [{ id: "a", name: "Jo" }]))`);
  assert.deepEqual(note.map((b) => [b.dataset.kind, b.dataset.value]), [["mention", "Jo Smith"], ["tag", "push"]]);
});
test("picking a chip filters the recording; the same chip again in the player clears it, elsewhere it filters again", () => {
  const on = { kind: "tag", value: "Rotate", ids: [], set: "s1" };
  assert.deepEqual(plain(ui$.nextChip(null, "tag", "Rotate", [], "s1", true)), on);
  assert.equal(ui$.nextChip(on, "tag", "rotate", [], "s1", true), null);
  assert.deepEqual(plain(ui$.nextChip(on, "tag", "Rotate", [], "s1", false)), on);
  assert.deepEqual(plain(ui$.nextChip(on, "mention", "Jo", ["a"], "s1", true)), { kind: "mention", value: "Jo", ids: ["a"], set: "s1" });
});
test("a moment link keeps a formatted label, and deep lists render without running out of stack", () => {
  const { ctx } = page(["util.js", "icons.js", "i18n.js", "layout.js", "events.js", "markdown.js"], { st: {}, openLink() {} });
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

const ctx = (extra = {}) => ({ me: "me", name: (id) => ({ me: "Aki", ren: "Ren", koya: "Koya" })[id] || id, online: (id) => id !== "koya", ...extra });
const view = (owner, extra = {}) => ({ owner, recording: false, end_us: 9, have: 5, total: 10, verified_by: ["me"], parts: [{ v: {}, m: {} }], ...extra });
const finishedSet = (players, extra = {}) => ({ id: "s1", live: false, wanted: false, players, ...extra });

test("the view line names the first member who blocks the playback, in one sentence", () => {
  ui$.setLanguage("en");
  const line = (set, c = ctx(), key) => ui$.viewLine(set, c, key);
  assert.equal(line(finishedSet([view("me"), view("ren")])), null);
  const missing = line(finishedSet([view("me"), view("ren", { verified_by: ["ren"], have: 0 })]));
  assert.deepEqual(plain({ text: missing.text, action: missing.action }), { text: "Ren's video missing", action: "Download" });
  assert.equal(line(finishedSet([view("me"), view("ren", { verified_by: ["ren"] })])).text, "Receiving Ren's video");
  assert.equal(line(finishedSet([view("me"), view("ren", { verified_by: ["ren"], have: 0 })], { wanted: true })).text, "Receiving Ren's video");
  assert.equal(line(finishedSet([view("koya", { verified_by: ["koya"] })])).text, "Koya's video hasn't arrived");
  assert.equal(line(finishedSet([view("koya", { verified_by: ["ren", "koya"] })]), ctx({ online: () => false })).text, "Koya's video hasn't arrived");
  const peers = { ren: { connected: true, path: "relay" }, koya: { connected: true, path: "direct" } };
  assert.equal(line(finishedSet([view("koya", { verified_by: ["ren"] })]), ctx({ online: (id) => ui$.sendsPicture(peers[id]) })).text, "Koya's video hasn't arrived");
  assert.equal(line(finishedSet([view("ren", { verified_by: ["koya"] })]), ctx({ online: (id) => ui$.sendsPicture(peers[id]) })).text, "Receiving Ren's video");
  const silent = finishedSet([view("me", { parts: [{ m: {} }] })]);
  assert.equal(line(silent), null);
  assert.equal(line(silent, ctx(), "me"), null);
  assert.equal(line(finishedSet([view("ren", { verified_by: ["ren"], recording: true })])), null);
  assert.equal(line({ ...finishedSet([view("ren", { verified_by: ["ren"] })]), live: true }), null);
  assert.equal(line(finishedSet([view("ren", { verified_by: ["ren"], have: 0 }), view("koya", { verified_by: ["koya"] })])).text, "Ren's video missing");
  ui$.setLanguage("ja");
  assert.equal(line(finishedSet([view("koya", { verified_by: ["koya"] })])).text, "Koya の映像はまだ届いていません");
  ui$.setLanguage("en");
});

test("a Live tile tells at most one problem, only while this PC records", () => {
  ui$.setLanguage("en");
  const now = 1000 * 1e6;
  const m = (extra) => ({ recording: true, now, start: now - 600e6, voiceEnd: now - 2e6, pictureEnd: now - 5e6, ...extra });
  assert.equal(ui$.tileProblem(m({})), null);
  assert.equal(ui$.tileProblem(m({ recording: false, voiceEnd: now - 90e6 })), null);
  assert.equal(ui$.tileProblem(m({ voiceEnd: now - 61e6 })), "No voice for 1 min");
  assert.equal(ui$.tileProblem(m({ voiceEnd: now - 60e6 })), null);
  assert.equal(ui$.tileProblem(m({ pictureEnd: now - 15e6 })), "15 s behind");
  assert.equal(ui$.tileProblem(m({ voiceEnd: null, start: null, pictureEnd: undefined })), null);
  assert.equal(ui$.tileProblem(m({ voiceEnd: null, start: now - 90e6 })), "No voice for 1 min");
  ui$.setLanguage("ja");
  assert.equal(ui$.tileProblem(m({ voiceEnd: now - 61e6 })), "1分間声がありません");
  ui$.setLanguage("en");
});

test("Live lists every room member in the saved order and then in join order, whatever their connection, with a picture only while online and sharing", () => {
  const state = (extra = {}) => ({
    id: "me", layout: { order: ["koya", "me"] }, capture: { source: null, live: null, working: false },
    peers: [{ id: "ren", connected: true, sharing: true, live: { size: 5, path: "p" } }, { id: "koya", connected: false, sharing: false }, { id: "mia", connected: true, sharing: false }],
    sets: [], ...extra,
  });
  const list = ui$.livePlayers(state());
  assert.deepEqual(plain(list.map((p) => p.key)), ["koya", "me", "ren", "mia"]);
  assert.deepEqual(plain(list.map((p) => p.online)), [false, true, true, true]);
  assert.deepEqual(plain(list.map((p) => p.sharing)), [false, false, true, false]);
  assert.deepEqual(plain(list.map((p) => p.picture)), [false, false, true, false]);
  assert.equal(ui$.livePlayers(state({ peers: [{ id: "koya", connected: false, sharing: true }] })).find((p) => p.key === "koya").picture, false);
  assert.equal(list[2].file.size, 5);
  const relayed = ui$.livePlayers(state({ peers: [{ id: "ren", connected: true, sharing: true, path: "relay", live: { size: 5 } }] })).find((p) => p.key === "ren");
  assert.deepEqual(plain({ relayed: relayed.relayed, file: relayed.file, picture: relayed.picture }), { relayed: true, file: null, picture: true });
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
  assert.equal(ui$.analysisReason({ state: "running", progress: null }, name), "Analyzing");
  assert.equal(ui$.analysisState({ state: "running", progress: 0.42 }, "a", name), "Analyzing 42% · Ren's PC");
  assert.equal(ui$.analysisState({ state: "left", reason: "receiving" }, "a", name), "Receiving");
  assert.equal(ui$.analysisState({ state: "done" }, "a", name), "Done");
  assert.equal(ui$.analysisState({ state: "idle" }, "a", name), "Not analyzed");
  ui$.setLanguage("ja");
  assert.equal(line({ left: 3 }), "残り 3 件 · Ren のPC");
  ui$.setLanguage("en");
});

test("every end-to-end step is named in docs/test-cases.md", () => {
  const cases = fs.readFileSync(path.join(ui, "..", "docs", "test-cases.md"), "utf8");
  const steps = [...fs.readFileSync(path.join(ui, "..", "scripts", "e2e.mjs"), "utf8").matchAll(/await step\("((?:[^"\\]|\\.)*)"/g)].map((m) => JSON.parse(`"${m[1]}"`));
  assert.deepEqual(steps.filter((s) => !cases.includes(s)), []);
});

test("mentions match member names, longest first, with spaces and Japanese, and not inside words", () => {
  const names = ["Jo Smith", "Jo", "たろう", "😀 Ace"];
  const parts = (text) => plain(ui$.mentionParts(text, names));
  assert.deepEqual(parts("hi @Jo Smith and @jo!"), ["hi ", { mention: "Jo Smith" }, " and ", { mention: "jo" }, "!"]);
  assert.deepEqual(parts("@たろう、@😀 Ace"), [{ mention: "たろう" }, "、", { mention: "😀 Ace" }]);
  assert.deepEqual(parts("mail a@Jo.com @Joe @"), ["mail a@Jo.com @Joe @"]);
  assert.deepEqual(parts(""), []);
});

test("a filter chip keeps notes with that tag or mention", () => {
  ui$.setStorage({ names: { a: "Jo" }, name: "Me" });
  const note = { body: "go @Jo #Rotate", tags: ["Rotate"] };
  assert.equal(ui$.chipMatches(note, null), true);
  assert.equal(ui$.chipMatches(note, { kind: "tag", value: "rotate" }), true);
  assert.equal(ui$.chipMatches(note, { kind: "tag", value: "push" }), false);
  assert.equal(ui$.chipMatches(note, { kind: "mention", value: "jo" }), true);
  assert.equal(ui$.chipMatches({ body: "plain", tags: [] }, { kind: "mention", value: "Jo" }), false);
  assert.deepEqual([...ui$.memberNames()], ["Jo", "Me"]);
});

test("a mention follows a renamed member by id and still matches the filter", () => {
  ui$.setStorage({ id: "me", names: { a: "Joe", b: "Joe", me: "You" }, name: "Mia" });
  const note = { body: "go @Jo now", mentions: [{ id: "a", name: "Jo" }] };
  assert.deepEqual(plain(ui$.mentionWho("Jo", note.mentions)), { value: "Joe", ids: ["a"] });
  assert.deepEqual(plain(ui$.mentionWho("Joe", null)), { value: "Joe", ids: ["a", "b"] });
  assert.deepEqual(plain(ui$.mentionWho("Gone", [{ id: "z", name: "Gone" }])), { value: "Gone", ids: ["z"] });
  assert.equal(ui$.chipMatches(note, { kind: "mention", value: "Joe", ids: ["a"] }), true);
  assert.equal(ui$.chipMatches(note, { kind: "mention", value: "Joe", ids: ["b"] }), false);
  assert.equal(ui$.chipMatches({ body: "go @Joe", mentions: null }, { kind: "mention", value: "joe", ids: ["a"] }), true);
  assert.equal(ui$.chipMatches({ body: "no mention", mentions: [] }, { kind: "mention", value: "Joe", ids: ["a"] }), false);
});

test("typing # or @ suggests known tags or member names and inserts the pick", () => {
  const tags = ["Rotate", "push"];
  const names = ["Jo Smith", "Jon", "たろう"];
  assert.deepEqual(plain(ui$.hintAt("go #ro", 6, tags, names, 5)).items, ["Rotate"]);
  assert.deepEqual(plain(ui$.hintAt("go #Rotate", 10, tags, names, 5)).items, []);
  assert.deepEqual(plain(ui$.hintAt("hi @jo", 6, tags, names, 5)).items, ["Jo Smith", "Jon"]);
  assert.deepEqual(plain(ui$.hintAt("hi @Jo S", 8, tags, names, 5)).items, ["Jo Smith"]);
  assert.deepEqual(plain(ui$.hintAt("hi @", 4, tags, names, 2)).items, ["Jo Smith", "Jon"]);
  assert.deepEqual(plain(ui$.hintAt("a@jo", 4, tags, names, 5)).items, []);
  assert.deepEqual(plain(ui$.hintAt("", 0, tags, names, 5)).items, []);
  const h = ui$.hintAt("hi @jo tail", 6, tags, names, 5);
  assert.deepEqual(plain(ui$.applyHint("hi @jo tail", 6, h, "Jo Smith")), { text: "hi @Jo Smith  tail", caret: 13 });
});

const rustConst = (file, name) => Number(new RegExp(`const ${name}: usize = (\\d+);`).exec(fs.readFileSync(path.join(ui, "..", "src-tauri", "src", file), "utf8"))[1]);

test("every text input limit in the page is the limit the app cuts at", () => {
  const { LIMITS } = ui$;
  assert.deepEqual([LIMITS.name, LIMITS.card, LIMITS.room, LIMITS.note, LIMITS.board, LIMITS.code, LIMITS.relay, LIMITS.relays], [rustConst("main.rs", "NAME_MAX"), rustConst("main.rs", "NAME_MAX"), rustConst("main.rs", "ROOM_NAME_MAX"), rustConst("main.rs", "NOTE_MAX"), rustConst("main.rs", "BOARD_MAX"), rustConst("room.rs", "CODE_MAX"), rustConst("relay.rs", "URL_MAX"), rustConst("relay.rs", "URLS_MAX")]);
});

test("typed text is cleaned like the app cleans it: one trimmed line, no invisible characters, counted in whole characters", () => {
  const clean = (s, n = 24) => ui$.cleanText(s, n);
  assert.equal(clean("  Aki\nRen\t "), "Aki Ren");
  assert.equal(clean("\u200B\u3000\u3164\uFEFF"), "");
  assert.equal(clean("A\u200Bk\u00ADi\u202E"), "Aki");
  assert.equal([...clean("あ".repeat(30))].length, 24);
  assert.equal([...clean("😀".repeat(30))].length, 24);
  assert.equal(clean("👨\u200D👩\u200D👧"), "👨\u200D👩\u200D👧");
  assert.equal([...clean(`a${"\u0301".repeat(30)}`)].length, 4);
  assert.equal(clean("<b>Aki</b> & \"Ren\""), "<b>Aki</b> & \"Ren\"");
});

test("a submit that is still running ignores the next click or Enter", async () => {
  let runs = 0;
  let release;
  const go = ui$.single(() => { runs++; return new Promise((done) => { release = done; }); });
  go(); go();
  assert.equal(runs, 1);
  release();
  await new Promise((done) => setTimeout(done, 0));
  go();
  assert.equal(runs, 2);
});

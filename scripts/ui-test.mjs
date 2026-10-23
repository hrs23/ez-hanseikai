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
const ui$ = vm.runInContext(`${source}\n({ uiCommands, uiState, uiRun, listMove, strokeAlpha, INK_TOOLS, INK_TOOL_TITLE, nextInkTool, LIMITS, escapeStep, editKey, cleanText, single, changed, matchEnds, matchStarts, matchLabel, labelMatchEnds, shownTags, noteMoments, progressText, matchChapters, stepTarget, matchTargets, rowHit, rowActions, authorOf, noteCount, matchCount, formatWhen, fullWhen, sessionWhen, keysOf, hotkeyRows, memberStatus, windowTitle, longId, sendsPicture, viewLine, tileProblem, livePlayers, noScreen, ordered, extensionLine, analysisReason, analysisState, analysisPill, analysisActions, analysisDetails, analysisDetailText, runningMark, cardParts, dropPos, dropSlot, boardSpan, boardFont, cellSpan, fitFont, deletePreview, exportChoice, exportArgs, exportView, oldSessions, onThisPc, sizeOf, canDelete, deleteForm, noPicture, soundKey, voiceTone, known, VOICE_SIDES, VOICE_TONES, VOICE_SIDE_DEFAULT, VOICE_TONE_DEFAULT, minimapRect, talking, stillTalking, markSpeak, seekSum, osdSeek, osdVolume, osdSpeed, SPEAK_HANG_MS, downloadStats, sessionNotice, aiPurposes, aiPrompt, aiFacts, aiTask, aiTaskKey, aiMenuItems, aiPreview, aiCopyText, aiTaskOf, aiDebrief, apiUrl, apiReferenceBlocks, rateOf, liveEdge, netState, leftText, flowRows, feedbackPreview, feedbackReady, errorReport, shownError, liveStep, liveFollow, liveAvail, partyMove, sameParty, plan, grownIndex, covers, endOf, waiting, AHEAD_S, HOLD_MAX_MS, RETRY_MS, LIVE_LAG_MAX_S, LIVE_HEADROOM_S, LIVE_SHRINK_GAP_S, LIVE_CATCH_UP, JA, t, sessionName, mdParse, mdInline, mdVouched, mdHooks, tagName, mentionParts, chipMatches, nextChip, mentionWho, hintAt, applyHint, memberNames, clampSide, captureStates, micLevel, meterPos, SIDE_MIN, SIDE_MAX, SIDE_DEFAULT, popupSpot, POP_GAP, POP_EDGE, setTitles: (x) => { titles = x; }, setStorage: (x) => { st = x; }, setLanguage: (l) => { language = l; } })`, context);
test("a changed() check against a tile's own store reports again on a rebuilt tile", () => {
  const first = {};
  assert.equal(ui$.changed("what", [1], first), true);
  assert.equal(ui$.changed("what", [1], first), false);
  assert.equal(ui$.changed("what", [1], {}), true);
});

const plain = (v) => JSON.parse(JSON.stringify(v));

const moment = (m, extra = {}) => ({ role: "moment", m, plugin: { id: "apex-matches" }, tags: ["match"], ...extra });

test("recorded voices take the chosen side and tone, Auto goes left only in a Watch Party, and Center with Clear leaves them as recorded", () => {
  const tone = (...a) => plain(ui$.voiceTone(...a));
  assert.equal(tone("auto", "clear", false), null);
  assert.deepEqual(tone("auto", "clear", true), { pan: -0.6 });
  assert.deepEqual(tone("auto", "strong", true), { highpass: 150, lowpass: 1800, pan: -0.8 });
  assert.deepEqual(tone("auto", "mild", false), { lowpass: 3000, pan: 0 });
  assert.deepEqual(tone("left", "clear", false), { pan: -0.6 });
  assert.deepEqual(tone("right", "clear", true), { pan: 0.6 });
  assert.equal(tone("center", "clear", true), null);
  assert.deepEqual(tone("center", "mild", false), { lowpass: 3000, pan: 0 });
  assert.equal(tone("up", "clear", true), null);
  assert.equal(tone("toString", "clear", true), null);
  assert.equal(tone("left", "loud", true), null);
  assert.equal(tone(ui$.VOICE_SIDE_DEFAULT, ui$.VOICE_TONE_DEFAULT, false), null);
  assert.deepEqual(tone(ui$.VOICE_SIDE_DEFAULT, ui$.VOICE_TONE_DEFAULT, true), { pan: -0.6 });
  assert.deepEqual([ui$.known("right", ui$.VOICE_SIDES, "auto"), ui$.known("mild", ui$.VOICE_SIDES, "auto"), ui$.known(null, ui$.VOICE_TONES, "clear"), ui$.known("toString", ui$.VOICE_TONES, "clear")], ["right", "auto", "clear", "clear"]);
  const html = read("index.html");
  for (const [id, table] of [["voiceside", ui$.VOICE_SIDES], ["voicetone", ui$.VOICE_TONES]]) assert.deepEqual([...html.match(new RegExp(`id="${id}"[^]*?</div>`))[0].matchAll(/data-v="(\w+)"/g)].map((m) => m[1]), Object.keys(table));
});

test("a speaking id marks every element that carries it and not the others, and the own muted mic never lights", () => {
  const node = (id) => { const c = new Set(); return { dataset: { speak: id }, classList: { toggle: (n, on) => (on ? c.add(n) : c.delete(n)), has: (n) => c.has(n) } }; };
  const nodes = [node("a"), node("b"), node("a"), node("me")];
  const lit = () => nodes.map((n) => n.classList.has("speaking"));
  ui$.markSpeak(nodes, new Set(["a"]), "me", false);
  assert.deepEqual(lit(), [true, false, true, false]);
  ui$.markSpeak(nodes, new Set(["a", "me"]), "me", false);
  assert.deepEqual(lit(), [true, false, true, true]);
  ui$.markSpeak(nodes, new Set(["a", "me"]), "me", true);
  assert.deepEqual(lit(), [true, false, true, false]);
  assert.equal(nodes[3].classList.has("micoff"), true);
  ui$.markSpeak(nodes, new Set(), "me", false);
  assert.deepEqual(lit(), [false, false, false, false]);
});

test("a menu or popup touches the edge of what opened it on the asked side, flips when that side is short, and stays inside the window", () => {
  const { popupSpot, POP_GAP: g, POP_EDGE: e } = ui$;
  const at = (left, top, right, bottom) => ({ left, top, right, bottom });
  const spot = (...a) => plain(popupSpot(...a));
  assert.deepEqual(spot(at(8, 12, 255, 52), 220, 110, 1280, 720, false), { left: 8, top: 52 + g });
  assert.deepEqual(spot(at(197, 686, 221, 710), 220, 40, 1280, 720, false), { left: 197, top: 686 - g - 40 });
  assert.deepEqual(spot(at(228, 628, 254, 672), 340, 116, 1280, 720, true), { left: 228, top: 628 - g - 116 });
  assert.deepEqual(spot(at(10, 20, 40, 50), 300, 200, 1280, 720, true), { left: 10, top: 50 + g });
  assert.deepEqual(spot(at(1236, 10, 1264, 38), 220, 70, 1280, 720, false), { left: 1264 - 220, top: 38 + g });
  assert.deepEqual(spot(at(600, 300, 600, 300), 220, 100, 1280, 720, false), { left: 600, top: 300 + g });
  assert.deepEqual(spot(at(600, 700, 600, 700), 220, 100, 1280, 720, false), { left: 600, top: 700 - g - 100 });
  for (let i = 0; i < 2000; i++) {
    const [vw, vh] = [400 + (i % 7) * 230, 300 + (i % 5) * 170];
    const left = e + ((i * 37) % (vw - 2 * e)), top = e + ((i * 53) % (vh - 2 * e)), a = at(left, top, Math.min(vw - e, left + (i % 4) * 30), Math.min(vh - e, top + (i % 3) * 20));
    const [w, h] = [Math.min(vw - 2 * e, 120 + (i % 9) * 40), Math.min(vh - 2 * e, 30 + (i % 11) * 60)];
    const p = popupSpot(a, w, h, vw, vh, i % 2 === 0);
    assert.ok(p.left >= e && p.top >= e && p.left + w <= vw - e && p.top + h <= vh - e, `outside ${JSON.stringify([a, w, h, vw, vh])}`);
    const fits = a.top - g - e >= h || vh - e - g - a.bottom >= h;
    const across = a.left + w <= vw - e || a.right - w >= e;
    if (fits && across) assert.ok((p.top === a.bottom + g || p.top + h === a.top - g) && p.left <= a.right && p.left + w >= a.left, `apart ${JSON.stringify([a, w, h, vw, vh, p])}`);
  }
  const app = read("app.js");
  for (const name of ["openMenu", "showPop"]) assert.match(app.match(new RegExp(`function ${name}\\([^]*?\\n}`))[0], /openPopup\(/);
  assert.match(read("style.css"), /\[popover\] \{ inset: auto; margin: 0;/);
});


const MINIMAP_SIZES = [[1280, 720], [1366, 768], [1600, 900], [1920, 1080], [1920, 1200], [2560, 1080], [2560, 1440], [3440, 1440], [3840, 1600], [3840, 2160], [5120, 1440], [1440, 1080], [1024, 768], [1280, 960], [720, 1280], [1000, 1000], [1280, 536], [1280, 360], [960, 540], [960, 270]];

test("the minimap crop is a square inside the frame, anchored top left from the height, at every capture size", () => {
  const byHeight = new Map();
  for (const [w, h] of MINIMAP_SIZES) {
    const r = ui$.minimapRect(w, h);
    const px = [r.x * w, r.y * h, r.w * w, r.h * h];
    assert.ok(px.every((v) => Number.isFinite(v) && v >= 0), `${w}x${h} ${px}`);
    assert.ok(px[2] > 0 && px[3] > 0, `${w}x${h} empty`);
    assert.ok(px[0] + px[2] <= w + 1e-9 && px[1] + px[3] <= h + 1e-9, `${w}x${h} outside`);
    assert.ok(Math.abs(px[2] - px[3]) < 1e-9, `${w}x${h} not square`);
    assert.ok(Math.abs(px[0] - 0.04 * h) < 1e-9 && Math.abs(px[1] - 0.04 * h) < 1e-9, `${w}x${h} not from the height`);
    if (h >= 720) assert.ok(px[2] >= 64, `${w}x${h} too small`);
    const same = byHeight.get(h);
    if (same && w >= h && same[0] >= h) assert.deepEqual([px[0], px[2]].map(Math.round), same[1], `${w}x${h} differs from ${same[0]}x${h}`);
    byHeight.set(h, [w, [px[0], px[2]].map(Math.round)]);
  }
  const px = (w, h) => { const r = ui$.minimapRect(w, h); return [r.x * w, r.y * h, r.w * w, r.h * h].map(Math.round); };
  assert.deepEqual(px(1280, 720), [29, 29, 169, 169]);
  assert.deepEqual(px(1920, 1080), [43, 43, 254, 254]);
  assert.deepEqual(px(3840, 2160), [86, 86, 508, 508]);
  assert.deepEqual(px(100, 1000), [40, 40, 60, 60]);
  for (const [w, h] of [[0, 0], [1920, 0], [0, 1080], [NaN, 1080], [1920, undefined], [-1, 1080], [10, 1000]]) assert.equal(ui$.minimapRect(w, h), null);
});

test("the page scripts together declare no name twice", () => {
  const files = [...read("index.html").matchAll(/<script src="([^"]+)"/g)].map((m) => m[1]);
  assert.doesNotThrow(() => new vm.Script(files.map(read).join("\n")));
});

test("match ends and starts come only from plugin moments tagged match, told apart by the start tag", () => {
  const list = [moment(300), moment(100), moment(200, { tags: ["other"] }), moment(250, { plugin: null }), { role: "chapter", m: 50, tags: ["match"], plugin: {} }, moment(20, { tags: ["match", "start"] }), moment(30, { tags: ["start"] })];
  assert.deepEqual([...ui$.matchEnds(list)], [100, 300]);
  assert.deepEqual([...ui$.matchStarts(list)], [20]);
});

test("a match end reads as the end of the match it closes and the chapters as the match, in English and Japanese", () => {
  const list = [moment(300, { text: "Match end" }), moment(100, { text: "Match end" }), moment(200, { kind: "note", text: "push", plugin: null, tags: [] })];
  const read = () => plain(ui$.labelMatchEnds(list).map((n) => [n.m, n.body ?? null, n.text]));
  list.push(moment(140, { text: "Match start", tags: ["match", "start"] }));
  assert.deepEqual(read(), [[300, "Match 2 end", "Match 2 end"], [100, "Match 1 end", "Match 1 end"], [200, null, "push"], [140, "Match 2 start", "Match 2 start"]]);
  assert.equal(ui$.matchLabel(2), "Match 2");
  ui$.setLanguage("ja");
  assert.deepEqual(read().map((r) => r[1]), ["試合 2 終了", "試合 1 終了", null, "試合 2 開始"]);
  assert.equal(ui$.matchLabel(2), "試合 2");
  ui$.setLanguage("en");
});

test("a plugin's match and start tags are not shown as chips and other tags are", () => {
  assert.deepEqual([...ui$.shownTags(moment(100, { tags: ["match", "start", "win"] }))], ["win"]);
  assert.deepEqual([...ui$.shownTags({ role: "moment", m: 1, plugin: null, tags: ["match"] })], ["match"]);
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

test("a match runs from its detected start, and from the previous end when the start is missing or implausible", () => {
  assert.deepEqual(plain(ui$.matchChapters([300, 700], 900, true, [20, 400, 760])), [{ m: 20, end: 300, n: 1 }, { m: 400, end: 700, n: 2 }, { m: 760, end: 900, n: 3 }]);
  assert.deepEqual(plain(ui$.matchChapters([300, 700], 900, false, [250, 650])), [{ m: 0, end: 300, n: 1 }, { m: 300, end: 700, n: 2 }]);
  assert.deepEqual(plain(ui$.matchChapters([300], 900, false, [900])), [{ m: 0, end: 300, n: 1 }]);
});

test("A and D land a minute before the previous or next match end", () => {
  const targets = ui$.matchTargets([30, 400, 900]);
  assert.deepEqual([...targets], [0, 340, 840]);
  assert.equal(ui$.stepTarget(targets, 340, 1, 0.5), 840);
  assert.equal(ui$.stepTarget(targets, 340, -1, 0.5), 0);
  assert.equal(ui$.stepTarget(targets, 900, 1, 0.5), undefined);
});

test("a click on a timeline row jumps only when it is not on a control, a selection or the end of a drag", () => {
  const on = (hit) => ({ closest: () => hit });
  assert.equal(ui$.rowHit(on(null), false, false), true);
  assert.equal(ui$.rowHit(on({}), false, false), false);
  assert.equal(ui$.rowHit(on(null), true, false), false);
  assert.equal(ui$.rowHit(on(null), false, true), false);
});

test("a timeline row menu lists only what its row can do", () => {
  const row = (o) => [...ui$.rowActions({ seekable: true, m: 5, text: "x", mine: true, kind: "note", ...o })];
  assert.deepEqual(row({}), ["go", "copy", "link", "edit", "delete"]);
  assert.deepEqual(row({ mine: false }), ["go", "copy", "link"]);
  assert.deepEqual(row({ kind: "mark", text: "" }), ["go", "link"]);
  assert.deepEqual(row({ seekable: false, m: null }), ["copy", "edit", "delete"]);
});

test("a timeline row shows the member's avatar, or the extension's icon for an extension event", () => {
  assert.deepEqual({ ...ui$.authorOf({ kind: "note", author: "a" }) }, { kind: "member", id: "a" });
  assert.deepEqual({ ...ui$.authorOf({ kind: "mark", author: "b" }) }, { kind: "member", id: "b" });
  assert.deepEqual({ ...ui$.authorOf({ kind: "match_end", author: "c", plugin: { id: "p" } }) }, { kind: "extension" });
});

test("counts leave out drawings, deleted notes and events that are gone", () => {
  const byKey = new Map([["a", { kind: "note" }], ["b", { kind: "drawing" }], ["c", { kind: "note" }], ["d", { kind: "note", plugin: { id: "p" }, tags: ["match"] }], ["e", { kind: "match_start", plugin: { id: "p" }, tags: ["match", "start"] }]]);
  const set = { events: ["a", "b", "c", "d", "e", "gone"] };
  assert.equal(ui$.noteCount(set, byKey), 3);
  assert.equal(ui$.matchCount(set, byKey), 1);
  byKey.delete("c");
  byKey.delete("b");
  assert.equal(ui$.noteCount(set, byKey), 2);
});

test("a time is shown with its date, never with relative words", () => {
  const now = new Date(2026, 9, 23, 0, 30);
  assert.equal(ui$.formatWhen(new Date(2026, 9, 23, 0, 1), now, "en"), "Oct 23 00:01");
  assert.equal(ui$.formatWhen(new Date(2026, 9, 22, 23, 59), now, "en"), "Oct 22 23:59");
  assert.equal(ui$.formatWhen(new Date(2026, 8, 30, 18, 8), now, "en"), "Sep 30 18:08");
  assert.equal(ui$.formatWhen(new Date(2025, 11, 31, 23, 59), now, "en"), "Dec 31, 2025 23:59");
  assert.equal(ui$.formatWhen(new Date(2026, 9, 23, 0, 1), now, "ja"), "10/23 00:01");
  assert.equal(ui$.formatWhen(new Date(2026, 9, 22, 23, 59), now, "ja"), "10/22 23:59");
  assert.equal(ui$.formatWhen(new Date(2025, 11, 31, 23, 59), now, "ja"), "2025/12/31 23:59");
  assert.match(ui$.fullWhen(new Date(2026, 9, 23, 0, 1, 7), "en"), /00:01:07/);
  ui$.setLanguage("ja");
  assert.equal(ui$.sessionWhen("2026-10-19 13-00-00", now), "10/19 13:00");
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
  const rows = ui$.hotkeyRows([{ action: "mark", keys: "Alt+F9", default: "Ctrl+Alt+M" }]);
  assert.deepEqual(rows.map((r) => [r.label, r.changed]), [["Mark this moment", true]]);
  for (const r of rows) assert.ok(ui$.JA[r.label], r.label);
});

test("a global shortcut another app holds shows its row in a warning state", () => {
  const hotkeys = [{ action: "mark", keys: "Ctrl+Alt+M", default: "Ctrl+Alt+M", taken: true }];
  assert.deepEqual(ui$.hotkeyRows(hotkeys).map((r) => r.warning), ["In use by another app"]);
  assert.ok(ui$.JA["In use by another app"]);
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
  const flow = { relay: "relay.example.net", download: null, update: { host: "github.com" }, report: { host: "reports.example.org" }, time: { host: "time.cloudflare.com", ok: true }, api: { on: false } };
  const names = { a: "Aki", b: "Ren" };
  const direct = ui$.flowRows({ names, peers: [peerOf("a")] }, flow);
  assert.deepEqual(plain(direct[0]), { key: "Members", level: "ok", text: "All direct" });
  assert.deepEqual(plain(direct.map((r) => r.key)), ["Members", "Relay server", "Update check", "Feedback", "Time check", "Downloads"]);
  assert.deepEqual(plain(direct[4]), { key: "Time check", level: "info", text: "time.cloudflare.com · at start and every hour" });
  assert.deepEqual(plain(ui$.flowRows({ names, peers: [] }, { ...flow, time: { host: "time.cloudflare.com", ok: false } })[4]), { key: "Time check", level: "warn", text: "time.cloudflare.com · Unreachable" });
  assert.equal(direct[1].text, "Default · relay.example.net · Sees only encrypted traffic");
  assert.equal(direct[3].text, "reports.example.org · only when you send it");
  assert.equal(ui$.flowRows({ names, peers: [] }, { ...flow, report: { host: null } })[3].text, "Not in this build");
  assert.equal(direct[2].text, "github.com · at start and every 4 h");
  const relayed = ui$.flowRows({ names, peers: [peerOf("a"), peerOf("b", { path: "relay", relay: "relay2.example.net" })] }, { ...flow, download: "files.example.org", api: { on: true } });
  assert.deepEqual(plain(relayed[0]), { key: "Members", level: "warn", text: "Through a relay server: Ren" });
  assert.ok(relayed[1].text.startsWith("Default · relay.example.net, relay2.example.net"));
  assert.deepEqual(plain(relayed[5]), { key: "Downloads", level: "warn", text: "Downloading Video tools from files.example.org" });
  assert.deepEqual(plain(relayed[6]), { key: "Local API", level: "info", text: "This PC only" });
  assert.equal(ui$.flowRows({ names, peers: [] }, null)[0].text, "No one else is online");
});

test("feedback shows what is sent, needs a comment or the diagnostics, and an error opens it with the message", () => {
  ui$.setLanguage("en");
  assert.equal(ui$.feedbackPreview("idea", "More maps", null), "Kind: Request\n\nMore maps");
  assert.equal(ui$.feedbackPreview("bug", "  ", "report text"), "Kind: Problem\n\n(no comment)\n\n---\nreport text");
  assert.equal(ui$.feedbackPreview("toString", "x", null), "Kind: Problem\n\nx");
  assert.equal(ui$.feedbackPreview("bug", "x", "report", 1500000), "Kind: Problem\n\nx\n\n+ The log, masked the same way and compressed (1.5 MB)\n\n---\nreport");
  assert.equal(ui$.feedbackPreview("bug", "x", null, 1500000), "Kind: Problem\n\nx");
  assert.equal(ui$.feedbackPreview("comment", "x".repeat(ui$.LIMITS.feedback + 5), null).length, "Kind: Comment\n\n".length + ui$.LIMITS.feedback);
  assert.deepEqual([ui$.feedbackReady(" ", false), ui$.feedbackReady(" ", true), ui$.feedbackReady("hi", false)], [false, true, true]);
  assert.equal(ui$.errorReport(new Error("Export failed")), "Error: Export failed");
  assert.equal(ui$.errorReport("Couldn't send. Check your connection."), "Error: Couldn't send. Check your connection.");
  const last = { text: "Export failed: disk full" };
  assert.deepEqual([ui$.shownError(last, true), ui$.shownError(last, false), ui$.shownError(null, true)], ["Error: Export failed: disk full", "", ""]);
  const html = read("index.html");
  assert.deepEqual([...html.match(/id="fbkind"[^]*?<\/div>/)[0].matchAll(/data-v="(\w+)"/g)].map((m) => m[1]), ["bug", "idea", "comment"]);
  assert.match(html, /id="fbdiag" type="checkbox" checked/);
  assert.match(html, /id="fbimage" type="checkbox">/);
});

test("the new Diagnostics texts have Japanese translations", () => {
  for (const word of ["Members", "Relay server", "Update check", "Feedback", "Time check", "Downloads", "Local API"]) assert.ok(word in ui$.JA, word);
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
  assert.equal(ui$.osdSeek(-15), "−15 s");
  assert.equal(ui$.osdSeek(30), "+30 s");
  assert.equal(ui$.osdVolume(0.7, false), "Volume 70%");
  assert.equal(ui$.osdVolume(0.7, true), "Muted");
  assert.equal(ui$.osdVolume(0, false), "Muted");
  assert.equal(ui$.osdSpeed(1.5), "1.5×");
  ui$.setLanguage("ja");
  assert.equal(ui$.osdSeek(-5), "−5秒");
  assert.equal(ui$.osdVolume(0.35, false), "音量 35%");
  ui$.setLanguage("en");
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
  for (const [en, ja] of [["Start recording", "録画開始"], ["Stop recording", "録画終了"], ["Stop debrief", "反省会の録音を終了"], ["Recordings", "録画一覧"], ["Recording", "録画中"], ["Standby", "待機中"], ["Jump to latest", "最新に戻る"]]) assert.equal(ui$.JA[en], ja);
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
  for (const m of read("pages.js").matchAll(/^ {2}\["((?:[^"\\]|\\.)*)", \[/gm)) phrases.add(text(m[1]));
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

test("the side panel's Analysis and Debrief rows are one roomy line each under short headers", () => {
  const css = read("style.css");
  const rule = (sel) => css.match(new RegExp(`^${sel.replace(/[.]/g, "\\.")} \\{([^}]*)\\}`, "m"))[1];
  assert.ok(parseInt(rule(".prow").match(/min-height: (\d+)px/)[1], 10) >= 40);
  assert.match(rule(".prow .nm"), /white-space: nowrap/);
  assert.match(rule(".prow .st"), /white-space: nowrap/);
  assert.match(rule(".plow .phd"), /padding: var\(--s3\)/);
  const app = read("app.js");
  const render = app.slice(app.indexOf("function renderAnalysis("), app.indexOf("function debriefMenu("));
  assert.doesNotMatch(render, /ptx|meta|pill state/);
  ui$.setLanguage("ja");
  assert.equal(ui$.t("Analysis"), "解析");
  assert.equal(ui$.t("Debriefs"), "反省");
  ui$.setLanguage("en");
});

test("Settings has a switch to notify when someone joins, kept by the app", () => {
  assert.match(read("index.html"), /data-t="Notify when someone joins"[^\n]*<input id="notifyjoins" type="checkbox" class="switch" role="switch">/);
  assert.match(read("pages.js"), /\$\("notifyjoins"\)\.checked = st\.notify_joins/);
  assert.match(read("pages.js"), /run\("set_notify_joins", \{ on: \$\("notifyjoins"\)\.checked \}\)/);
});

test("the folder export is a menu action of the page", () => {
  assert.match(read("pages.js"), /"export-folder":/);
});

test("the export dialog shows the full path it saves to, offers the options only for a recording with a picture, and waits for the path", () => {
  const video = { players: [{ owner: "a", parts: [{ v: true }] }] };
  const voice = { players: [{ owner: "a", parts: [{ v: false }] }] };
  const folder = "C:\\Users\\Ann\\Videos\\EZ Hanseikai\\Team debrief 2026-10-23 14-05 13 min (2)";
  assert.deepEqual(plain(ui$.exportView(voice, folder)), { options: false, video: false, path: folder, ready: true });
  assert.deepEqual(plain(ui$.exportView(video, null)), { options: true, video: false, path: "", ready: false });
  assert.deepEqual(plain(ui$.exportView(voice, folder, true)), { options: true, video: true, path: folder, ready: true });
});

test("the export options fall back to the defaults (burn-ins on) and to everyone for a member not in the recording, send no member for everyone, and the debrief video sends its kind and burn-ins without a range", () => {
  const defaults = { range: "whole", quality: "standard", member: "", notes: true, marks: true, matches: true };
  assert.deepEqual(plain(ui$.exportChoice(null, ["a"])), defaults);
  assert.deepEqual(plain(ui$.exportChoice({ range: "x", quality: 3, member: "gone", notes: "no" }, ["a"])), defaults);
  assert.deepEqual(plain(ui$.exportChoice({ range: "matches", quality: "light", member: "a", marks: false }, ["a", "b"])), { range: "matches", quality: "light", member: "a", notes: true, marks: false, matches: true });
  assert.deepEqual(plain(ui$.exportArgs({ ...defaults, member: "a", notes: false }, true)), { kind: "debrief_video", quality: "standard", member: "a", notes: false, marks: true, matches: true });
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
  assert.equal(items[0].heading, "Copy a prompt for an external agent");
  assert.ok(!items[0].run);
  assert.deepEqual(plain(items.slice(1).map((i) => i.label)), ["Write notes from the voices", "Summarize a recording into the game plan", "Find the mistakes in a match"]);
  assert.ok(items.length >= 4 && items.length <= 5);
  items[1].run();
  assert.equal(calls.length, 1);
  assert.equal(calls[0].title, items[1].label);
  assert.ok(calls[0].text.includes(info.token));
  const preview = ui$.aiPreview(calls[0].text, calls[0].token);
  assert.ok(!preview.includes(info.token) && preview.includes("••••••••") && preview.includes("llms.txt"));
  const off = ui$.aiMenuItems({ ...info, on: false, token: "" }, room, null, actions);
  assert.deepEqual(plain(off.map((i) => i.heading || i.label)), ["Copy a prompt for an external agent", "Turn on the local API"]);
  off[1].run();
  assert.equal(calls[1], "enable");
  assert.ok("Copy prompt" in ui$.JA);
  assert.ok("Copy a prompt for an external agent" in ui$.JA);
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
  assert.match(ui$.sessionName("2026-10-19 13-00-00", new Date(2026, 9, 19)), /^Oct 19 13:00$/);
  ui$.setTitles({});
});

test("a member row says one short state", () => {
  const at = new Date(2026, 9, 19, 12, 0);
  ui$.setLanguage("en");
  const say = (m) => ui$.memberStatus({ connected: true, sharing: false, ...m }, at).text;
  assert.equal(say({}), "Online · no screen");
  assert.equal(say({ sharing: true }), "Standby");
  assert.equal(say({ sharing: true, relayed: true }), "No picture via relay");
  assert.equal(say({ relayed: true }), "Online · no screen");
  assert.equal(say({ connected: false }), "Offline");
  assert.equal(say({ connected: false, seen_us: new Date(2026, 9, 19, 9, 5).getTime() * 1000 }), "Offline · last seen Oct 19 09:05");
  ui$.setLanguage("ja");
  assert.equal(say({}), "オンライン · 画面なし");
  assert.equal(say({ sharing: true }), "待機中");
  assert.equal(say({ sharing: true, relayed: true }), "中継のため映像なし");
  assert.match(say({ connected: false }), /^オフライン/);
  ui$.setLanguage("en");
});

test("a last seen time has the full local date and time, IDs show one length, and the window is named after the room and recording", () => {
  const us = new Date(2026, 9, 19, 9, 5).getTime() * 1000;
  ui$.setLanguage("en");
  assert.match(ui$.memberStatus({ connected: false, seen_us: us }, new Date(2026, 9, 19, 12, 0)).title, /October 19, 2026.*09:05/);
  assert.equal(ui$.longId("aaaabbbbccccdddd"), "aaaa·bbbb·cccc");
  assert.equal(ui$.windowTitle("EZ Hanseikai", "", ""), "EZ Hanseikai");
  assert.equal(ui$.windowTitle("EZ Hanseikai", "Night Squad", ""), "Night Squad");
  assert.equal(ui$.windowTitle("EZ Hanseikai", "Night Squad", "Yesterday 23:07"), "Night Squad - Yesterday 23:07");
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
  constructor(tag) { this.tag = tag; this.children = []; this.dataset = {}; this.style = { setProperty(k, v) { this[k] = v; } }; this.attributes = {}; this.classList = { toggle() {}, add() {}, remove() {}, contains: () => false }; }
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

test("a picked picture the page cannot decode goes through ffmpeg, a huge one is refused first, and the icon steps its quality down until it fits", async () => {
  const sent = [];
  const decodes = (x) => (x.type === "image/png" ? Promise.resolve({ width: 256, height: 256, from: "ffmpeg" }) : x.name.endsWith(".png") ? Promise.resolve({ width: 4032, height: 3024, from: "page" }) : Promise.reject(new Error("InvalidStateError")));
  const window = { __TAURI__: { core: { invoke: async (name, bytes) => { sent.push([name, bytes.length]); if (name !== "convert_picture") throw "No such command."; return new Uint8Array([1, 2, 3]).buffer; } } } };
  const { ctx } = page(["util.js", "faces.js"], { window, createImageBitmap: decodes, Blob, Uint8Array });
  const run = (code) => vm.runInContext(code, ctx);
  const file = (name, size) => ({ name, size, type: "", arrayBuffer: async () => new ArrayBuffer(size) });
  assert.equal((await run("openImage")(file("photo.png", 10))).from, "page");
  assert.deepEqual(sent, []);
  assert.equal((await run("openImage")(file("IMG_0001.HEIC", 20))).from, "ffmpeg");
  assert.deepEqual(sent, [["convert_picture", 20]]);
  await assert.rejects(run("openImage")(file("huge.heic", run("IMAGE_FILE_MAX") + 1)), (e) => e === "That picture file is too large.");
  assert.equal(sent.length, 1);
  const max = run("IMAGE_MAX");
  const tried = [];
  const sized = (sizes) => (q) => { tried.push(q); return "x".repeat(sizes[tried.length - 1]); };
  assert.equal(run("fitImage")(sized([max])).length, max);
  tried.length = 0;
  assert.equal(run("fitImage")(sized([max + 1, max + 1, 100])).length, 100);
  assert.deepEqual(tried, [...run("IMAGE_QUALITIES")].slice(0, 3));
  tried.length = 0;
  assert.throws(() => run("fitImage")(sized(run("IMAGE_QUALITIES").map(() => max + 1))), (e) => e === "That image is too large.");
  const rust = fs.readFileSync(path.join(ui, "..", "src-tauri", "src", "room.rs"), "utf8");
  assert.equal(Number(rust.match(/pub const IMAGE_MAX: usize = (\d+);/)[1]), max);
  assert.match(read("index.html"), /id="imagefile" type="file" accept="image\/\*,\.heic,\.heif,\.avif"/);
});

test("an avatar shows the image, a built-in shape or the initial; an unknown shape or none shows the plain person; a room without an icon shows a preset picked from its id, and a room with no id its initial", () => {
  const image = "data:image/webp;base64,AAAA";
  const state = { id: "me", name: "Aki", names: { ren: "Ren", koya: "Koya", mia: "Mia" }, colors: { me: 1, ren: 2, koya: 3, mia: 4 }, capture: { mix: { mic: { muted: false } } }, avatars: { me: "tiger-head", ren: image, koya: "gone", mia: "data:image/svg+xml;base64,AAAA" } };
  const { ctx } = page(["util.js", "faces.js", "live.js"], { speaking: new Set(), st: state, nameOf: (id) => id, colorIndex: (id) => state.colors[id] || 0 });
  const draw = (code) => vm.runInContext(code, ctx);
  const tiger = draw(`avatar("me", "sm")`);
  assert.deepEqual([tiger.find("path").length, tiger.textContent, tiger.style.background], [1, "", "var(--member-sky)"]);
  assert.equal(draw(`avatar("me", "sm")`).find("path")[0].attributes.d, draw(`FACES["tiger-head"]`));
  assert.equal(draw(`avatar("ren")`).find("img")[0].src, image);
  for (const id of ["koya", "mia", "nobody"]) {
    const a = draw(`avatar("${id}")`);
    assert.deepEqual([a.textContent, a.find("img").length, a.find("path")[0].attributes.d, a.className.includes("person")], ["", 0, draw(`PERSON_GLYPH`), true]);
  }
  assert.equal(draw(`avatar("me", "sm")`).className.includes("person"), false);
  const room = draw(`roomIcon({ name: "Practice", icon: "" }, "sm")`);
  assert.deepEqual([room.textContent, room.className], ["P", "av sm room"]);
  const preset = draw(`roomIcon({ id: "ab12", name: "Practice", icon: "" }, "sm")`);
  assert.deepEqual([preset.textContent, preset.find("path").length, preset.className], ["", 1, "av sm room"]);
  const shape = (icon) => draw(`roomIcon({ id: "ab12", name: "Practice", icon: "${icon}" }, "sm")`).find("path")[0]?.attributes.d;
  assert.deepEqual([draw(`ROOM_PRESETS.length`), draw(`ROOM_PRESETS.some((n) => FACES[n])`)], [24, false]);
  assert.deepEqual([draw(`PRESETS.length`), draw(`PRESETS.some((n) => ROOM_FACES[n])`)], [24, false]);
  assert.equal(shape("lightning-helix"), draw(`ROOM_FACES["lightning-helix"]`));
  assert.equal(shape("cat"), draw(`FACES["lynx-head"]`));
  const fallback = draw(`ROOM_FACES[defaultPreset("ab12")]`);
  for (const icon of ["unknown", "", "constructor", "__proto__", "toString", "a b'\\\"<>&\\u00e9\\u3042", "data:image/svg+xml;base64,AAAA"]) assert.equal(shape(icon), fallback);
  const odd = (icon) => draw(`roomIcon({ id: "ab12", name: "Practice", icon: ${JSON.stringify(icon)} }, "sm")`);
  for (const icon of [null, undefined, 0, "constructor"]) assert.deepEqual([odd(icon).textContent, odd(icon).find("path")[0].attributes.d], ["", fallback]);
  assert.equal(draw(`roomIcon({ name: "Practice", icon: "unknown" }, "sm")`).textContent, "P");
  Object.assign(state.names, { n1: "Ann", n2: "Bo", n3: "Cy", n4: "Di", n5: "Ed", n6: "Flo", n7: "Gus" });
  Object.assign(state.avatars, { n1: "", n2: "constructor", n3: "__proto__", n4: "toString", n5: "a b'\"<>&\u3042", n6: null });
  for (const id of ["n1", "n2", "n3", "n4", "n5", "n6", "n7"]) {
    const face = draw(`avatar("${id}", "sm")`);
    assert.deepEqual([face.find("path").length, face.find("img").length, face.textContent], [1, 0, ""]);
  }
  assert.ok(draw(`ROOM_PRESETS`).includes(draw(`defaultPreset("ab12")`)));
  const legacyCrests = "flame bolt crown skull swords shield crosshair rocket wings trophy gem comet peak mask star chevrons".split(" ");
  const legacyAnimals = "cat fox frog panda owl bunny octopus bear penguin koala wolf tiger raccoon shark whale crab".split(" ");
  const old = { bolt: "lightning-helix", swords: "crossed-swords", shield: "checked-shield", wings: "angel-wings", chevrons: "crown", gem: "eclipse", skull: "skull-crossed-bones", star: "sun", comet: "fire-ray" };
  for (const name of legacyCrests) {
    const d = shape(name);
    assert.ok(d, name);
    assert.equal(d, name in old ? draw(`ROOM_FACES["${old[name]}"]`) : draw(`ROOM_FACES[${JSON.stringify(name)}] || LEGACY_ROOM_FACES[${JSON.stringify(name)}]`));
  }
  const animal = { cat: "lynx-head", octopus: "spider-alt", whale: "shark-jaws", fox: "fox-head", owl: "owl", bear: "bear-head", wolf: "wolf-head", tiger: "tiger-head", shark: "shark-jaws" };
  for (const name of legacyAnimals) {
    const face = draw(`picture("", "red", ${JSON.stringify(name)}, "sm")`);
    assert.equal(face.find("path")[0].attributes.d, draw(`FACES[${JSON.stringify(animal[name] || name)}] || LEGACY_FACES[${JSON.stringify(name)}]`), name);
    assert.equal(face.className.includes("person"), false);
  }
  for (const name of draw(`[...PRESETS, ...ROOM_PRESETS]`)) assert.ok(draw(`knownIcon(${JSON.stringify(name)})`), name);
  assert.equal(draw(`knownIcon("nonsense")`), false);
  assert.equal(draw(`roomIcon({ id: "ab12", name: "Other", icon: "" }, "sm")`).style.background, preset.style.background);
  assert.equal(draw(`roomIcon({ name: "Practice", icon: "${image}" }, "sm")`).find("img").length, 1);
  assert.deepEqual(plain(draw(`look("ren")`)), [2, image.length, image.slice(-16)]);
  const chip = draw(`nameChip(roomIcon({ name: "Practice", icon: "" }, "sm"), "Practice", "x")`);
  assert.deepEqual([chip.tag, chip.className, chip.children[0].className, chip.find("b")[0].textContent, chip.textContent], ["span", "chip", "av sm room", "Practice", "PPracticex"]);
  const two = draw(`stack("Aki", "aaaa", true)`);
  assert.deepEqual([two.tag, two.className, two.find("b")[0].textContent, two.find("small")[0].textContent, draw(`stack("Aki", "")`).children.length], ["span", "t lg", "Aki", "aaaa", 1]);
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
  const { ctx } = page(["util.js", "icons.js", "i18n.js", "layout.js", "events.js", "markdown.js"], { st: { id: "me", name: "Mia", names: { a: "Jo Smith", b: "Bo", c: "Bo" } }, openLink() {}, pickChip: (...a) => picks.push(a), avatar: (id, size) => Object.assign(new Fake("span"), { className: `av ${size}`, id }) });
  const faces = (b) => b.find("span").map((a) => [a.className, a.id]);
  const chips = (code) => vm.runInContext(code, ctx).find("button").filter((b) => b.className === "tag");
  const card = chips(`cardDom("Plan @jo smith\\n- push #Rotate with @Mia", mdHooks())`);
  assert.deepEqual(card.map((b) => [b.dataset.kind, b.dataset.value, b.textContent]), [["mention", "Jo Smith", "@Jo Smith"], ["tag", "Rotate", "#Rotate"], ["mention", "Mia", "@Mia"]]);
  for (const b of card) b.onclick({ stopPropagation() {} });
  assert.deepEqual(plain(picks), [["mention", "Jo Smith", ["a"]], ["tag", "Rotate", []], ["mention", "Mia", ["me"]]]);
  assert.deepEqual(card.map(faces), [[["av xs", "a"]], [], [["av xs", "me"]]]);
  const note = chips(`el("p", {}, ...inline("go @Jo #push", [{ id: "a", name: "Jo" }]))`);
  assert.deepEqual(note.map((b) => [b.dataset.kind, b.dataset.value]), [["mention", "Jo Smith"], ["tag", "push"]]);
  assert.deepEqual(chips(`cardDom("@Bo @Gone", mdHooks({ names: ["Bo", "Gone"] }))`).map(faces), [[], []]);
  assert.deepEqual(chips(`el("p", {}, ...inline("@Gone", [{ id: "z", name: "Gone" }]))`).map(faces), [[]]);
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

test("an extension row says one thing: no runner, what failed, who it waits for, or what is left; an Analysis row waiting on the runner's own PC says it waits for the recording to end or for the video tools", () => {
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
  assert.equal(ui$.analysisReason({ state: "left", reason: "recording" }, name), "Waiting for the recording to end");
  assert.equal(ui$.analysisReason({ state: "left", reason: "ffmpeg" }, name), "Waiting for the video tools");
  assert.equal(ui$.analysisReason({ state: "left", reason: "no_runner" }, name), "No runner");
  assert.equal(ui$.analysisReason({ state: "failed" }, name), "Failed");
  assert.equal(ui$.analysisReason({ state: "failed", reason: "missing" }, name), "File missing");
  assert.equal(ui$.analysisReason({ state: "running", progress: 0.424 }, name), "Analyzing 42%");
  assert.equal(ui$.analysisReason({ state: "running", progress: null }, name), "Analyzing");
  assert.equal(ui$.analysisState({ state: "running", progress: 0.42 }, "a", name), "Analyzing 42% · Ren's PC");
  assert.equal(ui$.analysisState({ state: "left", reason: "receiving" }, "a", name), "Receiving");
  assert.equal(ui$.analysisState({ state: "done" }, "a", name), "Done");
  assert.equal(ui$.runningMark({ analysis: [{ state: "done" }, { state: "running", progress: 0.5 }] }), "Analyzing 50%");
  assert.equal(ui$.runningMark({ analysis: [{ state: "running" }] }), "Analyzing");
  for (const state of ["left", "done", "failed", "idle"]) assert.equal(ui$.runningMark({ analysis: [{ state }] }), null);
  assert.equal(ui$.runningMark({}), null);
  assert.equal(ui$.analysisState({ state: "idle" }, "a", name), "Not analyzed");
  const pill = (a) => plain(ui$.analysisPill(a));
  assert.deepEqual(pill({ state: "done" }), { text: "Done", tone: "ok" });
  assert.deepEqual(pill({ state: "failed" }), { text: "Failed", tone: "warn" });
  assert.deepEqual(pill({ state: "running", progress: 0.5 }), { text: "Analyzing 50%", tone: "run" });
  assert.deepEqual(pill({ state: "left" }), { text: "Waiting", tone: "gray" });
  assert.deepEqual(pill({ state: "idle" }), { text: "Not analyzed", tone: "gray" });
  const ids = (a) => plain(ui$.analysisActions(a)).map((x) => x.id + (x.disabled ? "!" : ""));
  assert.deepEqual(ids({ state: "done" }), ["details", "again"]);
  assert.deepEqual(ids({ state: "idle" }), ["details", "again"]);
  assert.deepEqual(ids({ state: "running" }), ["details"]);
  assert.deepEqual(ids({ state: "left" }), ["details", "again!"]);
  assert.deepEqual(ids({ state: "failed", error: "boom" }), ["details", "again", "copy"]);
  assert.deepEqual(ids({ state: "failed", reason: "missing" }), ["details", "again"]);
  const rows = ui$.analysisDetails({ state: "failed", error: "boom", at_us: 5 }, { name: "Apex match detection", version: "1", who: name, runner: "a", recording: "Rec", finished: () => "Oct 23 00:01", results: 3 });
  assert.deepEqual(plain(rows).map((r) => r.key), ["name", "state", "reason", "runner", "recording", "finished", "results", "error"]);
  assert.equal(ui$.analysisDetailText(rows), "Extension: Apex match detection 1\nState: Failed\nReason: Failed\nRunner: Ren\nRecording: Rec\nFinished: Oct 23 00:01\nResults: 3\nError: boom");
  ui$.setLanguage("ja");
  assert.equal(line({ left: 3 }), "残り3件 · Ren のPC");
  assert.equal(ui$.analysisReason({ state: "left", reason: "recording" }, name), "録画が終わるのを待っています");
  assert.equal(ui$.analysisReason({ state: "left", reason: "ffmpeg" }, name), "動画ツールを待っています");
  ui$.setLanguage("en");
});

test("one UI command registry describes its commands with their current arguments, runs a command by name and builds the state from its readers", () => {
  const ran = [];
  let sets = ["a"];
  const registry = {
    seek: { about: "Seek", args: { seconds: "number" }, run: ({ seconds }) => ran.push(["seek", seconds]), reads: { position_s: () => 4 } },
    open_recording: { about: "Open", args: { set: () => sets }, run: ({ set }) => ran.push(["open", set]), reads: { set: () => "a" } },
    pause: { about: "Pause", run: (args) => ran.push(["pause", plain(args)]) },
  };
  assert.deepEqual(plain(ui$.uiCommands(registry)), { seek: { about: "Seek", args: { seconds: "number" } }, open_recording: { about: "Open", args: { set: ["a"] } }, pause: { about: "Pause", args: {} } });
  sets = ["a", "b"];
  assert.deepEqual(plain(ui$.uiCommands(registry).open_recording.args.set), ["a", "b"]);
  assert.deepEqual([ui$.uiRun(registry, "seek", { seconds: 12 }), ui$.uiRun(registry, "open_recording", { set: "b" }), ui$.uiRun(registry, "pause"), ui$.uiRun(registry, "jump", {}), ui$.uiRun(registry, "toString", {})], [true, true, true, false, false]);
  assert.deepEqual(ran, [["seek", 12], ["open", "b"], ["pause", {}]]);
  assert.deepEqual(plain(ui$.uiState(registry, { tiles: () => [] })), { position_s: 4, set: "a", tiles: [] });
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
  assert.deepEqual([LIMITS.name, LIMITS.card, LIMITS.room, LIMITS.note, LIMITS.board, LIMITS.code, LIMITS.relay, LIMITS.feedback], [rustConst("main.rs", "NAME_MAX"), rustConst("main.rs", "NAME_MAX"), rustConst("main.rs", "ROOM_NAME_MAX"), rustConst("main.rs", "NOTE_MAX"), rustConst("main.rs", "BOARD_MAX"), rustConst("room.rs", "CODE_MAX"), rustConst("relay.rs", "URL_MAX"), rustConst("report.rs", "COMMENT_MAX")]);
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

test("an inline editor cancels on Esc and saves on Enter (Ctrl+Enter in a multi-line one), and ignores both while an IME is composing", () => {
  const key = (e, multiline) => ui$.editKey(e, multiline);
  assert.equal(key({ key: "Escape" }), "cancel");
  assert.equal(key({ key: "Escape" }, true), "cancel");
  assert.equal(key({ key: "Enter" }), "save");
  assert.equal(key({ key: "Enter" }, true), null);
  assert.equal(key({ key: "Enter", ctrlKey: true }, true), "save");
  assert.equal(key({ key: "a" }), null);
  assert.equal(key({ key: "Enter", isComposing: true }), null);
  assert.equal(key({ key: "Escape", keyCode: 229 }), null);
});

test("arrow keys move one row in a list and stop at its ends, and Home and End go to the first and last", () => {
  const move = ui$.listMove;
  assert.equal(move("ArrowDown", 0, 3), 1);
  assert.equal(move("ArrowDown", 2, 3), 2);
  assert.equal(move("ArrowUp", 0, 3), 0);
  assert.equal(move("Home", 2, 3), 0);
  assert.equal(move("End", 0, 3), 2);
  assert.equal(move("Enter", 0, 3), null);
  assert.equal(move("ArrowDown", 0, 0), null);
});

test("Esc closes a menu or dialog first, then a popup, then leaves a text field, then full screen, then closes the Board window, then the panel", () => {
  const step = (o) => ui$.escapeStep({ menu: false, dialog: false, pop: false, typing: false, full: false, panel: false, ...o });
  assert.equal(step({ full: true, menu: true }), "none");
  assert.equal(step({ full: true, dialog: true }), "none");
  assert.equal(step({ full: true, pop: true, typing: true }), "pop");
  assert.equal(step({ full: true, typing: true }), "blur");
  assert.equal(step({ full: true, panel: true }), "full");
  assert.equal(step({ panel: true }), "panel");
  assert.equal(step({ window: true, full: true }), "full");
  assert.equal(step({ window: true, typing: true }), "blur");
  assert.equal(step({ window: true, dialog: true }), "none");
  assert.equal(step({ window: true }), "close");
  assert.equal(step({}), "none");
});

test("the drawing button cycles Pen, Laser, off on P, every state has a Japanese tooltip with the key, and the player keys have no duplicates", () => {
  assert.deepEqual([ui$.nextInkTool("pen"), ui$.nextInkTool("laser"), ui$.nextInkTool(null)], ["laser", null, "pen"]);
  const jaTitles = Object.values(plain(ui$.INK_TOOL_TITLE));
  assert.equal(jaTitles.length, 3);
  ui$.setLanguage("ja");
  for (const title of jaTitles) { assert.ok(title.includes("(P)")); assert.notEqual(ui$.t(title), title); }
  ui$.setLanguage("en");
  const body = read("app.js").match(/const PLAYER_KEYS = \{([\s\S]*?)\n\};/)[1];
  const keys = [...body.matchAll(/^  ("?[^\s":]+"?|" "):/gm)].map((m) => m[1].replaceAll('"', "").toLowerCase());
  assert.equal(new Set(keys).size, keys.length);
  assert.ok(keys.includes("p") && keys.includes("x") && keys.includes("a") && keys.includes("l"));
  assert.match(read("pages.js"), /\["P", "X"\]/);
  assert.match(read("index.html"), /id="tool"/);
});

test("a saved stroke shows near its moment when paused, and for two seconds with a short fade while playing", () => {
  assert.equal(ui$.strokeAlpha(10, 10.5, false, false), 1);
  assert.equal(ui$.strokeAlpha(10, 12, false, false), 0);
  assert.equal(ui$.strokeAlpha(10, 9.9, true, false), 0);
  assert.equal(ui$.strokeAlpha(10, 10, true, false), 1);
  assert.equal(ui$.strokeAlpha(10, 11.4, true, false), 1);
  const fading = ui$.strokeAlpha(10, 11.75, true, false);
  assert.ok(fading > 0 && fading < 1);
  assert.equal(ui$.strokeAlpha(10, 12, true, false), 0);
  assert.equal(ui$.strokeAlpha(10, 11.75, true, true), 1);
});


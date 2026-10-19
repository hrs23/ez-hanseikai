import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const ui = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "ui");
const read = (f) => fs.readFileSync(path.join(ui, f), "utf8");
const source = ["util.js", "i18n.js", "layout.js", "events.js", "review.js", "plugins.js", "ai.js"].map(read).join("\n");
const context = vm.createContext({ window: { __TAURI__: { core: {} } }, document: { querySelectorAll: () => [], documentElement: {} }, console, Intl, Date });
const ui$ = vm.runInContext(`${source}\n({ matchEnds, matchChapters, stepTarget, matchTargets, groupDrawings, dayLabel, keysOf, flowAt, flowBands, flowSteps, isReview, voiceOnly, deletePreview, oldSessions, soundKey, pluginProgress, isBusy, workerState, downloadStats, sessionNotice, aiPurposes, aiPrompt, aiMenuItems, apiUrl, rateOf, liveEdge, netState, summaryLines, leftText, JA, t, sessionName, setTitles: (x) => { titles = x; }, setLanguage: (l) => { language = l; } })`, context);
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

const S = 1e6;
const party = (t, set, wall, playing, by = "a") => ({ t_us: t, by, shared: { set, wall_us: wall, at_us: t, playing, rate: 1, mode: "grid", order: [] } });
const flow = [party(100 * S, "s1", 1000 * S, true), party(130 * S, "s1", 1030 * S, false, "b"), party(160 * S, "s1", 1200 * S, true), party(200 * S, "s2", 50 * S, true, "b")];

test("a review replays what the Watch Party watched at each moment", () => {
  assert.deepEqual(plain(ui$.flowAt(flow, 90 * S)), { set: "s1", playing: false, wall_us: 1000 * S, rate: 1, focus: null, mode: "grid", slots: 0, order: [], by: "a", at: 100 * S });
  assert.equal(ui$.flowAt(flow, 110 * S).wall_us, 1010 * S);
  const paused = ui$.flowAt(flow, 150 * S);
  assert.equal(paused.playing, false);
  assert.equal(paused.wall_us, 1030 * S);
  assert.equal(ui$.flowAt(flow, 170 * S).wall_us, 1210 * S);
  assert.equal(ui$.flowAt(flow, 260 * S).set, "s2");
  assert.equal(ui$.flowAt([], 1), null);
});

test("the bands under a review's seekbar show which session was watched", () => {
  assert.deepEqual(plain(ui$.flowBands(flow, 90 * S, 300 * S)), [{ set: "s1", from: 90 * S, to: 200 * S }, { set: "s2", from: 200 * S, to: 300 * S }]);
  assert.deepEqual(plain(ui$.flowBands(flow, 140 * S, 180 * S)), [{ set: "s1", from: 140 * S, to: 180 * S }]);
});

test("what was shown lists openings, plays, pauses and jumps", () => {
  assert.deepEqual(plain(ui$.flowSteps(flow).map((s) => [s.kind, s.by])), [["open", "a"], ["pause", "b"], ["play", "a"], ["open", "b"]]);
  const seek = [party(0, "s1", 0, false), party(10 * S, "s1", 1 * S, false), party(20 * S, "s1", 90 * S, false)];
  assert.deepEqual(plain(ui$.flowSteps(seek).map((s) => s.kind)), ["open", "seek"]);
});

test("a review session has voices only and Watch Party records", () => {
  const voice = { players: [{ parts: [{ m: {} }] }], party: [flow[0]] };
  assert.equal(ui$.isReview(voice), true);
  assert.equal(ui$.isReview({ ...voice, party: [] }), false);
  assert.equal(ui$.isReview({ players: [{ parts: [{ v: {}, m: {} }] }], party: [flow[0]] }), false);
  assert.equal(ui$.voiceOnly({ players: [] }), false);
});

test("the delete preview lists every part and warns when nobody else has one", () => {
  const set = { id: "s1", progress: { have: 30 }, players: [{ owner: "me", have: 10, verified_by: ["me", "b"] }, { owner: "b", have: 20, verified_by: ["me"] }, { owner: "c", have: 0, verified_by: [] }] };
  assert.deepEqual(plain(ui$.deletePreview([set], "me")), [{ id: "s1", bytes: 30, players: [{ owner: "me", bytes: 10, alone: false }, { owner: "b", bytes: 20, alone: true }] }]);
  const sets = ["s5", "s4", "s3", "s2", "s1"].map((id) => ({ id, live: false, progress: { have: id === "s2" ? 0 : 1 } }));
  assert.deepEqual([...ui$.oldSessions(sets, 2)], ["s3", "s1"]);
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
  assert.deepEqual(levels, { Connections: "warn", Capture: "info", Microphone: "info", Transfers: "warn", Storage: "warn", Versions: "warn", Plugins: "fail" });
});

test("every fixed text has a Japanese translation", () => {
  const used = new Set();
  for (const f of ["app.js", "side.js", "pages.js", "review.js", "events.js", "util.js", "draw.js", "ai.js"]) {
    const s = read(f);
    for (const m of s.matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"/g)) used.add(JSON.parse(`"${m[1]}"`));
    for (const m of s.matchAll(/\bt\([^)]*\?\s*"((?:[^"\\]|\\.)*)"\s*:\s*"((?:[^"\\]|\\.)*)"/g)) { used.add(JSON.parse(`"${m[1]}"`)); used.add(JSON.parse(`"${m[2]}"`)); }
  }
  for (const m of read("index.html").matchAll(/data-t(?:-title|-placeholder|-label)?="([^"]*)"/g)) used.add(m[1]);
  const missing = [...used].filter((k) => !(k in ui$.JA));
  assert.deepEqual(missing, []);
});

test("the all-players view plays the viewer's own sound and a focused player plays its own", () => {
  assert.equal(ui$.soundKey(0, ["a", "b", "c"], "b"), "b");
  assert.equal(ui$.soundKey(0, ["a", "b"], "z"), "a");
  assert.equal(ui$.soundKey(0, ["a"], "z"), "a");
  assert.equal(ui$.soundKey(3, ["a", "b", "c"], "a"), "c");
  assert.equal(ui$.soundKey(0, [], "a"), null);
});

test("a plugin's progress counts its finished sessions and its busy members", () => {
  const jobs = [{ plugin: "t", state: "done" }, { plugin: "t", state: "claimed", by: "b" }, { plugin: "t", state: "waiting" }, { plugin: "x", state: "done" }];
  assert.deepEqual(plain(ui$.pluginProgress(jobs, "t")), { done: 1, total: 3 });
  assert.deepEqual(plain(ui$.pluginProgress(jobs, "none")), { done: 0, total: 0 });
  assert.equal(ui$.isBusy(jobs, "t", "b"), true);
  assert.equal(ui$.isBusy(jobs, "t", "a"), false);
  assert.equal(ui$.workerState({ on: false, ready: true }, true), "off");
  assert.equal(ui$.workerState({ on: true, ready: false }, false), "missing");
  assert.equal(ui$.workerState({ on: true, ready: true }, true), "busy");
  assert.equal(ui$.workerState({ on: true, ready: null }, false), "ready");
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
  for (const f of ["app.js", "side.js", "pages.js", "review.js", "events.js", "util.js", "plugins.js", "ai.js", "index.html"]) {
    const shown = [...read(f).matchAll(/\bt\(\s*"((?:[^"\\]|\\.)*)"|data-t(?:-title|-placeholder|-label)?="([^"]*)"/g)].map((m) => m[1] || m[2]);
    assert.deepEqual(shown.filter((x) => /session/i.test(x)), [], f);
  }
  for (const [en, ja] of [["Start recording", "録画開始"], ["Stop recording", "録画終了"], ["Recordings", "録画一覧"], ["Recording", "録画中"], ["Standby", "待機中"], ["Jump to latest", "最新に戻る"]]) assert.equal(ui$.JA[en], ja);
});

test("the sidebar has a Room button and no Game plan entry", () => {
  const html = read("index.html");
  assert.match(html, /<button id="roomname"[^>]*>[^]*?data-t="Room"/);
  assert.doesNotMatch(html, /navboard|id="roomtitle"|id="boardpage"/);
  assert.match(html, /data-tab="board"/);
  assert.equal(ui$.JA["Room"], "ルーム");
});

test("every English phrase in the pages has a Japanese one", () => {
  const files = fs.readdirSync(ui).filter((f) => f.endsWith(".js") && f !== "i18n.js" && f !== "debug.js");
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
  const actions = { enable: () => calls.push("enable"), copy: (text) => calls.push(text) };
  const items = ui$.aiMenuItems(info, room, "2026-10-19 21-30-00", actions);
  assert.deepEqual(plain(items.map((i) => i.label)), ["Write notes from the transcript", "Summarize a recording into the game plan", "Find the mistakes in a match"]);
  assert.ok(items.length >= 3 && items.length <= 4);
  items[0].run();
  assert.equal(calls.length, 1);
  assert.match(calls[0], /Authorization: Bearer/);
  const off = ui$.aiMenuItems({ ...info, on: false, token: "" }, room, null, actions);
  assert.deepEqual(plain(off.map((i) => i.label)), ["Turn on the local API"]);
  off[0].run();
  assert.equal(calls[1], "enable");
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

test("Settings has one switch for the local API with its address and token buttons, and the room and player offer AI", () => {
  const html = read("index.html");
  assert.match(html, /<section data-pane="api">[^]*?<input id="apion" type="checkbox" class="switch" role="switch">[^]*?id="apiaddr"[^]*?id="apicopy"[^]*?id="apinew"[^]*?<\/section>/);
  assert.match(html, /<button data-pane="api">/);
  assert.match(html, /<button id="aibtn"[^>]*>[^]*?data-t="AI"/);
  assert.match(html, /<button id="aiplay"[^>]*>[^]*?data-t="AI"/);
  assert.equal((html.match(/id="apion"/g) || []).length, 1);
  for (const word of ["API", "Local API", "Copy token", "New token", "AI", "The port is in use"]) assert.ok(word in ui$.JA, word);
});

test("an imported recording is named by its title and the others by their time", () => {
  ui$.setLanguage("en");
  ui$.setTitles({ "2026-10-19 12-30-00": "Scrim vs Team B" });
  assert.equal(ui$.sessionName("2026-10-19 12-30-00"), "Scrim vs Team B");
  assert.match(ui$.sessionName("2026-10-19 13-00-00", new Date(2026, 9, 19)), /^Today 13:00$/);
  ui$.setTitles({});
});

test("the import and folder export dialogs, menu actions and phrases exist", () => {
  const html = read("index.html");
  for (const id of ["importdlg", "importfolder", "importfiles", "importtitle", "importviews", "importgo", "folderdlg", "folderaligned", "folderyes", "folderno"]) assert.match(html, new RegExp(`id="${id}"`), id);
  const pages = read("pages.js");
  for (const action of ["import-recording", "export-folder"]) assert.match(pages, new RegExp(`"${action}":`), action);
  assert.match(pages, /tauri:\/\/drag-drop/);
  for (const word of ["Imported", "Imported by {name}", "Import recording", "Choose folder…", "Choose files…", "Importing {n}%", "Import finished", "Export to folder…", "Start every video together"]) assert.ok(word in ui$.JA, word);
});

test("errors about a session folder read in Japanese too", () => {
  ui$.setLanguage("ja");
  assert.equal(ui$.t("Row 3 of the notes file: A note needs text."), "メモのファイルの 3 行目: メモには文章が必要です。");
  assert.equal(ui$.t("Row 3 of the notes file has no valid time."), "メモのファイルの 3 行目の時刻が正しくありません。");
  assert.equal(ui$.t("gone.mp4 is missing."), "gone.mp4 が見つかりません。");
  assert.equal(ui$.t("views must be a list."), "views はリストでなければなりません。");
  assert.equal(ui$.t("../x.mp4 must be a file inside the folder."), "../x.mp4 はフォルダの中のファイルでなければなりません。");
  assert.equal(ui$.t("Something nobody translated."), "Something nobody translated.");
  ui$.setLanguage("en");
  assert.equal(ui$.t("gone.mp4 is missing."), "gone.mp4 is missing.");
});

test("every fixed message the import and export code can send has a Japanese one", () => {
  const sources = ["session.rs", "import.rs", "bundle.rs"].map((f) => fs.readFileSync(path.join(ui, "..", "src-tauri", "src", f), "utf8").split("#[cfg(test)]")[0]);
  const used = new Set();
  for (const text of sources) for (const m of text.matchAll(/(?:Err\(|ok_or\(|ok_or_else\(\|\| |return Err\()"((?:[^"\\]|\\.)*)"/g)) used.add(JSON.parse(`"${m[1]}"`));
  for (const text of sources) for (const m of text.matchAll(/"((?:[A-Z][^"{}\\]*)\.)"(?:\.into\(\)|\.to_string\(\))/g)) used.add(JSON.parse(`"${m[1]}"`));
  const fixed = [...used].filter((x) => /^[A-Z]/.test(x) && x.includes(" ") && !x.includes("{"));
  assert.ok(fixed.length > 20, `found ${fixed.length} messages`);
  assert.deepEqual(fixed.filter((x) => !(x in ui$.JA)), []);
});

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const ui = path.join(path.dirname(fileURLToPath(import.meta.url)), "..", "ui");
const source = ["util.js", "i18n.js", "layout.js", "events.js"].map((f) => fs.readFileSync(path.join(ui, f), "utf8")).join("\n");
const context = vm.createContext({ window: { __TAURI__: { core: {} } }, document: {}, console, Intl, Date });
const ui$ = vm.runInContext(`${source}\n({ matchEnds, matchChapters, stepTarget, matchTargets, groupDrawings, dayLabel, keysOf, setLanguage: (l) => { language = l; } })`, context);

const moment = (m, extra = {}) => ({ role: "moment", m, plugin: { id: "apex-matches" }, tags: ["match"], ...extra });

test("match ends come only from plugin moments tagged match", () => {
  const list = [moment(300), moment(100), moment(200, { tags: ["other"] }), moment(250, { plugin: null }), { role: "chapter", m: 50, tags: ["match"], plugin: {} }];
  assert.deepEqual([...ui$.matchEnds(list)], [100, 300]);
});

test("matches run between consecutive ends, with the open one only while recording", () => {
  assert.deepEqual(JSON.parse(JSON.stringify(ui$.matchChapters([100, 300], 500, false))), [{ m: 0, end: 100, n: 1 }, { m: 100, end: 300, n: 2 }]);
  assert.deepEqual(JSON.parse(JSON.stringify(ui$.matchChapters([100, 300], 500, true))).at(-1), { m: 300, end: 500, n: 3 });
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
});

test("shortcuts need Ctrl or Alt and read like the native menu", () => {
  const key = (code, mods = {}) => ui$.keysOf({ code, key: "", ctrlKey: false, altKey: false, shiftKey: false, ...mods });
  assert.equal(key("KeyM", { ctrlKey: true, altKey: true }), "Ctrl+Alt+M");
  assert.equal(key("F9", { altKey: true, shiftKey: true }), "Alt+Shift+F9");
  assert.equal(key("Digit5", { ctrlKey: true }), "Ctrl+5");
  assert.equal(key("KeyM", { shiftKey: true }), null);
  assert.equal(key("ControlLeft", { ctrlKey: true }), null);
});

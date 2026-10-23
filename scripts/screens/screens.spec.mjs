// Renders the main screens with the Tauri core replaced by mock.js and compares each with its picture in baseline/.
// Update the pictures after an intended change: npx playwright test -c scripts/screens --update-snapshots
import { test, expect } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const ui = path.join(here, "..", "..", "ui");
const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".woff2": "font/woff2", ".svg": "image/svg+xml", ".png": "image/png" };
const thumb = (hue) => `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><defs><linearGradient id="g" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue} 60% 40%)"/><stop offset="1" stop-color="hsl(${hue + 120} 50% 25%)"/></linearGradient></defs><rect width="320" height="180" fill="url(#g)"/></svg>`;

test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(new Date("2026-10-23T21:00:00+09:00"));
  await page.addInitScript({ path: path.join(here, "mock.js") });
  await page.route("http://app.local/**", (route) => {
    const p = decodeURIComponent(new URL(route.request().url()).pathname);
    if (p.startsWith("/media/thumb/")) return route.fulfill({ body: thumb((p.length * 37) % 360), contentType: "image/svg+xml" });
    if (p.startsWith("/media/")) return route.fulfill({ status: 404 });
    const f = path.join(ui, p === "/" ? "index.html" : p);
    if (!f.startsWith(ui) || !fs.existsSync(f)) return route.fulfill({ status: 404 });
    return route.fulfill({ body: fs.readFileSync(f), contentType: types[path.extname(f)] || "application/octet-stream" });
  });
  await page.goto("http://app.local/index.html");
  await page.waitForFunction(() => typeof st !== "undefined" && st && !document.getElementById("app").hidden);
  await page.evaluate(() => document.fonts.ready);
});

const shot = async (page, name) => {
  await page.waitForTimeout(500);
  await expect(page).toHaveScreenshot(`${name}.png`);
};

const screens = {
  live: async () => {},
  player: (page) => page.evaluate(() => openSession(st.sets[0].id)),
  members: (page) => page.evaluate(() => showRoomTab("members")),
  board: (page) => page.evaluate(() => showRoomTab("board")),
  settings: (page) => page.evaluate(() => openSettings("general")),
  diagnostics: (page) => page.evaluate(() => openDiagnostics()),
  feedback: (page) => page.evaluate(() => openFeedback("bug")),
  lobby: (page) => page.evaluate(() => { const s = window.__mock.state; s.room = null; for (const r of s.rooms) r.active = false; return refresh(); }),
};

for (const [name, open] of Object.entries(screens)) {
  test(name, async ({ page }) => {
    await open(page);
    await shot(page, name);
  });
}

// Saves pictures of the main screens, with the Tauri core replaced by mock.js, to look at after a UI change.
// node scripts/screens/shoot.mjs [folder]   (default: screens/ in the repository, which git ignores)
import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const ui = path.join(here, "..", "..", "ui");
const out = path.resolve(process.argv[2] || path.join(here, "..", "..", "screens"));
const types = { ".html": "text/html", ".js": "text/javascript", ".css": "text/css", ".woff2": "font/woff2", ".svg": "image/svg+xml", ".png": "image/png" };
const thumb = (hue) => `<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><defs><linearGradient id="g" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue} 60% 40%)"/><stop offset="1" stop-color="hsl(${hue + 120} 50% 25%)"/></linearGradient></defs><rect width="320" height="180" fill="url(#g)"/></svg>`;

const screens = {
  live: () => {},
  player: () => openSession(st.sets[0].id),
  members: () => showRoomTab("members"),
  board: () => showRoomTab("board"),
  settings: () => openSettings("general"),
  shortcuts: () => openSettings("shortcuts"),
  recordings: () => openSettings("storage"),
  privacy: () => openSettings("privacy"),
  diagnostics: () => openDiagnostics(),
  profile: () => openProfile("b2kenji"),
  roomsettings: () => openRoomSettings(),
  feedback: () => openFeedback("bug"),
  lobby: () => { const s = window.__mock.state; s.room = null; for (const r of s.rooms) r.active = false; return refresh(); },
};

fs.mkdirSync(out, { recursive: true });
// CI uses the Chrome already on the runner (SCREENS_CHANNEL=chrome) instead of downloading Chromium.
const browser = await chromium.launch({ channel: process.env.SCREENS_CHANNEL || undefined });
for (const [width, height] of [[1280, 760], [960, 600]]) {
  for (const [name, open] of Object.entries(screens)) {
    const page = await browser.newPage({ viewport: { width, height }, locale: "ja-JP", timezoneId: "Asia/Tokyo", colorScheme: "dark" });
    page.on("pageerror", (e) => console.log(`${name}: ${e.message}`));
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
    await page.evaluate(open);
    await page.evaluate(() => document.fonts.ready);
    await page.waitForTimeout(500);
    const file = path.join(out, `${name}-${width}.png`);
    await page.screenshot({ path: file });
    console.log(file);
    await page.close();
  }
}
await browser.close();

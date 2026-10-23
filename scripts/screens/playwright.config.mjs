import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: ".",
  snapshotPathTemplate: "{testDir}/baseline/{arg}{ext}",
  outputDir: "../../test-results/screens",
  reporter: [["list"]],
  use: { viewport: { width: 1280, height: 760 }, locale: "ja-JP", timezoneId: "Asia/Tokyo", colorScheme: "dark", deviceScaleFactor: 1 },
  expect: { toHaveScreenshot: { maxDiffPixels: 100, animations: "disabled", caret: "hide" } },
});

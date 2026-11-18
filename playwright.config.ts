// The screenshot tests (roadmap M6-T10): every screen of the gallery
// (ui/gallery.html) in light and dark, rendered by WebKit and compared with
// the stored images in tests/visual/. `bun run visual -- --update-snapshots`
// stores new ones after a deliberate change; review them before committing.
import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testDir: "tests/visual",
  testMatch: "*.visual.ts",
  snapshotPathTemplate: "tests/visual/{arg}{ext}",
  fullyParallel: true,
  reporter: process.env.CI ? "github" : "list",
  expect: {
    toHaveScreenshot: { maxDiffPixelRatio: 0.03, threshold: 0.3, animations: "disabled" },
  },
  use: {
    baseURL: "http://localhost:5198",
    ...devices["Desktop Safari"],
    deviceScaleFactor: 1,
  },
  projects: [{ name: "webkit", use: { browserName: "webkit" } }],
  webServer: {
    command: "bunx vite --port 5198 --strictPort",
    cwd: "ui",
    url: "http://localhost:5198/gallery.html",
    reuseExistingServer: !process.env.CI,
  },
});

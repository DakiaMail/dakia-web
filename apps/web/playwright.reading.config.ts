import release from "./playwright.release.config";
import { defineConfig } from "@playwright/test";
const port = Number(process.env.DAKIA_READING_PORT || 4198);
const origin = `http://127.0.0.1:${port}`;
process.env.DAKIA_FIXTURE_ORIGIN = origin;
export default defineConfig({
  ...release,
  outputDir: process.env.DAKIA_READING_FULL ? "test-results/reading-full" : "test-results/reading",
  reporter: [["list"], ["json", { outputFile: process.env.DAKIA_READING_FULL ? "test-results/reading-full.json" : "test-results/reading.json" }]],
  testDir: "./browser", testMatch: process.env.DAKIA_READING_FULL ? "**/*.spec.ts" : "reading-*.spec.ts", workers: 1,
  use: { baseURL: origin, viewport: { width: 1480, height: 980 }, launchOptions: { executablePath: process.env.CHROME_PATH, channel: process.env.CHROME_PATH ? undefined : "chrome" } },
  webServer: { command: `npm exec vite -- preview --host 127.0.0.1 --port ${port} --strictPort`, url: origin, reuseExistingServer: false },
});

import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./browser",
  testIgnore: ["pwa.spec.ts", "perf-probe.spec.ts"],
  workers: 1,
  use: {
    baseURL: process.env.DAKIA_TEST_ORIGIN || "http://127.0.0.1:4173",
    launchOptions: { executablePath: process.env.CHROME_PATH || "/usr/bin/chromium", args: ["--no-sandbox"] },
    viewport: { width: 1480, height: 980 },
  },
  webServer: process.env.DAKIA_TEST_ORIGIN ? undefined : {
    command: "npm exec vite -- preview --host 127.0.0.1 --port 4173 --strictPort",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: false,
  },
});

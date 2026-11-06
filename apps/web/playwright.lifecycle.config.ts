import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./browser", testMatch: "lifecycle.spec.ts", workers: 1, forbidOnly: true,
  outputDir: "test-results/lifecycle",
  reporter: [["list"], ["json", { outputFile: process.env.DAKIA_BROWSER_REPORT || "test-results/lifecycle.json" }]],
  use: { baseURL: "http://127.0.0.1:4187", viewport: { width: 390, height: 844 }, launchOptions: { executablePath: process.env.CHROME_PATH || "/usr/bin/google-chrome", args: ["--no-sandbox"] } },
  webServer: { command: "node browser/lifecycle-server.mjs", url: "http://127.0.0.1:4187", reuseExistingServer: false, timeout: 60_000 },
});

import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./browser",
  testMatch: "pwa.spec.ts",
  workers: 1,
  forbidOnly: true,
  outputDir: "test-results/pwa",
  reporter: [["list"], ["json", { outputFile: process.env.DAKIA_BROWSER_REPORT || "test-results/pwa.json" }]],
  use: {
    baseURL: "http://127.0.0.1:4186",
    launchOptions: { executablePath: process.env.CHROME_PATH || "/usr/bin/google-chrome", args: ["--no-sandbox"] },
    viewport: { width: 390, height: 844 },
  },
  webServer: {
    command: "npx vite preview --host 127.0.0.1 --port 4186 --strictPort",
    url: "http://127.0.0.1:4186",
    reuseExistingServer: false,
  },
});

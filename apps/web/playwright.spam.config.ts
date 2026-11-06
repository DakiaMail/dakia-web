import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./browser",
  testMatch: ["spam-toolbar.spec.ts", "reader-actions.spec.ts", "reader-return.spec.ts"],
  workers: 1,
  use: { baseURL: "http://127.0.0.1:4191", launchOptions: { executablePath: process.env.CHROME_PATH, args: ["--no-sandbox"] }, viewport: { width: 1480, height: 980 } },
  webServer: { command: "npm run dev -- --port 4191 --strictPort", url: "http://127.0.0.1:4191", reuseExistingServer: false },
});

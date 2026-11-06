import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./browser",
  testMatch: ["smart-view.spec.ts", "archive.spec.ts", "inbox-cache.spec.ts"],
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:4189",
    launchOptions: { executablePath: process.env.CHROME_PATH || "/usr/bin/chromium", args: ["--no-sandbox"] },
    viewport: { width: 1480, height: 980 },
  },
  webServer: {
    command: "npm exec vite preview -- --host 127.0.0.1 --port 4189 --strictPort",
    url: "http://127.0.0.1:4189",
    reuseExistingServer: false,
  },
});

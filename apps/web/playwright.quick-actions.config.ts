import { defineConfig } from "@playwright/test";
import base from "./playwright.config";
export default defineConfig({
  ...base,
  testMatch: ["contextual-actions.spec.ts", "row-quick-actions.spec.ts"],
  webServer: { command: "npm run dev -- --port 4278 --strictPort", url: "http://127.0.0.1:4278", reuseExistingServer: false },
  use: { ...base.use, baseURL: "http://127.0.0.1:4278", headless: false, hasTouch: true,
    launchOptions: { executablePath: "/usr/bin/chromium", args: ["--no-sandbox"] } },
});

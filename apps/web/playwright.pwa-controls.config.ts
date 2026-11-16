import { defineConfig } from "@playwright/test";
import base from "./playwright.config";
export default defineConfig({
  ...base,
  testIgnore: [],
  testMatch: "pwa-controls.spec.ts",
  use: { ...base.use, baseURL: "http://127.0.0.1:4187" },
  webServer: { command: "npm run dev -- --port 4187 --strictPort", url: "http://127.0.0.1:4187", reuseExistingServer: false },
});

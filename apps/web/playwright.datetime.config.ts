import { defineConfig } from "@playwright/test";
import base from "./playwright.config";
export default defineConfig({
  ...base,
  testMatch: ["date-time.spec.ts", "mail.spec.ts", "keyboard.spec.ts"],
  use: { ...base.use, baseURL: "http://127.0.0.1:4188" },
  webServer: { command: "npm run dev -- --port 4188 --strictPort", url: "http://127.0.0.1:4188", reuseExistingServer: !process.env.CI },
});

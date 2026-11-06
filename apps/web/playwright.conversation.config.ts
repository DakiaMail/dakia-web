import { defineConfig } from "@playwright/test";
import process from "node:process";
import base from "./playwright.config";
export default defineConfig(base, {
  testMatch: ["conversation.spec.ts", "conversation-refresh.spec.ts"],
  use: { baseURL: process.env.DAKIA_TEST_ORIGIN || "http://127.0.0.1:4187" },
  webServer: process.env.DAKIA_TEST_ORIGIN ? undefined : {
    command: "npm exec vite preview -- --host 127.0.0.1 --port 4187 --strictPort",
    url: "http://127.0.0.1:4187", reuseExistingServer: false,
  },
});

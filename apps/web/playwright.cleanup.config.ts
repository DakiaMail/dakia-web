import { defineConfig } from "@playwright/test";
import base from "./playwright.optimistic.config";
export default defineConfig({
  ...base,
  testMatch: ["unsubscribe-cleanup.spec.ts", "unsubscribe.spec.ts", "archive.spec.ts"],
  use: { ...base.use, baseURL: process.env.DAKIA_TEST_ORIGIN || "http://127.0.0.1:4196" },
  webServer: process.env.DAKIA_TEST_ORIGIN ? undefined : {
    command: "npm exec vite -- preview --host 127.0.0.1 --port 4196 --strictPort",
    url: "http://127.0.0.1:4196", reuseExistingServer: false,
  },
});

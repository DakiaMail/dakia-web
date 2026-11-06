import { defineConfig } from "@playwright/test";
import process from "node:process";
import base from "./playwright.config";

const origin = `http://127.0.0.1:${process.env.DAKIA_TEST_PORT || "4194"}`;
export default defineConfig({
  ...base,
  use: { ...base.use, baseURL: origin },
  webServer: {
    command: `npm exec vite preview -- --host 127.0.0.1 --port ${new URL(origin).port} --strictPort`,
    url: origin,
    reuseExistingServer: false,
  },
});

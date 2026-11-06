import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

// Exercise the built candidate in isolation, never a stale sibling dev server.
export default defineConfig({
  ...base,
  use: { ...base.use, baseURL: "http://127.0.0.1:4189" },
  webServer: {
    command: "npm exec vite preview -- --host 127.0.0.1 --port 4189 --strictPort",
    url: "http://127.0.0.1:4189",
    reuseExistingServer: false,
  },
});

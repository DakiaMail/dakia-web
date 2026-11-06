import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

export default defineConfig({
  ...base,
  use: { ...base.use, baseURL: "http://127.0.0.1:4191" },
  webServer: {
    command: "npm exec vite preview -- --host 127.0.0.1 --port 4191 --strictPort",
    url: "http://127.0.0.1:4191",
    reuseExistingServer: false,
  },
});

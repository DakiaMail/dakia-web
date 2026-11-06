import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

// Exercise immutable production assets, including the real font files and CSP,
// rather than letting development transforms change during a verification run.
export default defineConfig({
  ...base,
  webServer: {
    command: "npm exec vite preview -- --host 127.0.0.1 --port 4173 --strictPort",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: false,
  },
});

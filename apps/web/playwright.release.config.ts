import { defineConfig } from "@playwright/test";
import base from "./playwright.config";

// Run the entire suite against a fresh production preview, never an owner's
// running dev server. Chrome is required explicitly by the release runner.
export default defineConfig({
  ...base,
  forbidOnly: true,
  testIgnore: ["personal-first-backend.spec.ts","snooze-backend.spec.ts", "device-translation-backend.spec.ts", "rules-backend.spec.ts", "keyboard-backend.spec.ts", "pwa.spec.ts", "release-backend.spec.ts", "full-text-backend.spec.ts", "image-redirects.spec.ts", "lifecycle.spec.ts", "translation-backend.spec.ts", "unsubscribe-backend.spec.ts", "silent-cleanup-backend.spec.ts"],
  retries: 0,
  outputDir: "test-results/release",
  reporter: [["list"], ["json", { outputFile: process.env.DAKIA_BROWSER_REPORT || "test-results/release.json" }]],
  use: { ...base.use, timezoneId: "Europe/Tallinn", trace: "retain-on-failure", screenshot: "only-on-failure" },
  webServer: {
    command: "npm exec vite preview -- --host 127.0.0.1 --port 4173 --strictPort",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: false,
  },
});

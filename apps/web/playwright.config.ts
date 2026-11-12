import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./browser",
  testIgnore: ["personal-first-backend.spec.ts","snooze-backend.spec.ts", "device-translation-backend.spec.ts", "rules-backend.spec.ts", "keyboard-backend.spec.ts", "pwa.spec.ts", "release-backend.spec.ts", "full-text-backend.spec.ts", "image-redirects.spec.ts", "lifecycle.spec.ts", "translation-backend.spec.ts", "unsubscribe-backend.spec.ts", "silent-cleanup-backend.spec.ts"], // Dedicated production/backend fixtures.
  workers: 1,
  use: {
    baseURL: "http://127.0.0.1:4173",
    launchOptions: {
      executablePath: process.env.CHROME_PATH || "/usr/bin/google-chrome",
      args: ["--no-sandbox"],
    },
    viewport: { width: 1480, height: 980 },
  },
  webServer: {
    command: "npm run dev -- --port 4173 --strictPort",
    url: "http://127.0.0.1:4173",
    reuseExistingServer: true,
  },
});

import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./browser", testMatch: ["unsubscribe.spec.ts", "unsubscribe-cleanup.spec.ts", "bulk-actions.spec.ts", "action-failure.spec.ts", "reader-return.spec.ts", "spam-toolbar.spec.ts"], workers: 1,
  use: { baseURL: process.env.DAKIA_TEST_URL || "http://127.0.0.1:4197", launchOptions: { executablePath: process.env.CHROME_PATH, args: ["--no-sandbox"] }, viewport: { width: 1480, height: 980 } },
  webServer: process.env.DAKIA_TEST_URL ? undefined : { command: "npm exec vite -- preview --host 127.0.0.1 --port 4197 --strictPort", url: "http://127.0.0.1:4197", reuseExistingServer: false },
});

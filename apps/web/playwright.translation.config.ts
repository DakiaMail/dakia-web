import { defineConfig } from "@playwright/test";
import base from "./playwright.backend.config";
export default defineConfig({
  ...base,
  testMatch: "translation-backend.spec.ts",
  outputDir: "test-results/translation",
  reporter: [["list"], ["json", { outputFile: process.env.DAKIA_BROWSER_REPORT || "test-results/translation.json" }]],
  use: { ...base.use, baseURL: "http://127.0.0.1:4336" },
  webServer: {
    command: "cargo test --manifest-path ../../Cargo.toml -p dakia-server --lib browser_translation_fixture -- --ignored --nocapture",
    url: "http://127.0.0.1:4336/healthz", timeout: 180000, reuseExistingServer: false,
    gracefulShutdown: { signal: "SIGTERM", timeout: 1000 },
  },
});

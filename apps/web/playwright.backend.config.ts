import { defineConfig } from "@playwright/test";
import base from "./playwright.release.config";

export default defineConfig({
  ...base,
  testMatch: "release-backend.spec.ts",
  testIgnore: [],
  outputDir: "test-results/backend",
  reporter: [["list"], ["json", { outputFile: process.env.DAKIA_BROWSER_REPORT || "test-results/backend.json" }]],
  use: {
    ...base.use,
    baseURL: "http://127.0.0.1:4327",
  },
  webServer: {
    command: "cargo +1.98.0 test --manifest-path ../../Cargo.toml -p dakia-server --lib browser_release_fixture -- --ignored --nocapture",
    url: "http://127.0.0.1:4327/healthz",
    timeout: 180_000,
    reuseExistingServer: false,
    gracefulShutdown: { signal: "SIGTERM", timeout: 1000 },
  },
});

import { defineConfig } from "@playwright/test";
import base from "./playwright.backend.config";
export default defineConfig({
  ...base,
  outputDir: "test-results/full-text-backend",
  testMatch: "full-text-backend.spec.ts",
  reporter: [["list"], ["json", { outputFile: process.env.DAKIA_BROWSER_REPORT || "test-results/full-text-backend.json" }]],
  webServer: { ...base.webServer as object,
    command: "DAKIA_FULL_TEXT_FIXTURE=1 cargo +1.98.0 test --manifest-path ../../Cargo.toml -p dakia-server --lib browser_release_fixture -- --ignored --nocapture",
    url: "http://127.0.0.1:4327/healthz", timeout: 180000, reuseExistingServer: false,
  },
});

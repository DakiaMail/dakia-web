import { defineConfig } from "@playwright/test";
import base from "./playwright.backend.config";
export default defineConfig({
  ...base,
  testMatch: "silent-cleanup-backend.spec.ts",
  outputDir: "test-results/silent-cleanup",
  reporter: [["list"], ["json", { outputFile: process.env.DAKIA_BROWSER_REPORT || "test-results/silent-cleanup.json" }]],
  webServer: { ...base.webServer as object,
    command: "DAKIA_CLEANUP_FIXTURE=1 cargo +1.98.0 test --manifest-path ../../Cargo.toml -p dakia-server --lib browser_release_fixture -- --ignored --nocapture",
  },
});

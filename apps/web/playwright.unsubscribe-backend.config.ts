import { defineConfig } from "@playwright/test";
import base from "./playwright.backend.config";
export default defineConfig({
  ...base,
  testMatch: "unsubscribe-backend.spec.ts",
  outputDir: "test-results/unsubscribe-backend",
  reporter: [["list"], ["json", { outputFile: process.env.DAKIA_BROWSER_REPORT || "test-results/unsubscribe-backend.json" }]],
  webServer: { ...base.webServer as object,
    command: "cargo +1.98.0 test --manifest-path ../../Cargo.toml -p dakia-server --lib browser_unsubscribe_fixture -- --ignored --nocapture",
  },
});

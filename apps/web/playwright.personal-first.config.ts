import { defineConfig } from "@playwright/test";
import base from "./playwright.backend.config";
export default defineConfig({ ...base, testMatch: "personal-first-backend.spec.ts", testIgnore: [],
  outputDir: "artifacts/personal-first", reporter: [["list"], ["json", { outputFile: "artifacts/personal-first/report.json" }]],
  webServer: { ...base.webServer!, command: "DAKIA_CLASSIFICATION_FIXTURE=1 cargo +1.98.0 test --manifest-path ../../Cargo.toml -p dakia-server --lib browser_release_fixture -- --ignored --nocapture" },
});

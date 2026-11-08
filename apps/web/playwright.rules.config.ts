import { defineConfig } from "@playwright/test";
import base from "./playwright.backend.config";
export default defineConfig({ ...base,
  webServer: { ...base.webServer as object, command: "DAKIA_CLEANUP_FIXTURE=1 DAKIA_RULES_FIXTURE=1 cargo +1.98.0 test --manifest-path ../../Cargo.toml -p dakia-server --lib browser_release_fixture -- --ignored --nocapture" }, testMatch: "rules-backend.spec.ts", outputDir: "artifacts/rules-backend", reporter: [["list"], ["json", { outputFile: "artifacts/rules-backend/report.json" }]] });

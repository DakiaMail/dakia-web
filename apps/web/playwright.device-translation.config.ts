import { defineConfig } from "@playwright/test";
import { fileURLToPath } from "node:url";
import base from "./playwright.backend.config";
export default defineConfig({
  ...base, testMatch: "device-translation-backend.spec.ts", testIgnore: [],
  outputDir: "test-results/device-translation", timeout: 180_000,
  reporter: [["list"], ["json", { outputFile: "test-results/device-translation.json" }]],
  use: { ...base.use, baseURL: "http://127.0.0.1:4346" },
  webServer: {
    command: "cargo test --manifest-path ../../Cargo.toml -p dakia-server --lib browser_device_translation_fixture -- --ignored --nocapture",
    env: { DAKIA_WORKERS: fileURLToPath(new URL("../../workers", import.meta.url)), DAKIA_MODEL_CACHE: process.env.DAKIA_MODEL_CACHE || "/private/tmp/dakia-production-translation-models" },
    url: "http://127.0.0.1:4346/healthz", timeout: 180_000, reuseExistingServer: false,
    gracefulShutdown: { signal: "SIGTERM", timeout: 1000 },
  },
});

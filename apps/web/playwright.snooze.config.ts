import { defineConfig } from "@playwright/test";
import base from "./playwright.backend.config";
export default defineConfig({ ...base, testMatch: "snooze-backend.spec.ts", testIgnore: [], outputDir: "artifacts/snooze-backend", reporter: [["list"]] });

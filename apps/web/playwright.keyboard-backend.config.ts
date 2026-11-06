import { defineConfig } from "@playwright/test";
import base from "./playwright.backend.config";
export default defineConfig({ ...base, testMatch: "keyboard-backend.spec.ts", outputDir: "artifacts/keyboard-backend", reporter: [["list"], ["json", { outputFile: "artifacts/keyboard-backend/report.json" }]] });

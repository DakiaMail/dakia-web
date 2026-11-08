import { defineConfig } from "@playwright/test";
import demo from "./playwright.demo.config";
export default defineConfig({
  ...demo,
  testIgnore: [],
  testMatch: "parity.spec.ts",
  snapshotPathTemplate: "{testDir}/../artifacts/parity-baselines/{arg}{ext}",
  webServer: [
    { command: "npx vite preview --host 127.0.0.1 --outDir dist-demo --port 4175 --strictPort", url: "http://127.0.0.1:4175", reuseExistingServer: !process.env.CI },
    { command: "npx vite preview --host 127.0.0.1 --port 4176 --strictPort", url: "http://127.0.0.1:4176", reuseExistingServer: !process.env.CI },
  ],
});

import { defineConfig } from "@playwright/test";
import base from "./playwright.release.config";

export default defineConfig({
  ...base,
  testIgnore: [],
  outputDir: "test-results/images",
  testMatch: ["image-redirects.spec.ts", "image-errors.spec.ts", "images.spec.ts"],
  webServer: [
    {
      command: "npm exec vite preview -- --host 127.0.0.1 --port 4173 --strictPort",
      url: "http://127.0.0.1:4173",
      reuseExistingServer: false,
    },
    {
      command: "cargo test --locked --manifest-path ../../Cargo.toml -p dakia-server --lib browser_image_redirect_fixture -- --ignored --nocapture",
      url: "http://127.0.0.1:4339/healthz",
      timeout: 180_000,
      reuseExistingServer: false,
    },
  ],
});

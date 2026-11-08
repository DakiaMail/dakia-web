import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./demo-browser",
  workers: 1,
  testIgnore: ["parity.spec.ts"],
  use: {
    baseURL: process.env.DEMO_URL || "http://127.0.0.1:4175",
    launchOptions: { executablePath: process.env.CHROME_PATH || "/usr/bin/google-chrome", args: ["--no-sandbox"] },
    viewport: { width: 1480, height: 980 },
    screenshot: "only-on-failure",
  },
  webServer: process.env.DEMO_URL ? undefined : {
    command: "npx vite preview --host 127.0.0.1 --outDir dist-demo --port 4175 --strictPort",
    url: "http://127.0.0.1:4175", reuseExistingServer: !process.env.CI,
  },
});

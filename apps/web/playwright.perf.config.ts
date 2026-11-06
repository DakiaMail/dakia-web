import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir:"./browser", testMatch:"perf-probe.spec.ts", workers:1,
  use:{baseURL:"http://127.0.0.1:4187",launchOptions:{executablePath:"/usr/bin/chromium",args:["--no-sandbox"]}},
  webServer:{command:"npm exec vite preview -- --host 127.0.0.1 --port 4187 --strictPort",url:"http://127.0.0.1:4187",reuseExistingServer:false},
});

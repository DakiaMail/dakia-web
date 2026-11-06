import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { fileURLToPath } from "node:url";
import { randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
const local = (p: string) => fileURLToPath(new URL(p, import.meta.url));
const buildId = process.env.DAKIA_BUILD_ID || randomUUID();
if (!/^[a-zA-Z0-9_-]{1,100}$/.test(buildId)) throw new Error("Invalid DAKIA_BUILD_ID");
export default defineConfig({
  define: { "import.meta.env.VITE_DAKIA_BUILD_ID": JSON.stringify(buildId) },
  plugins: [react(), {
    name: "dakia-release-version",
    apply: "build",
    generateBundle() {
      this.emitFile({ type: "asset", fileName: "app-version.json", source: JSON.stringify({ version: buildId }) });
      // A UI-only release must also produce a byte-different worker.
      this.emitFile({ type: "asset", fileName: "sw.js", source: `/* Dakia build ${buildId} */\n${readFileSync(local("./public/sw.js"), "utf8")}` });
    },
  }],
  resolve: {
    dedupe: ["react", "react-dom"],
    alias: {
      "@dakia/api-types": local("../../packages/api-types/src/index.ts"),
      "@dakia/mail-renderer": local(
        "../../packages/mail-renderer/src/index.tsx",
      ),
      "@dakia/mail-composer": local(
        "../../packages/mail-composer/src/index.tsx",
      ),

    },
  },
  server: {
    fs: { allow: ["../.."] },
    proxy: {
      "/api": "http://127.0.0.1:3025",
      "/auth": "http://127.0.0.1:3025",
    },
  },
  test: { environment: "jsdom", include: ["src/**/*.test.{ts,tsx}"] },
});

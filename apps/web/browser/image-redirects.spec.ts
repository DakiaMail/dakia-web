import { test, expect } from "@playwright/test";
import { accounts, content, page as mail } from "./fixtures";

// Real Rust redirect coordinator, synthetic DNS/HTTP boundary. No mail or trackers.
for (const width of [390, 1480]) test(`eight redirected images decode independently ${width}`, async ({ page, request }) => {
  await page.setViewportSize({ width, height: 980 });
  const calls: string[] = [];
  const external: string[] = [];
  const html = "<h1>Synthetic redirect newsletter</h1>" +
    Array.from({ length: 8 }, (_, i) => `<p>Asset ${i + 1}: <img src="https://images.example.org/${i}.png" width="80" height="32"></p>`).join("") +
    '<img src="https://images.example.org/failure.png" width="80" height="32">';
  await page.route("**/*", async route => {
    const url = new URL(route.request().url());
    if (url.origin !== (process.env.DAKIA_FIXTURE_ORIGIN || "http://127.0.0.1:4173")) { external.push(url.href); return route.abort(); }
    const path = url.pathname;
    if (!path.startsWith("/api/")) return route.continue();
    const respond = (json: unknown) => route.fulfill({ json });
    if (path === "/api/session") return respond({ user_id: "fixture", csrf_token: "fixture" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": connected\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond(mail);
    if (path.endsWith("/conversation")) return respond({ rows: [content.message], continuation: null, complete: true });
    if (path.endsWith("/messages/m0")) return respond({ ...content, detected_language: "en", html });
    if (path.endsWith("/image")) {
      expect(route.request().headers()["x-csrf-token"]).toBe("fixture");
      const input = route.request().postDataJSON(); calls.push(new URL(input.url).pathname);
      const result = await request.post("http://127.0.0.1:4339/image", { data: input });
      return route.fulfill({ response: result });
    }
    if (path.endsWith("/operations")) { const op = route.request().postDataJSON(); return respond({ ...op, status: "applied", revision: op.expected_revision + 1 }); }
    return respond(null);
  });
  await page.goto("/");
  await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
  const frame = page.frameLocator('iframe[title="Message body"]');
  await expect.poll(() => calls.length).toBe(9);
  await expect(frame.locator("img[src]")).toHaveCount(8);
  await expect.poll(() => frame.locator("img").evaluateAll(nodes => nodes.length === 8 && nodes.every(n => (n as HTMLImageElement).naturalWidth === 80))).toBe(true);
  expect(external).toEqual([]);
  await expect(frame.locator('meta[http-equiv="Content-Security-Policy"]')).toHaveAttribute("content", /img-src data:/);
  await page.screenshot({ path: `artifacts/image-redirects-${width}.png`, fullPage: true });
});

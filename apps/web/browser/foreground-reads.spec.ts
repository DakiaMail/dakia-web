import { test, expect } from "@playwright/test";
import { accounts, content, page as mail } from "./fixtures";

for (const width of [390, 1480]) test(`keyboard open bypasses stalled prefetch, mark-read and discovery at ${width}`, async ({ page }, testInfo) => {
  await page.setViewportSize({ width, height: 844 });
  await page.addInitScript(theme => localStorage.setItem("dakia-theme", theme), width === 390 ? "dark" : "light");
  const message = { ...mail.rows[0], unread: true };
  let prefetches = 0, opens = 0, readUpdates = 0, discoveries = 0;
  let release!: () => void;
  const stalled = new Promise<void>(resolve => { release = resolve; });
  await page.route("**/api/**", async route => {
    const request = route.request(), url = new URL(request.url()), path = url.pathname;
    const respond = (json: unknown) => route.fulfill({ json }).catch(() => {});
    if (path === "/api/session") return respond({ user_id: "foreground-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/mailbox-counts")) return respond({ starred: 0, attention: 0 });
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows: [message], continuation: null, pending: false });
    if (path.endsWith("/conversation")) {
      discoveries++; await stalled;
      return respond({ rows: [message], continuation: null, complete: true });
    }
    if (path.endsWith("/operations") && request.method() === "POST") {
      readUpdates++; await stalled;
      return respond({ ...request.postDataJSON(), status: "applied", revision: message.revision + 1 });
    }
    if (path.endsWith(`/messages/${message.id}`)) {
      if (url.searchParams.get("prefetch") === "true") { prefetches++; await stalled; }
      else opens++;
      return respond({ ...content, message, text: "Foreground body is readable", html: "<p>Foreground body is readable</p>", attachments: [] });
    }
    return respond([]);
  });
  try {
    await page.goto("/");
    await expect.poll(() => prefetches).toBe(1);
    await page.locator(".message-open").focus();
    await page.keyboard.press("Enter");
    await expect(page.frameLocator('iframe[title="Message body"]').getByText("Foreground body is readable")).toBeVisible();
    expect(opens).toBe(1);
    await expect.poll(() => readUpdates).toBe(1);
    await expect.poll(() => discoveries).toBeGreaterThan(0);
    await expect(page.getByText("This message is taking longer to load.")).toHaveCount(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: testInfo.outputPath("foreground-reader.png") });
  } finally { release(); }
});

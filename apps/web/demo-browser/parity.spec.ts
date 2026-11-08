import { test, expect } from "@playwright/test";
import { DemoMailbox } from "../src/demo/mailbox";
import { seed } from "../src/demo/seed";

test("normal and sandbox builds render identical inbox and reader", async ({ page, browser }) => {
  test.skip(!!process.env.DEMO_URL, "Local production-build comparison only");
  const normal = await browser.newPage({ viewport: { width: 1480, height: 980 } });
  let state = seed();
  const mailbox = new DemoMailbox({ read: async () => structuredClone(state), write: async value => { state = structuredClone(value); } });
  await normal.route("**/api/**", async route => {
    if (new URL(route.request().url()).pathname.endsWith("/events")) {
      await route.fulfill({ status: 200, contentType: "text/event-stream", body: 'event: ready\ndata: {"cursor":1,"reset":true}\n\n' }); return;
    }
    const req = route.request();
    const response = await mailbox.fetch(new Request(req.url(), { method: req.method(), headers: req.headers(), ...(req.postDataBuffer() ? { body: new Uint8Array(req.postDataBuffer()!) } : {}) }));
    await route.fulfill({ status: response.status, headers: Object.fromEntries(response.headers), body: Buffer.from(await response.arrayBuffer()) });
  });
  for (const p of [normal, page]) await p.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  await normal.goto("http://127.0.0.1:4176"); await page.goto("/");
  for (const p of [normal, page]) {
    await expect(p.getByRole("button", { name: /Unread Maya Chen/ }).first()).toBeVisible();
    await p.evaluate(() => document.fonts.ready);
  }
  expect(await page.screenshot({ animations: "disabled" })).toEqual(await normal.screenshot({ animations: "disabled" }));
  for (const p of [normal, page]) {
    await p.getByRole("button", { name: /Unread Maya Chen/ }).first().click();
    await expect(p.getByRole("link", { name: /Launch review.txt/ })).toBeVisible();
    await expect(p.frameLocator('iframe[title="Message body"]').first().locator("body")).toContainText("Hi Alex");
  }
  expect(await page.screenshot({ animations: "disabled" })).toEqual(await normal.screenshot({ animations: "disabled" }));
  await normal.close();
});

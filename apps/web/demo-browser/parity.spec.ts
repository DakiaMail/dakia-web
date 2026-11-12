import { test, expect } from "@playwright/test";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { DemoMailbox } from "../src/demo/mailbox";
import { seed } from "../src/demo/seed";

test("normal and sandbox builds render identical inbox and reader", async ({ page, browser }, testInfo) => {
  test.skip(!!process.env.DEMO_URL, "Local production-build comparison only");
  const normal = await browser.newPage({ viewport: { width: 1480, height: 980 } });
  const sceneTime = new Date("2026-11-08T12:00:00Z");
  let state = seed(sceneTime.getTime());
  const mailbox = new DemoMailbox({ read: async () => structuredClone(state), write: async value => { state = structuredClone(value); } }, () => sceneTime.getTime());
  await normal.route("**/api/**", async route => {
    if (new URL(route.request().url()).pathname.endsWith("/events")) {
      await route.fulfill({ status: 200, contentType: "text/event-stream", body: 'event: ready\ndata: {"cursor":1,"reset":true}\n\n' }); return;
    }
    const req = route.request();
    const response = await mailbox.fetch(new Request(req.url(), { method: req.method(), headers: req.headers(), ...(req.postDataBuffer() ? { body: new Uint8Array(req.postDataBuffer()!) } : {}) }));
    await route.fulfill({ status: response.status, headers: Object.fromEntries(response.headers), body: Buffer.from(await response.arrayBuffer()) });
  });
  async function compare(name: string) {
    const expected = await normal.screenshot({ animations: "disabled", path: `artifacts/demo/parity-app-${name}.png` });
    const baseline = testInfo.snapshotPath(`${name}.png`);
    mkdirSync(dirname(baseline), { recursive: true }); writeFileSync(baseline, expected);
    const actual = await page.screenshot({ animations: "disabled", path: `artifacts/demo/parity-demo-${name}.png` });
    // Linux Chrome can vary a few edge pixels by 1/255 between pages.
    // Bound that noise to 50 pixels out of 1.45 million, with a strict threshold.
    expect(actual).toMatchSnapshot(`${name}.png`, { threshold: 0.01, maxDiffPixels: 50 });
  }
  for (const p of [normal, page]) {
    await p.clock.setFixedTime(sceneTime);
    await p.emulateMedia({ colorScheme: "light", reducedMotion: "reduce" });
  }
  await normal.goto("http://127.0.0.1:4176"); await page.goto("/");
  for (const p of [normal, page]) {
    await expect(p.getByRole("button", { name: /Unread Maya Chen/ }).first()).toBeVisible();
    await p.evaluate(() => document.fonts.ready);
  }
  await compare("inbox");
  for (const p of [normal, page]) {
    await p.getByRole("button", { name: /Unread Maya Chen/ }).first().click();
    await expect(p.getByRole("link", { name: /Launch review.txt/ })).toBeVisible();
    await expect(p.frameLocator('iframe[title="Message body"]').first().locator("body")).toContainText("Hi Alex");
  }
  await compare("reader");
  await normal.close();
});

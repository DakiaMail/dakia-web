import { test, expect } from "@playwright/test";
import { mkdirSync, writeFileSync } from "node:fs";
import { accounts, content, page as mail } from "./fixtures";

const paragraph = "A comfortable reading column should remain inset without losing sender spacing. ".repeat(12);
const cases = {
  unpadded: `<h1>Independent newsletter</h1><p>${paragraph}</p>`,
  padded: `<div style="padding:16px"><h1>Authored card</h1><p>${paragraph}</p></div>`,
  nested: `<table width="680" cellpadding="0" cellspacing="0"><tr><td style="padding:10px"><table width="640" cellpadding="0" cellspacing="0"><tr><td><h1>Nested newsletter</h1><p>${paragraph}</p></td></tr></table></td></tr></table>`,
  plaintext: "",
  longword: `<h1>Long references</h1><p>${"reference".repeat(130)}</p><pre>${"code".repeat(150)}</pre>`,
};
for (const [name, html] of Object.entries(cases)) for (const width of [320, 390, 1480, 1920]) test(`measured message gutters ${name} ${width}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 844 });
  const external: string[] = [];
  const writes: string[] = [];
  await page.route("**/*", route => {
    const url = new URL(route.request().url()), path = url.pathname;
    if (url.origin !== (process.env.DAKIA_FIXTURE_ORIGIN || "http://127.0.0.1:4173")) { external.push(url.href); return route.abort(); }
    if (!path.startsWith("/api/")) return route.continue();
    const respond = (json: unknown) => route.fulfill({ json });
    if (path === "/api/session") return respond({ user_id: "fixture", csrf_token: "fixture" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": connected\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows: mail.rows.map(row => ({ ...row, unread: false })) });
    if (path.endsWith("/conversation")) return respond({ rows: [{ ...content.message, unread: false }], continuation: null, complete: true });
    if (path.endsWith("/messages/m0")) return respond({ ...content, message: { ...content.message, unread: false }, html, text: paragraph, detected_language: "en" });
    if (route.request().method() !== "GET") writes.push(path);
    return respond(null);
  });
  await page.goto("/");
  await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
  const frame = page.frameLocator('iframe[title="Message body"]');
  await expect(frame.locator("p").first()).toBeVisible();
  const measure = () => frame.locator("body").evaluate(body => {
    const p = body.querySelector("p")!.getBoundingClientRect();
    return { left: p.left, right: innerWidth - p.right, padding: getComputedStyle(body).paddingLeft, width: innerWidth, overflow: body.scrollWidth - innerWidth,
      clipped: [...body.querySelectorAll("p,h1,pre,table,td")].filter(el => { const r = el.getBoundingClientRect(); return r.left < -1 || r.right > innerWidth + 1; }).map(el => el.tagName) };
  });
  let geometry = await measure();
  await expect(async () => {
    geometry = await measure();
    // Media queries belong to the message document, not the outer app viewport.
    // The split reader at 1480 can itself be narrower than 600px.
    if (geometry.width <= 600) {
      const expected = 12 + (name === "padded" ? 16 : name === "nested" ? 10 : 0);
      expect(geometry.left).toBe(expected);
      expect(geometry.right).toBe(expected);
      expect(geometry.padding).toBe("12px");
      expect(geometry.clipped).toEqual([]);
      expect(geometry.overflow).toBeLessThanOrEqual(1);
    } else expect(geometry.padding).toBe("2px");
  }).toPass();
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
  const iframe = page.locator('iframe[title="Message body"]');
  await expect(async () => {
    const height = await frame.locator("body").evaluate(body => body.getBoundingClientRect().height);
    const scale = await iframe.evaluate(node => (node as HTMLIFrameElement).getBoundingClientRect().width / (node as HTMLIFrameElement).clientWidth);
    expect((await iframe.boundingBox())!.height + 1).toBeGreaterThanOrEqual(height * scale);
  }).toPass();
  await page.locator(".conversation").evaluate(el => { el.scrollTop = el.scrollHeight; });
  const end = await iframe.boundingBox(), scroller = await page.locator(".conversation").boundingBox();
  expect(end!.y + end!.height).toBeLessThanOrEqual(scroller!.y + scroller!.height + 1);
  if (width <= 600) expect((await page.locator(".reader > .reader-toolbar").boundingBox())!.y).toBeGreaterThan(760);
  await page.locator(".conversation").evaluate(el => { el.scrollTop = 0; });
  mkdirSync("artifacts/email-gutters", { recursive: true });
  await page.screenshot({ path: `artifacts/email-gutters/${name}-${width}.png` });
  writeFileSync(`artifacts/email-gutters/${name}-${width}.json`, JSON.stringify(geometry, null, 2));
  expect(external).toEqual([]);
  expect(writes).toEqual([]);
});

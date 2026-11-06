import { test, expect } from "@playwright/test";
import { mkdirSync, readFileSync } from "node:fs";
import { accounts, content, page as mail } from "./fixtures";

const png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAFAAAAAgCAIAAAAKUWg7AAAAQklEQVR4nO3PAQkAIBDAQCMawKxmM4awP1iA3Tr7jmp9PwAGBgYGBgYeE3A94HrA9YDrAdcDrgdcD7gecD3gesD1HgWJJNNntUVKAAAAAElFTkSuQmCC";
// Synthetic fixed-width newsletter structure, not a copy of private bank mail.
const html = `<table width="700" style="width:700px"><tr><td style="padding:16px"><table width="660"><tr><td><p style="text-align:right;color:#e65c00">Example Bank</p><img src="https://images.example.org/banner.png" width="660" height="264" style="width:660px;height:264px" alt="Full newsletter banner"><h1 style="color:#e65c00">Hello, reader</h1><p>Thank you for staying with us!</p><table width="620"><tr><td style="background-color:#fff0e8;padding:12px"><h2>Your entries</h2><p>${"A long unbroken reference: " + "ABCD".repeat(90)}</p></td></tr></table><pre>${"unbroken".repeat(70)}</pre><p>End of newsletter</p></td></tr></table></td></tr></table><script>parent.document.body.textContent='unsafe'</script><img src="https://tracker.example.org/pixel" onerror="alert(1)"><form><input></form>`;

const templates = {
  newsletter: html,
  "colspan-newsletter": readFileSync(new URL("./fixtures/colspan-newsletter.html", import.meta.url), "utf8"),
  "mailchimp-body-image": readFileSync(new URL("./fixtures/mailchimp-body-image.html", import.meta.url), "utf8"),
  "mailchimp-two-column": readFileSync(new URL("./fixtures/mailchimp-two-column.html", import.meta.url), "utf8"),
};
for (const [name, html] of Object.entries(templates)) for (const width of [320, 390, 600, 1480]) test(`responsive ${name} and actions ${width}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 844 });

  const external: string[] = [];
  await page.route("**/*", route => {
    const url = new URL(route.request().url());
    if (url.origin !== new URL(test.info().project.use.baseURL!).origin) { external.push(url.href); return route.abort(); }
    const path = url.pathname;
    if (!path.startsWith("/api/")) return route.continue();
    const respond = (json: unknown) => route.fulfill({ json });
    if (path === "/api/session") return respond({ user_id: "fixture", csrf_token: "fixture" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": connected\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond(mail);
    if (path.endsWith("/conversation")) return respond({ rows: [content.message], continuation: null, complete: true });
    if (path.endsWith("/messages/m0")) return respond({ ...content, detected_language: "en", html });
    if (path.endsWith("/image")) return respond(png);
    if (path.endsWith("/operations")) { const op = route.request().postDataJSON(); return respond({ ...op, status: "applied", revision: op.expected_revision + 1 }); }
    return respond(null);
  });
  await page.goto("/");
  await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
  const reader = page.getByRole("region", { name: "Reading pane" });
  const frame = page.frameLocator('iframe[title="Message body"]');
  await expect(frame.locator("h1").first()).toBeVisible();
  if (name === "newsletter" || name === "colspan-newsletter") await expect(frame.getByAltText("Full newsletter banner")).toHaveJSProperty("naturalWidth", 80);
  await expect(frame.locator("script:not([nonce]),form,input")).toHaveCount(0);
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
  if (width <= 600) {
    const overflow = await frame.locator("body").evaluate(node => [...node.querySelectorAll("table,td,img,pre"), node].map(el => ({ tag: el.tagName, overflow: el.scrollWidth - el.clientWidth })));
    expect(overflow.filter(x => x.overflow > 1)).toEqual([]);
    // A clipped iframe can hide overflow from the outer app. Check rendered
    // bounds too, including leading gutters and nested image/table positions.
    const layout = await frame.locator("body").evaluate(node => {
      const width = node.ownerDocument.documentElement.clientWidth;
      return [...node.querySelectorAll("table,td,img")].map(el => {
        const rect = el.getBoundingClientRect();
        return { tag: el.tagName, left: rect.left, right: rect.right, width };
      }).filter(rect => rect.left < -1 || rect.right > rect.width + 1);
    });
    expect(layout).toEqual([]);
    if (name === "colspan-newsletter") {
      const geometry = await frame.getByAltText("Full newsletter banner").evaluate(el => {
        const rect = el.getBoundingClientRect();
        return { left: rect.left, right: rect.right, width: rect.width, viewport: innerWidth };
      });
      expect(geometry.left).toBeLessThan(geometry.viewport * 0.15);
      expect(geometry.width).toBeGreaterThan(geometry.viewport * 0.7);
      expect(Math.abs(geometry.left - (geometry.viewport - geometry.right))).toBeLessThan(4);
    }
    if (name === "newsletter") {
      const banner = await frame.getByAltText("Full newsletter banner").boundingBox();
      expect(banner!.width / banner!.height).toBeCloseTo(2.5, 1);
    }
    const toolbar = reader.locator(".reader-toolbar");
    if (width === 390) await page.locator(".app").evaluate(el => (el as HTMLElement).style.setProperty("--safe-bottom", "24px"));
    const box = await toolbar.boundingBox();
    expect(box!.y + box!.height).toBeGreaterThanOrEqual(842);
    expect(box!.height).toBeGreaterThanOrEqual(width === 390 ? 80 : 56);
    await toolbar.getByRole("button", { name: "More actions", exact: true }).click();
    const menu = reader.getByRole("group", { name: "More message actions" });
    await expect(menu).toBeVisible();
    expect((await menu.boundingBox())!.y).toBeLessThan(box!.y);
    await page.keyboard.press("Escape");
    await expect(toolbar.getByRole("button", { name: "More actions", exact: true })).toBeFocused();
    await page.locator(".conversation").evaluate(el => { el.scrollTop = el.scrollHeight; });
    if (name === "newsletter") await expect(frame.getByText("End of newsletter", { exact: true })).toBeVisible();
    expect((await toolbar.boundingBox())!.y).toBe(box!.y);
    await page.locator(".conversation").evaluate(el => { el.scrollTop = 0; });
  }
  mkdirSync("artifacts/mobile-reader", { recursive: true });
  await page.screenshot({ path: `artifacts/mobile-reader/${name}-${width}.png`, fullPage: true });
  expect(external).toEqual([]);
  if (width <= 600) {
    await reader.getByRole("button", { name: "More actions", exact: true }).click();
    await reader.getByRole("button", { name: "Reply to selected message", exact: true }).click();
  }
  else {
    await page.locator(".conversation").evaluate(el => { el.scrollTop = el.scrollHeight; });
    await reader.getByRole("button", { name: "Reply", exact: true }).click();
  }
  await page.screenshot({ path: `artifacts/mobile-reader/reply-${name}-${width}.png`, fullPage: true });
  await expect(page.getByRole("dialog")).toBeVisible();
});

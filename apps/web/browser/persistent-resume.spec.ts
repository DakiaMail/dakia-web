import { test, expect } from "@playwright/test";
import { accounts, messages, content, page as mail } from "./fixtures";
for (const mode of ["tab", "network"]) test(`${mode} resume warms the unchanged visible viewport`, async ({ page }) => {
  await page.addInitScript(mode => {
    (window as unknown as { fixturePaused: boolean }).fixturePaused = mode === "tab";
    if (mode === "tab") Object.defineProperty(document, "visibilityState", { configurable: true, get: () => (window as unknown as { fixturePaused: boolean }).fixturePaused ? "hidden" : "visible" });
    else Object.defineProperty(navigator, "onLine", { configurable: true, get: () => !(window as unknown as { fixturePaused: boolean }).fixturePaused });
  }, mode);
  const rows = messages.slice(0, 4).map(m => ({ ...m, unread: false }));
  const bodies: string[] = [];
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    if (path === "/api/session") return route.fulfill({ json: { user_id: "fixture", csrf_token: "fixture" } });
    if (path === "/api/accounts") return route.fulfill({ json: accounts });
    if (path === "/api/search") return route.fulfill({ json: { ...mail, rows } });
    const m = rows.find(m => path.endsWith(`/messages/${m.id}`));
    if (m) { bodies.push(m.id); return route.fulfill({ json: { ...content, message: m } }); }
    return route.fulfill({ json: [] });
  });
  await page.goto("/");
  await expect(page.locator(".message-open").first()).toBeVisible();
  if (mode === "network") await page.evaluate(() => {
    (window as unknown as { fixturePaused: boolean }).fixturePaused = true;
    window.dispatchEvent(new Event("offline"));
  });
  const before = bodies.length;
  expect(before).toBeLessThan(rows.length);
  await page.waitForTimeout(250);
  expect(bodies).toHaveLength(before);
  await page.evaluate(mode => {
    (window as unknown as { fixturePaused: boolean }).fixturePaused = false;
    if (mode === "tab") document.dispatchEvent(new Event("visibilitychange"));
    else window.dispatchEvent(new Event("online"));
  }, mode);
  await expect.poll(() => new Set(bodies).size).toBe(rows.length);
});

import { test, expect } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";

for (const dark of [false, true]) {
  test(`authentic branding without removed width controls ${dark ? "dark" : "light"}`, async ({ page }) => {
    await page.addInitScript(dark => localStorage.setItem("dakia-theme", dark ? "dark" : "light"), dark);
    await page.route("**/api/**", route => {
      const path = new URL(route.request().url()).pathname;
      const body = path === "/api/session" ? { user_id: "fixture", csrf_token: "fixture" }
        : path === "/api/accounts" ? accounts : path === "/api/search" ? mail : [];
      return path.endsWith("/events")
        ? route.fulfill({ contentType: "text/event-stream", body: ":connected\n\n" })
        : route.fulfill({ contentType: "application/json", body: JSON.stringify(body) });
    });
    await page.goto("/");
    await expect(page.locator(".message-row").first()).toBeVisible();
    await expect(page.getByRole("slider", { name: "Message list width" })).toHaveCount(0);
    await expect(page.locator(".list-width-control, .pane-size, .mail-count")).toHaveCount(0);
    await expect(page.getByText(/\d+ loaded/)).toHaveCount(0);
    await expect(page.locator(".brand img")).toHaveAttribute("src", "/dakia-icon-192.png");
    await page.screenshot({ path: `artifacts/branding-list-desktop-${dark ? "dark" : "light"}.png` });
    await page.setViewportSize({ width: 390, height: 844 });
    // Wait for the responsive drawer transition, not a half-translated frame.
    await expect.poll(() => page.locator(".sidebar").evaluate(el => el.getBoundingClientRect().right)).toBeLessThanOrEqual(0);
    await expect(page.getByRole("slider", { name: "Message list width" })).toHaveCount(0);
    await expect(page.getByText(/\d+ loaded/)).toHaveCount(0);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.screenshot({ path: `artifacts/branding-list-mobile-${dark ? "dark" : "light"}.png`, animations: "disabled" });
  });
}

test("only original website artwork ships; ICO is a multi-resolution favicon", async ({ page }) => {
  const response = await page.request.get("/favicon.ico");
  expect(response.ok()).toBe(true);
  const bytes = await response.body();
  expect(bytes.subarray(0, 6).toString("hex")).toBe("000001000300");
  expect([0, 1, 2].map(i => bytes[6 + i * 16])).toEqual([16, 32, 48]);
  const html = await (await page.request.get("/")).text();
  expect(html).not.toContain("dakia-icon.svg");
  const old = await page.request.get("/dakia-icon.svg");
  expect(old.headers()["content-type"]).not.toContain("image/svg+xml");
});

import { test, expect } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";

for (const width of [320, 390, 1480]) for (const dark of [false, true]) {
  test(`refresh states ${width} ${dark ? "dark" : "light"}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.addInitScript(dark => localStorage.setItem("dakia-theme", dark ? "dark" : "light"), dark);
    let state = "pending";
    let release: (() => void) | undefined;
    let requests = 0;
    await page.route("**/api/**", async route => {
      const path = new URL(route.request().url()).pathname;
      if (path === "/api/session") return route.fulfill({ json: { user_id: "fixture", csrf_token: "fixture" } });
      if (path === "/api/accounts") return route.fulfill({ json: accounts });
      if (path === "/api/search") {
        requests++;
        if (state === "network") await new Promise<void>(resolve => { release = resolve; });
        return route.fulfill({ json: { ...mail, pending: state === "pending", coverage: mail.coverage.map(c => ({ ...c, state: state === "network" ? "complete" : state, error: state === "error" ? "provider_unavailable" : null })) } });
      }
      return route.fulfill({ json: [] });
    });
    await page.goto("/");
    const button = page.locator(".header-refresh");
    const tooltip = page.getByRole("tooltip");
    await expect(button).toBeVisible();
    await expect(page.getByText("A few thoughts on the launch", { exact: true })).toBeVisible();
    await expect(button).not.toHaveClass(/is-updating/);
    await expect(page.locator(".inbox-sync-status")).toHaveCount(0);
    await expect(tooltip).toBeHidden();
    await button.hover();
    await expect(tooltip).toHaveText("Inbox refresh is pending.");
    await expect(tooltip).toBeVisible();
    await button.focus();
    await expect(button).toHaveAccessibleDescription("Inbox refresh is pending.");
    await page.screenshot({ path: `../../docs/screenshots/refresh/${width}-${dark ? "dark" : "light"}-updating.png` });
    await button.press("Escape");
    await expect(tooltip).toBeHidden();
    await page.emulateMedia({ reducedMotion: "reduce" });
    await expect(button.locator("svg")).toHaveCSS("animation-name", "none");
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await expect(button.locator("svg")).toHaveCSS("animation-name", "none");
    state = "complete";
    await button.press("Enter");
    await expect(button).not.toHaveClass(/is-updating/);
    await expect(button).toHaveAccessibleDescription("Refresh mail");
    await page.screenshot({ path: `../../docs/screenshots/refresh/${width}-${dark ? "dark" : "light"}-idle.png` });
    state = "network";
    const previousRequests = requests;
    await button.click();
    await expect.poll(() => requests).toBeGreaterThan(previousRequests);
    await expect(button).toHaveClass(/is-updating/);
    await expect(button.locator("svg")).toHaveCSS("animation-name", "mail-refresh-spin");
    state = "error";
    release!();
    await expect(button).toHaveClass(/has-error/);
    await expect(page.locator(".refresh-error-mark")).toBeVisible();
    await expect(page.locator(".inbox-sync-status")).toBeVisible();
    await page.mouse.move(0, 800);
    await button.hover();
    await expect(tooltip).toHaveText("Couldn’t update mail. Try refreshing again.");
    await page.screenshot({ path: `../../docs/screenshots/refresh/${width}-${dark ? "dark" : "light"}-failure.png` });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    state = "complete";
    await button.click();
    await expect(button).not.toHaveClass(/has-error/);
    await expect(page.locator(".inbox-sync-status")).toHaveCount(0);
    if (width <= 600) await page.getByRole("button", { name: "Open navigation" }).click();
    await page.getByRole("button", { name: "Starred", exact: true }).click();
    await expect(button).toHaveAccessibleDescription("Refresh mail");
  });
}

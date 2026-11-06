import { test, expect } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";

for (const width of [320, 390, 1480]) for (const dark of [false, true]) {
  test(`account activity ${width} ${dark ? "dark" : "light"}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.addInitScript(dark => localStorage.setItem("dakia-theme", dark ? "dark" : "light"), dark);
    await page.clock.install();
    let active = "work";
    let failed = false;
    let release: (() => void) | undefined;
    let hold = false;
    let mutations = 0;
    await page.route("**/api/**", async route => {
      if (route.request().method() !== "GET") mutations++;
      const path = new URL(route.request().url()).pathname;
      if (path === "/api/session") return route.fulfill({ json: { user_id: "fixture", csrf_token: "fixture" } });
      if (path === "/api/accounts") return route.fulfill({ json: accounts });
      if (path.endsWith("/sync-active")) {
        if (failed) return route.fulfill({ status: 503, json: { code: "unavailable", message: "Try again" } });
        return route.fulfill({ json: path.includes(`/${active}/`) });
      }
      if (path === "/api/search") {
        if (hold) await new Promise<void>(resolve => { release = resolve; });
        // Both accounts have incomplete coverage even when only one is active.
        return route.fulfill({ json: { ...mail, pending: true, coverage: mail.coverage.map(c => ({ ...c, state: "pending", inbox: { refreshing: true, last_scanned_at: null } })) } });
      }
      return route.fulfill({ json: [] });
    });
    await page.goto("/");
    const refresh = page.locator(".header-refresh");
    await expect(page.getByText("A few thoughts on the launch", { exact: true })).toBeVisible();
    if (width <= 600) await page.getByRole("button", { name: "Open navigation" }).click();
    // CSS compositor transitions use real time, independently of the JS clock.
    await expect.poll(async () => (await page.locator(".sidebar").boundingBox())!.x).toBe(0);
    await page.screenshot({ path: new URL(`../../../verification/screenshots/${process.env.SYNC_BASELINE ? "before" : "after"}-${width}-${dark ? "dark" : "light"}.png`, import.meta.url).pathname });
    await expect(refresh).not.toHaveClass(/is-updating/);
    const work = page.getByRole("img", { name: "Syncing Work", exact: true });
    const personal = page.getByRole("img", { name: "Syncing Personal", exact: true });
    await expect(work).toBeVisible();
    await expect(personal).toHaveCount(0);
    await expect(work).toHaveCSS("width", "6px");
    await expect(work).toHaveCSS("background-color", dark ? "rgb(169, 173, 183)" : "rgb(96, 104, 116)");
    await page.emulateMedia({ reducedMotion: "reduce" });
    await expect(work).toHaveCSS("animation-name", "none");
    await page.emulateMedia({ reducedMotion: "no-preference" });
    await expect(work).toHaveCSS("animation-name", "account-sync-pulse");
    active = "personal";
    // The activity query is independent of mailbox filters and coverage pages.
    await page.getByRole("navigation", { name: "Accounts", exact: true }).getByRole("button", { name: /Work/ }).click();
    await page.clock.fastForward(3100);
    await expect(work).toHaveCount(0);
    await expect(personal).toBeVisible();
    failed = true;
    await page.clock.fastForward(3100);
    await expect(page.locator(".account-sync")).toHaveCount(0);
    failed = false;
    active = "none";
    await page.clock.fastForward(3100);
    await expect(page.locator(".account-sync")).toHaveCount(0);
    if (width <= 600) await page.getByRole("button", { name: "Close navigation", exact: true }).first().click();
    hold = true;
    await refresh.click();
    await expect(refresh).toHaveClass(/is-updating/);
    await page.emulateMedia({ reducedMotion: "reduce" });
    await expect(refresh.locator("svg")).toHaveCSS("animation-name", "none");
    await expect.poll(() => !!release).toBe(true);
    hold = false;
    release!();
    await expect(refresh).not.toHaveClass(/is-updating/);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    expect(mutations).toBe(0);
  });
}

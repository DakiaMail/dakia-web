import { test, expect } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";

test("saved inbox renders while refresh is pending, then settles without a diagnostic panel", async ({ page }) => {
  let searches = 0;
  await page.route("**/api/**", route => {
    const path = new URL(route.request().url()).pathname;
    if (path === "/api/session") return route.fulfill({ json: { user_id: "fixture", csrf_token: "fixture" } });
    if (path === "/api/accounts") return route.fulfill({ json: [accounts[0]] });
    if (path === "/api/search") {
      if (new URL(route.request().url()).searchParams.get("attention") === "true") return route.fulfill({json:{...mail,rows:[],continuation:null}});
      searches++;
      expect(new URL(route.request().url()).searchParams.get("q")).toBe("folder:INBOX");
      return route.fulfill({ json: { ...mail, pending: searches === 1, coverage: [{ ...mail.coverage[0], state: searches === 1 ? "pending" : "complete" }] } });
    }
    return route.fulfill({ json: [] });
  });
  await page.goto("/");
  await expect(page.getByText("A few thoughts on the launch", { exact: true })).toBeVisible();
  // Background cache reconciliation does not impersonate a clicked refresh.
  await expect(page.locator(".header-refresh")).not.toHaveClass(/is-updating/);
  await expect(page.locator(".header-refresh-control [role=status]")).toHaveText("Inbox refresh is pending.");
  await page.screenshot({ path: "test-results/inbox-cached-refresh.png" });
  await expect(page.locator(".header-refresh")).not.toHaveClass(/is-updating/, { timeout: 6000 });
  await expect.poll(() => searches).toBe(2);
});

for (const state of ["pending", "error"] as const) {
  test(`empty ${state} inbox never claims all caught up`, async ({ page }) => {
    await page.route("**/api/**", route => {
      const path = new URL(route.request().url()).pathname;
      if (path === "/api/session") return route.fulfill({ json: { user_id: "fixture", csrf_token: "fixture" } });
      if (path === "/api/accounts") return route.fulfill({ json: accounts });
      if (path !== "/api/search" && !path.endsWith("/messages")) return route.fulfill({ json: [] });
      return route.fulfill({ json: { ...mail, rows: [], pending: state === "pending", coverage: mail.coverage.map(c => ({ ...c, state, error: state === "error" ? "provider_unavailable" : null })) } });
    });
    await page.goto("/");
    await expect(page.getByText("Checking for mail…", { exact: true })).toBeVisible();
    await expect(page.getByText("All caught up", { exact: true })).toHaveCount(0);
    await expect(page.locator(".header-refresh-control [role=status]")).toHaveText(state === "pending" ? "Inbox refresh is pending." : "Couldn’t update mail. Try refreshing again.");
    if (state === "error") await expect(page.locator(".inbox-sync-status")).toHaveText("Couldn’t update inbox. Showing saved mail.");
  });
}

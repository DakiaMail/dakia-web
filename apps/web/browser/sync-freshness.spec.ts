import { test, expect } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";

for (const width of [390, 1480]) {
  test(`limited inbox freshness is not perpetual loading ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.clock.install();
    let phase = "initial";
    let searches = 0;
    await page.route("**/api/**", async route => {
      const path = new URL(route.request().url()).pathname;
      if (path === "/api/session") return route.fulfill({ json: { user_id: "fixture", csrf_token: "fixture" } });
      if (path === "/api/accounts") return route.fulfill({ json: accounts });
      if (path === "/api/search") {
        searches++;
        return route.fulfill({ json: { ...mail, pending: phase === "initial" || phase === "refreshing", coverage: accounts.map((a, i) => ({
          account_id: a.id, folder_ids: ["INBOX"],
          state: phase === "error" && i === 0 ? "error" : phase === "initial" || phase === "refreshing" ? "pending" : i === 0 ? "partial" : "complete",
          error: phase === "error" && i === 0 ? "provider_unavailable" : null,
          inbox: { last_scanned_at: phase === "initial" ? null : 1793286000, refreshing: phase === "initial" || phase === "refreshing" },
        })) } });
      }
      return route.fulfill({ json: [] });
    });
    await page.goto("/");
    const button = page.locator(".header-refresh");
    await expect(button).not.toHaveClass(/is-updating/);
    phase = "fresh";
    await button.click();
    await expect(button).not.toHaveClass(/is-updating/);
    await expect(button).toHaveAccessibleDescription("Inbox refreshed. Provider coverage is limited; some mail may not be exposed.");
    const afterFresh = searches;
    await page.clock.fastForward(12000);
    expect(searches).toBe(afterFresh);
    await button.hover();
    await page.screenshot({ path: `../../docs/screenshots/sync/${width}-limited-fresh.png` });
    phase = "refreshing";
    await button.click();
    await expect(button).not.toHaveClass(/is-updating/);
    phase = "error";
    await button.click();
    await expect(button).toHaveClass(/has-error/);
    await expect(button).toHaveAccessibleDescription("Couldn’t update mail. Try refreshing again.");
    phase = "fresh";
    await button.click();
    await expect(button).not.toHaveClass(/is-updating|has-error/);
    await expect(button).toHaveAccessibleDescription("Inbox refreshed. Provider coverage is limited; some mail may not be exposed.");
  });
}

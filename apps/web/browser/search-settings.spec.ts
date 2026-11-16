import { test, expect } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";
import { mkdirSync } from "node:fs";
for (const theme of ["light", "dark"]) for (const viewport of [{ width: 1480, height: 820 }, { width: 390, height: 568 }, { width: 1024, height: 500 }]) {
  test(`search settings keyboard ${theme} ${viewport.width}x${viewport.height}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    let mode = "automatic";
    await page.route("**/api/**", route => {
      const path = new URL(route.request().url()).pathname;
      if (path === "/api/session") return route.fulfill({ json: { user_id: "search-fixture", csrf_token: "synthetic" } });
      if (path === "/api/accounts") return route.fulfill({ json: accounts });
      if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
      if (path.endsWith("/sync-active")) return route.fulfill({ json: false });
      if (path === "/api/search/settings") {
        if (route.request().method() === "PUT") mode = route.request().postDataJSON().mode;
        return route.fulfill({ json: { mode, enabled: mode === "on", can_manage: true, resources: { memory_bytes: 2147483648, cpus: 2 } } });
      }
      if (path === "/api/translation/settings") return route.fulfill({ json: { mode: "host", can_manage: false } });
      if (path === "/api/search" || path.endsWith("/messages")) return route.fulfill({ json: mail });
      return route.fulfill({ json: [] });
    });
    await page.addInitScript(value => localStorage.setItem("dakia-theme", value), theme);
    await page.goto("/");
    if (viewport.width < 600) { await page.getByRole("button", { name: "Open navigation" }).focus(); await page.keyboard.press("Enter"); }
    await page.getByRole("button", { name: "Manage workspace" }).focus(); await page.keyboard.press("Enter");
    await expect(page.getByRole("heading", { name: "Workspace", exact: true })).toBeFocused();
    const section = page.getByRole("region", { name: "Search", exact: true });
    const automatic = section.getByRole("radio", { name: /^Automatic/ });
    // Traverse the real Tab order to reach the new controls.
    for (let i = 0; i < 80 && !await automatic.evaluate(e => e === document.activeElement); i++) await page.keyboard.press("Tab");
    await expect(automatic).toBeFocused();
    await expect(automatic).toBeChecked();
    await page.keyboard.press("ArrowRight");
    await expect(section.getByRole("radio", { name: /^On Enable/ })).toBeChecked();
    await page.keyboard.press("Tab");
    await expect(section.getByRole("button", { name: "Save search settings" })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(section.getByRole("status").filter({ hasText: /^Search by meaning/ })).toHaveText("Search by meaning is enabled.");
    await page.reload();
    if (viewport.width < 600) { await page.getByRole("button", { name: "Open navigation" }).focus(); await page.keyboard.press("Enter"); }
    await page.getByRole("button", { name: "Manage workspace" }).focus(); await page.keyboard.press("Enter");
    await expect(section.getByRole("radio", { name: /^On Enable/ })).toBeChecked();
    await section.getByRole("radio", { name: /^On Enable/ }).focus();
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("Tab"); await page.keyboard.press("Enter");
    await expect(section.getByRole("status").filter({ hasText: /^Search by meaning/ })).toHaveText("Search by meaning is off.");
    await expect(section.getByRole("button", { name: "Save search settings" })).toBeVisible();
    await section.scrollIntoViewIfNeeded();
    mkdirSync("artifacts/search-settings", { recursive: true });
    await page.screenshot({ path: `artifacts/search-settings/${theme}-${viewport.width}x${viewport.height}.png` });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.getByRole("button", { name: "Back to mail" }).focus(); await page.keyboard.press("Enter");
    await expect(page.getByRole("button", { name: viewport.width < 600 ? "Open navigation" : "Manage workspace" })).toBeFocused();
  });
}

import { test, expect } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";

test("total sidebar counts survive navigation, settings, account changes and reload", async ({ page }) => {
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    const reply = (data: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(data) });
    if (path === "/api/session") return reply({ user_id: "sidebar-counts", csrf_token: "fixture" });
    if (path === "/api/accounts") return reply(accounts);
    if (path.endsWith("/mailbox-counts")) return reply(path.includes(accounts[0].id) ? { starred: 70, attention: 63 } : { starred: 5, attention: 2 });
    if (path === "/api/search") return reply(mail);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    return reply([]);
  });
  await page.goto("/");
  const nav = page.getByRole("navigation", { name: "Mailboxes", exact: true });
  const starred = nav.getByRole("button", { name: "Starred", exact: true });
  const attention = nav.getByRole("button", { name: "Needs attention", exact: true });
  const check = async () => {
    await expect(starred.locator(".count")).toHaveText("75");
    await expect(attention.locator(".count")).toHaveText("65");
    await expect(nav.getByRole("button", { name: "Unified inbox", exact: true }).locator(".count")).toHaveCount(0);
  };
  await check();
  await starred.focus(); await page.keyboard.press("Enter"); await check();
  await attention.focus(); await page.keyboard.press("Enter"); await check();
  await page.getByRole("button", { name: "Manage workspace", exact: true }).focus();
  await page.keyboard.press("Enter"); await check();
  await page.screenshot({ path: "/private/tmp/dakia-sidebar-counts-settings.png" });
  await page.reload(); await check();
  await page.getByRole("navigation", { name: "Accounts", exact: true }).getByRole("button").nth(1).focus();
  await page.keyboard.press("Enter");
  await expect(starred.locator(".count")).toHaveText("70");
  await expect(attention.locator(".count")).toHaveText("63");
  await page.screenshot({ path: "/private/tmp/dakia-sidebar-counts-light.png" });
  await page.getByRole("button", { name: /dark/i }).first().focus(); await page.keyboard.press("Enter");
  await page.screenshot({ path: "/private/tmp/dakia-sidebar-counts-dark.png" });
  await page.setViewportSize({ width: 390, height: 600 });
  await page.getByRole("button", { name: "Open navigation", exact: true }).focus(); await page.keyboard.press("Enter");
  await expect(starred.locator(".count")).toHaveText("70");
  await page.screenshot({ path: "/private/tmp/dakia-sidebar-counts-narrow.png" });
});

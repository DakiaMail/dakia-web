import { test, expect } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";

for (const dark of [false, true]) test(`global search from Snoozed includes unsnoozed mail, dark=${dark}`, async ({ page }, info) => {
  await page.addInitScript(dark => localStorage.setItem("dakia-theme", dark ? "dark" : "light"), dark);
  const snoozed = { ...mail.rows[0], id: "snoozed", thread_id: "snoozed-thread", subject: "Snoozed example", category: "people", category_status: "ready", unread: false };
  const normal = { ...snoozed, id: "normal", thread_id: "normal-thread", subject: "Unsnoozed target" };
  await page.route("**/api/**", route => {
    const url = new URL(route.request().url()), path = url.pathname;
    const respond = (json: unknown) => route.fulfill({ json });
    if (path === "/api/session") return respond({ user_id: "snoozed-search-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/snoozes")) return respond(path.includes(`/${snoozed.account_id}/`) ? [{ thread_id: snoozed.thread_id, until: "2099-01-01T09:00:00Z", message_count: 1 }] : []);
    if (path === "/api/search") return respond({ ...mail, rows: url.searchParams.get("q") === "is:snoozed" ? [snoozed] : [normal], continuation: null, pending: false });
    return respond([]);
  });
  await page.goto("/");
  await expect(page.locator("html")).toHaveAttribute("data-theme", dark ? "dark" : "light");
  for (const reloaded of [false, true]) {
    if (reloaded) await page.reload();
    const snoozedView = page.getByRole("button", { name: "Snoozed", exact: true });
    for (let i = 0; i < 40 && !(await snoozedView.evaluate(el => el === document.activeElement)); i++) await page.keyboard.press("Tab");
    await expect(snoozedView).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(page.getByText("Snoozed example", { exact: true })).toBeVisible();
    await expect(snoozedView.locator(".count")).toHaveText("1");
    await page.keyboard.press("/");
    await expect(page.getByRole("textbox", { name: "Search all mail", exact: true })).toBeFocused();
    await page.keyboard.type("target");
    await page.keyboard.press("Enter");
    await expect(page.getByRole("heading", { name: "Search", exact: true })).toBeVisible();
    await expect(page.locator(".message-open").filter({ hasText: "Unsnoozed target" })).toBeVisible();
  }
  await page.screenshot({ path: info.outputPath("search-from-snoozed.png") });
});

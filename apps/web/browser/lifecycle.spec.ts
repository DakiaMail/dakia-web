import { test, expect, type BrowserContext, type Page } from "@playwright/test";
import { accounts, content, page as mail } from "./fixtures";
async function fixture(context: BrowserContext) {
  await context.route("**/api/**", route => {
    const path = new URL(route.request().url()).pathname;
    const respond = (json: unknown) => route.fulfill({ json });
    if (path === "/api/session") return respond({ user_id: "upgrade-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/sync-active")) return respond(false);
    if (path === "/api/search") return respond({ ...mail, rows: mail.rows.map(m => ({ ...m, unread: false })) });
    if (path.endsWith("/conversation")) return respond({ rows: [{ ...mail.rows[0], unread: false }], continuation: null, complete: true });
    if (path.includes("/messages/")) return respond({ ...content, message: { ...mail.rows[0], unread: false } });
    if (path.includes("/drafts/") && route.request().method() === "PUT") return respond({ ...route.request().postDataJSON(), revision: 1 });
    return respond([]);
  });
}
async function boot(page: Page) {
  await page.goto("/"); await expect(page.getByRole("button", { name: "Compose", exact: true })).toBeVisible();
  await page.evaluate(async () => { await navigator.serviceWorker.ready; });
  await expect.poll(() => page.evaluate(() => !!navigator.serviceWorker.controller)).toBe(true);
}
async function checkVersion(page: Page) {
  await page.clock.install(); await page.clock.fastForward(61_000);
  await page.evaluate(() => window.dispatchEvent(new Event("visibilitychange")));
  await expect(page.getByText("New version available")).toBeVisible();
}
test.beforeEach(async ({ request, context }) => { await request.post("/__test__/deploy/v1"); await fixture(context); });

test("two real releases offer reload; another tab and its draft stay open", async ({ page, context, request }, testInfo) => {
  await boot(page);
  const other = await context.newPage(); await boot(other);
  await expect(page.getByText("New version available")).toHaveCount(0);
  await other.getByRole("button", { name: "Compose", exact: true }).click();
  const draft = other.getByRole("textbox", { name: "Subject", exact: true }); await draft.fill("Do not reload this draft");
  await request.post("/__test__/deploy/v2");
  await page.bringToFront(); await checkVersion(page);
  await expect.poll(() => page.evaluate(async () => !!(await navigator.serviceWorker.getRegistration())?.waiting)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("update-prompt.png"), fullPage: true });
  await other.bringToFront(); await checkVersion(other);
  const update = other.getByRole("complementary", { name: "App update", includeHidden: true });
  await expect(update.getByRole("button", { name: "Reload", includeHidden: true })).toBeDisabled();
  await page.bringToFront();
  await Promise.all([page.waitForEvent("domcontentloaded"), page.getByRole("button", { name: "Reload", exact: true }).click()]);
  await expect(page.getByRole("button", { name: "Compose", exact: true })).toBeVisible();
  await expect(page.getByText("New version available")).toHaveCount(0);
  await expect(draft).toHaveValue("Do not reload this draft");
  expect(await page.evaluate(() => caches.keys())).toEqual([]);
});

test("Later keeps the app open and a failed update check is quiet", async ({ page, request }) => {
  await boot(page); await page.evaluate(() => { document.documentElement.dataset.upgradeBoot = "original-document"; });
  await request.post("/__test__/deploy/v2"); await checkVersion(page);
  await page.getByRole("button", { name: "Later", exact: true }).click();
  await page.clock.fastForward(300_000);
  await expect(page.getByText("New version available")).toHaveCount(0);
  await expect(page.locator("html")).toHaveAttribute("data-upgrade-boot", "original-document");
  await page.route("**/app-version.json", route => route.abort("connectionfailed"));
  await page.clock.fastForward(300_000);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Compose", exact: true })).toBeVisible();
});

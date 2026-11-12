import { test as base, expect, chromium, type Page } from "@playwright/test";
import { mkdirSync, writeFileSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { accounts, content, page as mail } from "./fixtures";
const test = base.extend({
  context: async ({}, use) => {
    const profile = mkdtempSync(join(tmpdir(), "dakia-pwa-"));
    const context = await chromium.launchPersistentContext(profile, {
      executablePath: process.env.CHROME_PATH || "/usr/bin/chromium", headless: !process.env.PWA_HEADED,
      args: ["--no-sandbox"], baseURL: "http://127.0.0.1:4186", viewport: { width: 390, height: 844 },
    });
    await use(context);
    await context.close();
    rmSync(profile, { recursive: true, force: true });
  },
});
const dir = "artifacts/pwa";
async function fixture(page: Page) {
  await page.context().route("**/api/**", route => {
    const path = new URL(route.request().url()).pathname;
    const respond = (data: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(data) });
    if (path === "/api/session") return respond({ user_id: "pwa-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path === "/api/search") return respond({ ...mail, rows: mail.rows.map(m => ({ ...m, unread: false })) });
    if (path.endsWith("/conversation")) return respond({ rows: [{ ...mail.rows[0], unread: false }], continuation: null, complete: true });
    if (path.includes("/messages/")) return respond({ ...content, message: { ...mail.rows[0], unread: false } });
    return respond([]);
  });
  await page.goto("/");
  await expect(page.getByRole("tab", { name: "All", exact: true })).toBeVisible();
}
async function screenshot(page: Page, name: string) {
  mkdirSync(dir, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: `${dir}/${name}.png`, fullPage: true, animations: "disabled" });
}
async function workspace(page: Page) {
  await page.getByRole("button", { name: "Open navigation", exact: true }).click();
  await page.getByRole("button", { name: "Manage workspace", exact: true }).click();
}
test("production manifest, real worker, installability and static-only Cache Storage", async ({ page, context }) => {
  // Seed the old cache before the new worker activates.
  await page.goto("/manifest.webmanifest");
  await page.evaluate(async () => { await caches.open("dakia-shell-v1"); await caches.open("unrelated-app"); });
  await fixture(page);
  await page.evaluate(async () => { await navigator.serviceWorker.ready; });
  const cdp = await context.newCDPSession(page);
  const manifest = await cdp.send("Page.getAppManifest");
  expect(manifest.errors).toEqual([]);
  expect(JSON.parse(manifest.data!)).toMatchObject({ id: "/", start_url: "/", scope: "/", display: "standalone" });
  const result = await cdp.send("Page.getInstallabilityErrors");
  mkdirSync(dir, { recursive: true });
  writeFileSync(`${dir}/chromium-installability.json`, JSON.stringify({ manifest, result }, null, 2));
  expect(result.installabilityErrors).toEqual([]);
  for (const icon of JSON.parse(manifest.data!).icons) {
    const r = await page.request.get(icon.src);
    expect(r.ok()).toBe(true);
    expect(r.headers()["content-type"]).toContain("image/png");
  }
  // Activation cleanup removes only this app's obsolete shell caches.
  await expect.poll(() => page.evaluate(() => caches.keys())).toEqual(["unrelated-app", "dakia-static-v1"]);
  await page.evaluate(() => fetch("/api/private-sentinel").then(r => r.json()));
  await page.reload();
  await expect(page.getByRole("tab", { name: "All", exact: true })).toBeVisible();
  expect(await page.evaluate(() => caches.keys())).toEqual(["unrelated-app", "dakia-static-v1"]);
  const cached = await page.evaluate(async () => (await (await caches.open("dakia-static-v1")).keys()).map(r => new URL(r.url).pathname));
  expect(cached.length).toBeGreaterThan(0);
  expect(cached.every(path => path.startsWith("/assets/"))).toBe(true);
  await screenshot(page, "chromium-mobile-inbox");
  // Exercise a real installation in this disposable Chromium profile only.
  const manifestId = "http://127.0.0.1:4186/";
  await cdp.send("PWA.install", { manifestId, installUrlOrBundleUrl: manifestId });
  try {
  await cdp.send("PWA.changeAppUserSettings", { manifestId, displayMode: "standalone" });
  const installed = await cdp.send("PWA.getOsAppState", { manifestId });
  const opening = context.waitForEvent("page");
  const launched = await cdp.send("PWA.launch", { manifestId });
  writeFileSync(`${dir}/chromium-installed.json`, JSON.stringify({ installed, launched }, null, 2));
  const app = await opening;
  await app.waitForURL(manifestId);
  await expect.poll(() => app.evaluate(() => matchMedia("(display-mode: standalone)").matches)).toBe(true);
  await screenshot(app, "chromium-installed-standalone");
  // Traverse the actual installed app's browser history, not only a viewport
  // emulation. Back closes the reader; Back at the root stays in the PWA.
  await app.getByRole("tab", { name: "All", exact: true }).click();
  await app.locator(".message-open").first().click();
  await expect(app.locator(".app")).toHaveClass(/reading/);
  await app.goBack();
  await expect(app.locator(".app")).not.toHaveClass(/reading/);
  const historyLength = await app.evaluate(() => history.length);
  for (let i = 0; i < 3; i++) {
    await app.goBack();
    await expect.poll(() => app.evaluate(() => history.state?.dakiaNavigation?.id)).not.toBe("boundary");
    await expect(app.locator(".mailbox")).toBeVisible();
    expect(await app.evaluate(() => history.length)).toBe(historyLength);
  }
  } finally {
    await cdp.send("PWA.uninstall", { manifestId });
  }
});
test("iPhone Safari instructions, safe-area layout, reader and compose", async ({ page, context }) => {
  await page.addInitScript(() => Object.defineProperty(navigator, "userAgent", { get: () => "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1" }));
  await fixture(page);
  await workspace(page);
  await page.getByText("How to install", { exact: true }).click();
  await expect(page.getByText(/In Safari, tap Share/)).toBeVisible();
  await screenshot(page, "chromium-iphone-ua-install-instructions");
  // Chromium does not emulate display-mode via Emulation.setEmulatedMedia.
  // Simulate Safari's standalone property, not a native iOS installation.
  await page.addInitScript(() => Object.defineProperty(navigator, "standalone", { value: true }));
  await page.reload();
  await workspace(page);
  await expect(page.getByRole("region", { name: "Install Dakia" })).toHaveCount(0);
  await page.addStyleTag({ content: "body { --safe-top: 47px; --safe-bottom: 34px; --safe-left: 0px; --safe-right: 0px; }" });
  await page.getByRole("button", { name: "Back to mail", exact: true }).click();
  const header = await page.locator(".mailbox > header").boundingBox();
  expect(header!.y).toBe(47);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await screenshot(page, "chromium-standalone-emulated-safe-area-inbox");
  await page.locator(".message-open").first().click();
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  await screenshot(page, "chromium-standalone-emulated-reader");
  await page.getByRole("button", { name: "Back to inbox", exact: true }).click();
  await page.getByRole("button", { name: "Compose", exact: true }).filter({ visible: true }).first().click();
  await expect(page.getByRole("dialog", { name: "New message" })).toBeVisible();
  const dialog = await page.getByRole("dialog", { name: "New message" }).boundingBox();
  expect(dialog!.height).toBeLessThanOrEqual(844);
  await screenshot(page, "chromium-standalone-emulated-compose");
});
test("narrow login has manual install guidance without automatic permission prompts", async ({ page }) => {
  await page.setViewportSize({ width: 320, height: 640 });
  await page.route("**/api/session", route => route.fulfill({ status: 401, contentType: "application/json", body: '{}' }));
  await page.goto("/");
  await expect(page.getByRole("link", { name: "Sign in" })).toBeVisible();
  await expect(page.getByRole("region", { name: "Install Dakia" })).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await screenshot(page, "chromium-320-login");
});

import { test, expect, type Page } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";
async function fixture(page: Page, mobile = false) {
  if (mobile) {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "userAgent", { get: () => "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 Version/18.0 Mobile/15E148 Safari/604.1" });
      Object.defineProperty(navigator, "platform", { get: () => "iPhone" });
      Object.defineProperty(navigator, "maxTouchPoints", { get: () => 5 });
    });
  }
  await page.addInitScript(() => {
    let subscription: object | null = null;
    const endpoint = "https://fcm.googleapis.com/pwa-controls-fixture";
    const registration = { active: {}, showNotification: async () => {}, pushManager: {
      getSubscription: async () => subscription,
      subscribe: async () => subscription = { endpoint, toJSON: () => ({ endpoint, keys: {} }), unsubscribe: async () => { subscription = null; return true; } },
    } };
    Object.defineProperty(navigator, "serviceWorker", { value: { getRegistration: async () => registration } });
    Object.defineProperty(window, "Notification", { value: { permission: "default", requestPermission: async () => "granted" } });
    navigator.registerProtocolHandler = (...args: unknown[]) => { (window as any).protocolRegistration = args; };
  });
  let enabled = false, sends = 0;
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname, method = route.request().method();
    if (path === "/api/session") return route.fulfill({ json: { user_id: "pwa-controls", csrf_token: "synthetic" } });
    if (path === "/api/accounts") return route.fulfill({ json: accounts });
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/sync-active")) return route.fulfill({ json: false });
    if (path === "/api/search") return route.fulfill({ json: mail });
    if (path === "/api/push/public-key") return route.fulfill({ json: "AQID" });
    if (path === "/api/push/subscriptions") { enabled = true; return route.fulfill({ status: 204 }); }
    if (path.startsWith("/api/push/subscriptions/")) { if (method === "DELETE") enabled = false; return route.fulfill(method === "DELETE" ? { status: 204 } : { json: enabled }); }
    if (path.includes("/drafts/") && method === "PUT") return route.fulfill({ json: { ...route.request().postDataJSON(), revision: 1 } });
    if (path.endsWith("/submissions") && method === "POST") sends++;
    return route.fulfill({ json: [] });
  });
  return { enabled: () => enabled, sends: () => sends };
}
async function workspace(page: Page, mobile = false) {
  if (mobile) await page.getByRole("button", { name: "Open navigation", exact: true }).click();
  await page.getByRole("button", { name: "Manage workspace", exact: true }).click();
}
test("notification controls and email registration work by keyboard", async ({ page }) => {
  const state = await fixture(page); await page.goto("/"); await workspace(page);
  const enable = page.getByRole("button", { name: "Enable notifications", exact: true });
  await expect(enable).toBeEnabled(); await enable.focus(); await page.keyboard.press("Enter");
  const disable = page.getByRole("button", { name: "Disable notifications" });
  await expect(disable).toBeVisible(); expect(state.enabled()).toBe(true);
  await expect(disable).toBeFocused(); await page.keyboard.press("Tab");
  await expect(page.getByRole("button", { name: "Test notification" })).toBeFocused(); await page.keyboard.press("Enter");
  await expect(page.getByRole("status").filter({ hasText: "This checks this device" })).toBeVisible();
  await disable.focus(); await page.keyboard.press("Enter"); await expect(enable).toBeVisible(); expect(state.enabled()).toBe(false);
  const register = page.getByRole("button", { name: "Use Dakia for email links" }); await register.focus(); await page.keyboard.press("Enter");
  expect(await page.evaluate(() => (window as any).protocolRegistration)).toEqual(["mailto", "http://127.0.0.1:4187/?mailto=%s"]);
  await page.screenshot({ path: "artifacts/pwa-controls/settings-light.png", fullPage: true, animations: "disabled" });
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  await page.screenshot({ path: "artifacts/pwa-controls/settings-dark.png", fullPage: true, animations: "disabled" });
});
test("an email link opens a prefilled draft once without sending", async ({ page }) => {
  const state = await fixture(page);
  await page.goto(`/?mailto=${encodeURIComponent("mailto:alex+work@example.com?cc=copy@example.com&subject=Hello&body=First%20line%0A%3Cexample%3E")}`);
  const dialog = page.getByRole("dialog", { name: "New message", exact: true });
  await expect(dialog).toBeVisible(); await expect(dialog.getByLabel("Subject", { exact: true })).toHaveValue("Hello");
  await expect(dialog.getByRole("textbox", { name: "To", exact: true })).toHaveValue("alex+work@example.com");
  expect(new URL(page.url()).searchParams.has("mailto")).toBe(false); expect(state.sends()).toBe(0);
  await page.keyboard.press("Escape"); await expect(dialog).not.toBeVisible();
  await page.reload(); await expect(dialog).not.toBeVisible();
});
test("mobile installation guide closes with Escape and restores focus", async ({ page }) => {
  await fixture(page, true); await page.goto("/"); await workspace(page, true);
  const install = page.getByRole("button", { name: "Install Dakia", exact: true });
  await install.focus(); await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Install Dakia", exact: true });
  await expect(dialog).toBeVisible();
  await expect(page.locator("pwa-install").getByRole("button", { name: /Add to Home Screen/i })).toBeVisible();
  await page.screenshot({ path: "artifacts/pwa-controls/install-mobile-light.png", animations: "disabled" });
  await expect(page.locator("pwa-install").getByRole("button", { name: "close", exact: true })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.locator("pwa-install").getByRole("button", { name: "Add to Home Screen", exact: true })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.locator("pwa-install").locator("article")).toHaveClass(/how-to/);
  await expect(dialog).toBeVisible();
  await page.screenshot({ path: "artifacts/pwa-controls/install-mobile-instructions.png", animations: "disabled" });
  await page.keyboard.press("Escape"); await expect(dialog).not.toBeVisible(); await expect(install).toBeFocused();
  await install.press("Enter"); await expect(dialog).toBeVisible(); await page.emulateMedia({ colorScheme: "dark" });
  await page.screenshot({ path: "artifacts/pwa-controls/install-mobile-dark.png", animations: "disabled" });
  await page.setViewportSize({ width: 320, height: 568 });
  await page.locator("pwa-install").evaluate(async element => {
    await Promise.all(element.shadowRoot!.querySelector("article")!.getAnimations({ subtree: true }).map(animation => animation.finished.catch(() => {})));
  });
  await expect(page.locator("pwa-install").getByRole("button", { name: "Add to Home Screen", exact: true })).toBeInViewport();
  await page.screenshot({ path: "artifacts/pwa-controls/install-mobile-short.png", animations: "disabled" });
  await page.keyboard.press("Escape");
});

test("mobile invitation can be dismissed and stays dismissed after reload", async ({ page }) => {
  await fixture(page, true); await page.clock.install(); await page.goto("/");
  await expect(page.locator(".message-open").first()).toBeVisible();
  await page.clock.fastForward(8100);
  const invitation = page.getByRole("complementary", { name: "Add Dakia to your home screen" });
  await expect(invitation).toBeVisible();
  await page.screenshot({ path: "artifacts/pwa-controls/invitation-light.png", animations: "disabled" });
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  await page.screenshot({ path: "artifacts/pwa-controls/invitation-dark.png", animations: "disabled" });
  await invitation.getByRole("button", { name: "Not now" }).focus(); await page.keyboard.press("Enter");
  await expect(invitation).not.toBeVisible();
  await page.reload(); await page.clock.fastForward(10000); await expect(invitation).not.toBeVisible();
});
test("Android library dialog uses the captured browser offer", async ({ page }) => {
  await fixture(page); await page.setViewportSize({ width: 390, height: 844 });
  await page.addInitScript(() => {
    Object.defineProperty(navigator, "userAgent", { get: () => "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 Chrome/130.0.0.0 Mobile Safari/537.36" });
    Object.defineProperty(navigator, "maxTouchPoints", { get: () => 5 });
  });
  await page.clock.install(); await page.goto("/");
  await expect(page.locator(".message-open").first()).toBeVisible();
  await page.evaluate(() => {
    const event = Object.assign(new Event("beforeinstallprompt", { cancelable: true }), { platforms: ["web"], prompt: async () => { (window as any).installCalls = ((window as any).installCalls || 0) + 1; }, userChoice: Promise.resolve({ outcome: "dismissed", platform: "web" }) });
    window.dispatchEvent(event);
  });
  await page.clock.fastForward(8100);
  await page.getByRole("complementary", { name: "Add Dakia to your home screen" }).getByRole("button", { name: "Install Dakia" }).click();
  const dialog = page.getByRole("dialog", { name: "Install Dakia", exact: true });
  await expect(dialog).toBeVisible();
  const install = page.locator("pwa-install").getByRole("button", { name: "Install", exact: true });
  await expect(install).toBeVisible();
  await expect(install).toBeInViewport();
  await page.screenshot({ path: "artifacts/pwa-controls/install-android.png", animations: "disabled" });
  await install.focus(); await page.keyboard.press("Enter");
  await expect.poll(() => page.evaluate(() => (window as any).installCalls)).toBe(1);
  await expect(dialog).not.toBeVisible();
  await expect(page.locator(".mailbox")).toBeFocused();
});

test("settings stay usable on short mobile screens in both themes", async ({ page }) => {
  await fixture(page, true); await page.setViewportSize({ width: 320, height: 568 }); await page.goto("/"); await workspace(page, true);
  const notifications = page.getByRole("region", { name: "Notifications", exact: true });
  await expect(notifications.getByRole("button", { name: "Enable notifications" })).toBeEnabled();
  await notifications.screenshot({ path: "artifacts/pwa-controls/notifications-mobile-light.png" });
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  await notifications.screenshot({ path: "artifacts/pwa-controls/notifications-mobile-dark.png" });
  await page.getByRole("region", { name: "Email links", exact: true }).screenshot({ path: "artifacts/pwa-controls/email-links-mobile-dark.png" });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

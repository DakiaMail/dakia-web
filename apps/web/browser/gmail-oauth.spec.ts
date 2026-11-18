import { test, expect } from "@playwright/test";

for (const [theme, width, height] of [
  ["light", 1280, 900], ["dark", 1280, 900], ["light", 390, 700], ["dark", 900, 420],
] as const) {
  test(`Gmail discovery and keyboard setup ${theme} ${width}x${height}`, async ({ page }) => {
    await page.setViewportSize({ width, height });
    await page.addInitScript(theme => localStorage.setItem("dakia-theme", theme), theme);
    let submitted: unknown;
    let connected = false;
    let needsReconnect = false;
    await page.route("**/api/**", route => {
      const path = new URL(route.request().url()).pathname;
      const respond = (json: unknown) => route.fulfill({ json });
      if (path === "/api/session") return respond({ user_id: "fixture", csrf_token: "synthetic-csrf" });
      if (path === "/api/accounts/gmail") {
        if (route.request().method() === "POST") {
          expect(route.request().headers()["x-csrf-token"]).toBe("synthetic-csrf");
          submitted = route.request().postDataJSON();
          return respond({ url: new URL("/oauth-fixture", route.request().url()).href });
        }
        return respond({ enabled: true });
      }
      if (path === "/api/accounts") return respond(connected ? [{ id: "gmail-fixture", display_name: "Personal", email: "fixture@gmail.com", provider: "gmail", state: needsReconnect ? "reconnect" : "ready", revision: 1 }] : []);
      if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ":ready\n\n" });
      if (path === "/api/search") return respond({ rows: [], continuation: null, coverage: [] });
      return respond([]);
    });
    await page.route("**/oauth-fixture", route => {
      connected = true;
      needsReconnect = false;
      return route.fulfill({ status: 302, headers: { location: new URL("/", route.request().url()).href }, body: "" });
    });
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Welcome to Dakia" })).toBeVisible();
    const trigger = page.getByRole("button", { name: "Connect account", exact: true }).last();
    await trigger.focus();
    await page.keyboard.press("Enter");
    const dialog = page.getByRole("dialog", { name: "Connect an account" });
    await expect(dialog.getByRole("button", { name: "Close", exact: true })).toBeFocused();
    await page.keyboard.press("Tab");
    const gmail = dialog.getByRole("button", { name: "Connect Gmail", exact: true });
    await expect(gmail).toBeFocused();
    await expect(gmail.locator("img")).toHaveJSProperty("naturalWidth", 96);
    await page.screenshot({ animations: "disabled", path: `artifacts/gmail-entry-${theme}-${width}x${height}.png` });
    // Leave Automatic selected: entering Gmail must choose OAuth, not a password.
    await page.keyboard.press("Tab");
    await expect(dialog.getByRole("combobox")).toBeFocused();
    await page.keyboard.press("Tab");
    await page.keyboard.insertText("Personal");
    await page.keyboard.press("Tab");
    await page.keyboard.insertText("fixture@gmail.com");
    await expect(dialog.locator('input[type="password"]')).toHaveCount(0);
    await page.keyboard.press("Tab");
    const fallback = dialog.getByRole("button", { name: "Use app password instead" });
    await expect(fallback).toBeFocused();
    await expect(gmail).toBeEnabled();
    await gmail.scrollIntoViewIfNeeded();
    await page.screenshot({ animations: "disabled", path: `artifacts/gmail-oauth-${theme}-${width}x${height}.png` });
    expect(await dialog.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
    await page.keyboard.press("Enter");
    await page.keyboard.press("Tab");
    await expect(dialog.getByLabel("App password", { exact: true })).toBeFocused();
    await page.screenshot({ animations: "disabled", path: `artifacts/gmail-app-password-${theme}-${width}x${height}.png` });
    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("Enter");
    await page.keyboard.press("Tab");
    await expect(gmail).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await expect(trigger).toBeFocused();
    await page.keyboard.press("Enter");
    await page.keyboard.press("Tab");
    await page.keyboard.press("Enter"); // Explicit Gmail entry also works for Workspace addresses.
    await dialog.getByLabel("Account name").focus();
    await page.keyboard.insertText("Personal");
    await page.keyboard.press("Tab");
    await page.keyboard.insertText("fixture@gmail.com");
    await page.keyboard.press("Tab");
    await page.keyboard.press("Tab");
    await expect(gmail).toBeFocused();
    await page.keyboard.press("Enter");
    await expect.poll(() => submitted).toEqual({ email: "fixture@gmail.com", display_name: "Personal", account_id: null });
    await expect(dialog).toHaveCount(0);
    await page.reload();
    await expect(page.locator('nav[aria-label="Accounts"]')).toContainText("Personal");
    needsReconnect = true;
    await page.reload();
    if (width < 601) {
      await page.getByRole("button", { name: "Open navigation" }).focus();
      await page.keyboard.press("Enter");
    }
    await page.getByRole("button", { name: "Manage workspace" }).focus();
    await page.keyboard.press("Enter");
    const reconnect = page.getByRole("button", { name: "Reconnect", exact: true });
    await reconnect.focus();
    await page.keyboard.press("Enter");
    const reconnectDialog = page.getByRole("dialog", { name: "Reconnect account" });
    await expect(reconnectDialog.getByLabel("Email", { exact: true })).toHaveValue("fixture@gmail.com");
    await expect(reconnectDialog.locator('input[type="password"]')).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(reconnect).toBeFocused();
    await page.keyboard.press("Enter");
    const reconnectGmail = reconnectDialog.getByRole("button", { name: "Connect Gmail", exact: true });
    await expect(reconnectGmail).toBeEnabled();
    await reconnectGmail.focus();
    await expect(reconnectGmail).toBeFocused();
    await page.keyboard.press("Enter");
    await expect.poll(() => submitted).toEqual({ email: "fixture@gmail.com", display_name: "Personal", account_id: "gmail-fixture" });
    await expect.poll(() => needsReconnect).toBe(false);
  });
}

test("Gmail cancellation returns a readable error", async ({ page }) => {
  await page.route("**/api/**", route => route.fulfill({ contentType: "application/json", body: JSON.stringify(new URL(route.request().url()).pathname === "/api/session" ? { user_id: "fixture", csrf_token: "synthetic" } : []) }));
  await page.goto("/?gmail_error=cancelled");
  await expect(page.getByRole("alert")).toContainText("Google sign-in was cancelled");
});

for (const [reason, message] of [
  ["exchange", "couldn’t finish Google sign-in"],
  ["profile", "couldn’t confirm your Google account"],
  ["authentication", "Gmail rejected the connection"],
  ["connection", "couldn’t reach Gmail"],
]) {
  test(`Gmail callback explains ${reason} failure`, async ({ page }) => {
    await page.route("**/api/**", route => route.fulfill({ contentType: "application/json", body: JSON.stringify(new URL(route.request().url()).pathname === "/api/session" ? { user_id: "fixture", csrf_token: "synthetic" } : []) }));
    await page.goto(`/?gmail_error=${reason}`);
    await expect(page.getByRole("alert")).toContainText(message);
  });
}

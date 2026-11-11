import { test, expect } from "@playwright/test";

for (const [theme, width, height] of [
  ["light", 1280, 900], ["dark", 1280, 900], ["light", 390, 700], ["dark", 900, 420],
] as const) {
  test(`Google account setup by keyboard ${theme} ${width}x${height}`, async ({ page }) => {
    await page.setViewportSize({ width, height });
    await page.addInitScript(theme => localStorage.setItem("dakia-theme", theme), theme);
    let connected = false;
    let needsReconnect = false;
    let submitted: unknown;
    await page.route("**/api/**", route => {
      const path = new URL(route.request().url()).pathname;
      const respond = (body: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(body) });
      if (path === "/api/session") return respond({ user_id: "gmail-fixture", csrf_token: "synthetic-csrf" });
      if (path === "/api/accounts/gmail") {
        if (route.request().method() === "POST") {
          expect(route.request().headers()["x-csrf-token"]).toBe("synthetic-csrf");
          submitted = route.request().postDataJSON();
          return respond({ url: new URL("/oauth-fixture", route.request().url()).href });
        }
        return respond({ enabled: true });
      }
      if (path === "/api/accounts") return respond(connected ? [{ id: "gmail-fixture", display_name: "Disposable", email: "fixture@example.test", provider: "gmail", state: needsReconnect ? "reconnect" : "ready", revision: 1 }] : []);
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
    await expect(trigger).toBeFocused();
    await page.keyboard.press("Enter");
    const dialog = page.getByRole("dialog", { name: "Connect an account" });
    await expect(dialog.getByRole("button", { name: "Close", exact: true })).toBeFocused();
    await page.keyboard.press("Tab");
    await expect(dialog.getByRole("combobox")).toBeFocused();
    await page.keyboard.press("g");
    await expect(dialog.getByRole("combobox")).toHaveValue("gmail");
    await expect(dialog.getByRole("checkbox", { name: "Use an app password" })).toBeVisible();
    await page.keyboard.press("Tab");
    await expect(dialog.getByRole("textbox", { name: "Account name" })).toBeFocused();
    await page.keyboard.insertText("Disposable");
    await page.keyboard.press("Tab");
    await page.keyboard.insertText("fixture@example.test");
    await page.keyboard.press("Tab");
    const fallback = dialog.getByRole("checkbox", { name: "Use an app password" });
    await expect(fallback).toBeFocused();
    await expect(dialog.getByRole("button", { name: "Continue with Google" })).toBeVisible();
    await expect(dialog.locator('input[type="password"]')).toHaveCount(0);
    await page.screenshot({ animations: "disabled", path: `artifacts/gmail-oauth-${theme}-${width}x${height}.png` });
    expect(await dialog.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
    await page.keyboard.press("Space");
    await page.keyboard.press("Tab");
    await expect(dialog.getByLabel("App password", { exact: true })).toBeFocused();
    await page.keyboard.press("Tab");
    await expect(dialog.getByRole("button", { name: "Connect account", exact: true })).toBeFocused();
    await page.screenshot({ animations: "disabled", path: `artifacts/gmail-app-password-${theme}-${width}x${height}.png` });
    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("Space");
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await expect(trigger).toBeFocused();
    await page.keyboard.press("Enter");
    await page.keyboard.press("Tab");
    await page.keyboard.press("g");
    await expect(dialog.getByRole("combobox")).toHaveValue("gmail");
    await expect(dialog.getByRole("checkbox", { name: "Use an app password" })).toBeVisible();
    await page.keyboard.press("Tab");
    await page.keyboard.insertText("Disposable");
    await page.keyboard.press("Tab");
    await page.keyboard.insertText("fixture@example.test");
    await page.keyboard.press("Tab");
    await page.keyboard.press("Tab");
    await expect(page.getByRole("button", { name: "Continue with Google" })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect.poll(() => submitted).toEqual({ email: "fixture@example.test", display_name: "Disposable", account_id: null });
    await expect(dialog).toHaveCount(0);
    await expect.poll(() => connected).toBe(true);
    await expect(page.getByRole("combobox", { name: "Account filter" })).toContainText("Disposable");
    await page.reload();
    await expect(page.getByRole("combobox", { name: "Account filter" })).toContainText("Disposable");
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
    await expect(reconnectDialog.getByRole("button", { name: "Continue with Google" })).toBeVisible();
    await expect(reconnectDialog.getByRole("textbox", { name: "Email", exact: true })).toHaveValue("fixture@example.test");
    await page.screenshot({ animations: "disabled", path: `artifacts/gmail-reconnect-${theme}-${width}x${height}.png` });
    await page.keyboard.press("Escape");
    await expect(reconnect).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(reconnectDialog.getByRole("button", { name: "Continue with Google" })).toBeVisible();
    for (let n = 0; n < 5; n++) await page.keyboard.press("Tab");
    await expect(reconnectDialog.getByRole("button", { name: "Continue with Google" })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect.poll(() => submitted).toEqual({ email: "fixture@example.test", display_name: "Disposable", account_id: "gmail-fixture" });
    await expect.poll(() => needsReconnect).toBe(false);
  });
}

test("Gmail cancellation returns a readable error", async ({ page }) => {
  await page.route("**/api/**", route => route.fulfill({ contentType: "application/json", body: JSON.stringify(new URL(route.request().url()).pathname === "/api/session" ? { user_id: "fixture", csrf_token: "synthetic" } : []) }));
  await page.goto("/?gmail_error=cancelled");
  await expect(page.getByRole("alert")).toContainText("Google sign-in was cancelled");
});

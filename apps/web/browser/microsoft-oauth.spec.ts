import { test, expect } from "@playwright/test";

for (const [theme, width, height] of [
  ["light", 1280, 900], ["dark", 1280, 900], ["light", 390, 700], ["dark", 900, 420],
] as const) {
  test(`Microsoft setup and reconnect by keyboard ${theme} ${width}x${height}`, async ({ page }) => {
    await page.setViewportSize({ width, height });
    await page.addInitScript(theme => localStorage.setItem("dakia-theme", theme), theme);
    let connected = false;
    let needsReconnect = false;
    let submitted: unknown;
    await page.route("**/api/**", route => {
      const path = new URL(route.request().url()).pathname;
      const respond = (body: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(body) });
      if (path === "/api/session") return respond({ user_id: "microsoft-fixture", csrf_token: "synthetic-csrf" });
      if (path === "/api/accounts/microsoft") {
        if (route.request().method() === "POST") {
          expect(route.request().headers()["x-csrf-token"]).toBe("synthetic-csrf");
          submitted = route.request().postDataJSON();
          return respond({ url: new URL("/microsoft-fixture", route.request().url()).href });
        }
        return respond({ enabled: true });
      }
      if (path === "/api/accounts/gmail") return respond({ enabled: false });
      if (path === "/api/accounts") return respond(connected ? [{ id: "microsoft-fixture", display_name: "Disposable", email: "fixture@example.test", provider: "microsoft", state: needsReconnect ? "reconnect" : "ready", revision: 1 }] : []);
      if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ":ready\n\n" });
      if (path === "/api/search") return respond({ rows: [], continuation: null, coverage: [] });
      return respond([]);
    });
    await page.route("**/microsoft-fixture", route => {
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
    await expect(dialog.getByRole("combobox")).toBeFocused();
    await page.keyboard.press("m");
    await expect(dialog.getByRole("combobox")).toHaveValue("microsoft");
    await expect(dialog.getByRole("button", { name: "Continue with Microsoft" })).toBeEnabled();
    await page.keyboard.press("Tab");
    await expect(dialog.getByRole("textbox", { name: "Account name" })).toBeFocused();
    await page.keyboard.insertText("Disposable");
    await page.keyboard.press("Tab");
    await page.keyboard.insertText("fixture@tenant.test");
    await page.keyboard.press("Tab");
    await expect(dialog.getByRole("button", { name: "Continue with Microsoft" })).toBeFocused();
    await expect(dialog.locator('input[type="password"]')).toHaveCount(0);
    expect(await dialog.evaluate(el => {
      const button = el.querySelector("button.primary")!.getBoundingClientRect();
      const bounds = el.getBoundingClientRect();
      return button.bottom <= bounds.bottom - 10;
    })).toBe(true);
    await page.screenshot({ animations: "disabled", path: `artifacts/microsoft-oauth-${theme}-${width}x${height}.png` });
    expect(await dialog.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await expect(trigger).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(dialog.getByRole("button", { name: "Close", exact: true })).toBeFocused();
    await page.keyboard.press("Tab");
    await page.keyboard.press("m");
    await expect(dialog.getByRole("button", { name: "Continue with Microsoft" })).toBeEnabled();
    await page.keyboard.press("Tab");
    await page.keyboard.insertText("Disposable");
    await page.keyboard.press("Tab");
    await page.keyboard.insertText("fixture@tenant.test");
    await page.keyboard.press("Tab");
    await expect(dialog.getByRole("button", { name: "Continue with Microsoft" })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect.poll(() => submitted).toEqual({ email: "fixture@tenant.test", display_name: "Disposable", account_id: null });
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
    await expect(reconnectDialog.getByRole("button", { name: "Continue with Microsoft" })).toBeVisible();
    await expect(reconnectDialog.getByRole("textbox", { name: "Email", exact: true })).toHaveValue("fixture@example.test");
    await page.screenshot({ animations: "disabled", path: `artifacts/microsoft-reconnect-${theme}-${width}x${height}.png` });
    await page.keyboard.press("Escape");
    await expect(reconnect).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(reconnectDialog.getByRole("button", { name: "Close", exact: true })).toBeFocused();
    await expect(reconnectDialog.getByRole("button", { name: "Continue with Microsoft" })).toBeEnabled();
    for (let n = 0; n < 4; n++) await page.keyboard.press("Tab");
    await expect(reconnectDialog.getByRole("button", { name: "Continue with Microsoft" })).toBeFocused();
    await page.screenshot({ animations: "disabled", path: `artifacts/microsoft-reconnect-focused-${theme}-${width}x${height}.png` });
    await page.keyboard.press("Enter");
    await expect.poll(() => submitted).toEqual({ email: "fixture@example.test", display_name: "Disposable", account_id: "microsoft-fixture" });
    await expect.poll(() => needsReconnect).toBe(false);
  });
}

test("Microsoft unavailable setup never asks for a password", async ({ page }) => {
  await page.route("**/api/**", route => route.fulfill({ contentType: "application/json", body: JSON.stringify(new URL(route.request().url()).pathname === "/api/session" ? { user_id: "fixture", csrf_token: "synthetic" } : new URL(route.request().url()).pathname === "/api/accounts/microsoft" ? { enabled: false } : []) }));
  await page.goto("/");
  await page.getByRole("button", { name: "Connect account", exact: true }).last().focus();
  await page.keyboard.press("Enter");
  await page.keyboard.press("Tab");
  await page.keyboard.press("m");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("status")).toContainText("Microsoft sign-in is not configured");
  await expect(dialog.locator('input[type="password"]')).toHaveCount(0);
  await expect(dialog.getByRole("button", { name: "Connect account", exact: true })).toBeDisabled();
  await page.screenshot({ animations: "disabled", path: "artifacts/microsoft-unconfigured.png" });
});

test("Microsoft cancellation returns a readable error", async ({ page }) => {
  await page.route("**/api/**", route => route.fulfill({ contentType: "application/json", body: JSON.stringify(new URL(route.request().url()).pathname === "/api/session" ? { user_id: "fixture", csrf_token: "synthetic" } : []) }));
  await page.goto("/?microsoft_error=cancelled");
  await expect(page.getByRole("alert")).toContainText("Microsoft sign-in was cancelled");
});

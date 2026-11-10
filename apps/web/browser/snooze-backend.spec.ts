// Real HTTP, authentication, SQLite snoozes and operation outcomes. No API mocks.
import { test, expect, type Locator, type Page } from "@playwright/test";

test.use({ timezoneId: "Europe/Tallinn" });
test.beforeEach(async ({ context }) => {
  await context.addCookies([{ name: "__Host-dakia", value: "release-browser-fixture", url: "https://127.0.0.1:4327/", secure: true, sameSite: "Lax" }]);
});
async function tabTo(page: Page, target: Locator) {
  for (let i=0; i<160; i++) {
    if (await target.evaluate(el => el === document.activeElement)) return;
    await page.keyboard.press("Tab");
  }
  throw new Error("Could not reach the snooze control with Tab");
}
for (const dark of [false, true]) test(`snooze and restore are confirmed, counted and persisted ${dark ? "dark" : "light"}`, async ({ page }, info) => {
  if (dark) await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: dark ? 390 : 1480, height: dark ? 600 : 980 });
  await page.addInitScript(dark => localStorage.setItem("dakia-theme", dark ? "dark" : "light"), dark);
  if (!dark) {
    const session = await (await page.request.get("/api/session")).json();
    await page.addInitScript(user => localStorage.setItem(`dakia:${user}:keyboard-hints`, "true"), session.user_id);
  }
  await page.goto("/");
  const targetSubject = dark ? "Synthetic 3" : "Synthetic 2";
  const row = page.locator(".message-row").filter({ has: page.locator(".subject", { hasText: targetSubject }) });
  await expect(row).toBeVisible();
  await tabTo(page, row.locator(".message-open")); await page.keyboard.press("b");
  const dialog = page.getByRole("dialog", { name: "Snooze until" });
  await expect(dialog).toBeVisible();
  await tabTo(page, dialog.getByRole("button", { name: /^Tomorrow/ })); await page.keyboard.press("Enter");
  await expect(page.getByRole("status").filter({ hasText: /Email snoozed until tomorrow at/ })).toBeVisible();
  if (dark) await expect(page.locator(".dakia-toast")).toHaveCSS("animation-name", "none");
  await page.screenshot({ animations: "disabled", path: info.outputPath("snooze-confirmation.png") });
  await expect(row).toHaveCount(0);
  if (dark) { await tabTo(page, page.getByRole("button", { name: "Open navigation", exact: true })); await page.keyboard.press("Enter"); }
  const snoozed = page.getByRole("button", { name: "Snoozed", exact: true });
  await expect(snoozed.locator(".count")).toHaveText("1");
  await tabTo(page, snoozed); await page.keyboard.press("Enter");
  await expect(row).toBeVisible();
  await expect(row.locator(".snooze-deadline")).toContainText(/tomorrow at .*9/);
  await page.screenshot({ animations: "disabled", path: info.outputPath("snoozed-with-deadline.png") });
  await page.reload();
  if (dark) { await tabTo(page, page.getByRole("button", { name: "Open navigation", exact: true })); await page.keyboard.press("Enter"); }
  await expect(snoozed.locator(".count")).toHaveText("1");
  await tabTo(page, snoozed); await page.keyboard.press("Enter");
  await expect(row).toBeVisible();
  await tabTo(page, row.locator(".message-open")); await page.keyboard.press("Enter");
  await expect(page.locator(".snooze-notice")).toContainText(/tomorrow at/);
  if (dark) await page.setViewportSize({ width: 360, height: 320 });
  const more = page.locator(".reader").getByRole("button", { name: "More actions", exact: true });
  await tabTo(page, more); await page.keyboard.press("Enter");
  await expect(page.getByRole("group", { name: "More message actions" }).getByRole("button", { name: "Snooze", exact: true })).toBeFocused();
  await page.screenshot({ animations: "disabled", path: info.outputPath("snoozed-more-actions.png") });
  await page.keyboard.press("Escape"); await expect(more).toBeFocused();
  const restore = page.locator(".reader").getByRole("button", { name: "Move back to Inbox", exact: true });
  const toolbar = page.locator(".reader > .reader-toolbar");
  for (const button of await toolbar.locator(":scope > button, .message-actions-more > button").all()) {
    await expect(button).toHaveClass(/dakia-mail-action/);
    await expect(button).toHaveCSS("font-weight", "400");
    await expect(button).toHaveCSS("font-size", "13px");
  }
  const archiveHint = toolbar.getByRole("button", { name: "Archive", exact: true }).locator(".key-hint");
  if (dark) await expect(archiveHint).toHaveCount(0);
  else await expect(archiveHint).toHaveText("E");
  await page.screenshot({ animations: "disabled", path: info.outputPath("restore-action.png") });
  await tabTo(page, restore); await page.keyboard.press("Enter");
  await expect(page.getByRole("status").filter({ hasText: "Email moved back to Inbox." })).toBeVisible();
  await expect(row).toHaveCount(0);
  if (dark) { await tabTo(page, page.getByRole("button", { name: "Open navigation", exact: true })); await page.keyboard.press("Enter"); }
  await expect(snoozed.locator(".count")).toHaveText("0");
  await tabTo(page, page.getByRole("navigation", { name: "Mailboxes" }).getByRole("button", { name: /^Unified inbox/ })); await page.keyboard.press("Enter");
  await expect(row).toBeVisible();
  await expect(row.locator(".snooze-deadline")).toHaveCount(0);
  await page.reload();
  await expect(row).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("bulk snooze and Inbox restoration update all rows, confirmations and counts", async ({ page }) => {
  await page.goto("/");
  const subjects = ["Synthetic 4", "Synthetic 5"];
  const row = (subject: string) => page.locator(".message-row").filter({ has: page.locator(".subject", { hasText: subject }) });
  for (const subject of subjects) { await tabTo(page, row(subject).locator(".message-open")); await page.keyboard.press("x"); }
  await page.keyboard.press("b");
  await tabTo(page, page.getByRole("dialog").getByRole("button", { name: /^Tomorrow/ })); await page.keyboard.press("Enter");
  await expect(page.getByRole("status").filter({ hasText: /2 emails snoozed until tomorrow/ })).toBeVisible();
  for (const subject of subjects) await expect(row(subject)).toHaveCount(0);
  const snoozed = page.getByRole("button", { name: "Snoozed", exact: true });
  await expect(snoozed.locator(".count")).toHaveText("2");
  await tabTo(page, snoozed); await page.keyboard.press("Enter");
  for (const subject of subjects) {
    await expect(row(subject).locator(".snooze-deadline")).toContainText("tomorrow at");
    await tabTo(page, row(subject).locator(".message-open")); await page.keyboard.press("x");
  }
  const restore = page.locator(".selection-actions").getByRole("button", { name: "Move back to Inbox", exact: true });
  await tabTo(page, restore); await page.keyboard.press("Enter");
  await expect(page.getByRole("status").filter({ hasText: "2 emails moved back to Inbox." })).toBeVisible();
  await expect(snoozed.locator(".count")).toHaveText("0");
  await tabTo(page, page.getByRole("navigation", { name: "Mailboxes" }).getByRole("button", { name: /^Unified inbox/ })); await page.keyboard.press("Enter");
  for (const subject of subjects) await expect(row(subject)).toBeVisible();
  await page.reload();
  for (const subject of subjects) await expect(row(subject)).toBeVisible();
});

test("a stale snooze is rejected without a success toast, disappearance or inflated count", async ({ page }) => {
  await page.goto("/");
  const row = page.locator(".message-row").filter({ has: page.locator(".subject", { hasText: "Synthetic 6" }) });
  await tabTo(page, row.locator(".message-open")); await page.keyboard.press("b");
  // A separate client changes the revision after this picker captured its target.
  const accounts = await (await page.request.get("/api/accounts")).json();
  const rows = await (await page.request.get(`/api/accounts/${accounts[0].id}/messages?folder=INBOX`)).json();
  const target = rows.rows.find((m: { subject: string }) => m.subject === "Synthetic 6");
  const response = await page.request.post(`/api/accounts/${target.account_id}/operations`, { headers: { "x-csrf-token": "release-browser-csrf", origin: "http://127.0.0.1:4327" }, data: {
    id: crypto.randomUUID(), account_id: target.account_id, target_id: target.id, expected_revision: target.revision,
    change: { kind: "category", value: "people" }, status: "pending", revision: 0,
  }});
  expect(response.status()).toBe(200);
  await tabTo(page, page.getByRole("dialog").getByRole("button", { name: /^Tomorrow/ })); await page.keyboard.press("Enter");
  await expect(page.getByRole("alert").first()).toBeVisible();
  await expect(row).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: /Email snoozed until/ })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Snoozed", exact: true }).locator(".count")).toHaveText("0");
});

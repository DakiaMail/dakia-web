import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { accounts, content, page as mail } from "./fixtures";

const subject = "We've updated our Privacy Policy and Terms and Conditions";
const html = `<div style="background:#f5f5f8;padding:18px;border-radius:18px"><div style="background:white;padding:22px;border-radius:18px"><h1 style="font-size:24px;margin-bottom:48px">EXAMPLE FITNESS</h1><p>Hi,</p><p>We've updated our Terms &amp; Conditions and Privacy Policy. Both will take effect <b>on November 30, 2026.</b></p><p>This is a fictional newsletter for reviewing the email reader. Your message layout and its authored spacing stay unchanged.</p><p>${"Our policy explains how contact discovery, location and analytics work. ".repeat(8)}</p></div></div>`;
async function fixture(page: Page, options: { long?: boolean; failure?: "expired" | "provider" | "snapshot" } = {}) {
  const root = { ...mail.rows[0], unread: false, subject: options.long ? `${subject} ${"LongSubject".repeat(14)}` : subject,
    sender: { name: options.long ? "A very long fitness newsletter sender name that must never overlap the date" : "Example Fitness Team", email: `${options.long ? "longsender".repeat(10) : "newsletter"}@example.invalid` },
    to: [{ email: "alex@example.com" }], received_at: "2026-10-31T16:02:00Z" };
  const rows = [root, ...mail.rows.slice(1).map(m => ({ ...m, unread: false }))];
  const state = { starts: 0, expired: 0, provider: 0, snapshots: 0, recover: false, writes: [] as any[] };
  await page.route("**/api/**", route => {
    const url = new URL(route.request().url()), path = url.pathname;
    const respond = (json: unknown, status = 200) => route.fulfill({ json, status });
    if (path === "/api/session") return respond({ user_id: "chrome-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows });
    if (path.endsWith("/conversation")) {
      if (url.searchParams.has("snapshot")) {
        state.snapshots++;
        if (options.failure === "snapshot" && !state.recover) return respond({ code: "unavailable" }, 503);
        return respond({ rows: [root], complete: true, continuation: null });
      }
      const token = url.searchParams.get("continuation");
      if (!token) {
        state.starts++;
        return respond({ rows: [root], complete: !options.failure || options.failure === "snapshot", continuation: options.failure && options.failure !== "snapshot" ? (options.failure === "expired" && !state.recover ? "expired" : "fresh") : null });
      }
      if (options.failure === "expired" && token === "expired") { state.expired++; return respond({ code: "conversation_refresh_required" }, 409); }
      state.provider++;
      if (options.failure === "provider" && !state.recover) return respond({ code: "provider_unavailable" }, 502);
      return respond({ rows: [root], complete: true, continuation: null });
    }
    if (path.endsWith("/unsubscribe")) return respond(null);
    if (path.includes("/messages/")) return respond({ ...content, message: root, html, detected_language: "en", attachments: [] });
    if (path.endsWith("/operations")) { const op = route.request().postDataJSON(); state.writes.push(op); return respond({ ...op, status: "applied", revision: op.expected_revision + 1 }); }
    return respond([]);
  });
  // These app screenshots never access an account or a remote image.
  await page.route(/^https:\/\//, route => route.abort());
  await page.goto("/");
  return state;
}
async function open(page: Page) {
  await page.locator(".message-open").filter({ hasText: subject }).click();
  await expect(page.frameLocator('iframe[title="Message body"]').getByText("EXAMPLE FITNESS", { exact: true })).toBeVisible();
}
async function screenshot(page: Page, name: string) {
  mkdirSync("artifacts/reader-chrome", { recursive: true });
  await page.screenshot({ path: `artifacts/reader-chrome/${name}.png` });
}
for (const width of [320, 390, 600, 1480]) for (const long of [false, true]) test(`reader chrome ${width} long=${long}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 844 });
  const state = await fixture(page, { long });
  await expect(page.getByRole("button", { name: "Compose", exact: true }).first()).toBeVisible();
  await open(page);
  const reader = page.getByRole("region", { name: "Reading pane" });
  const heading = reader.getByRole("heading", { level: 2 });
  await expect(heading).toContainText(subject);
  if (width <= 600) {
    await expect(reader.getByRole("button", { name: "Compose", exact: true })).toBeHidden();
    await expect(reader.getByRole("button", { name: "Check for more messages", exact: true })).toBeHidden();
    expect((await heading.boundingBox())!.x).toBe(16);
    expect((await reader.locator(".conversation-summary").evaluate(el => getComputedStyle(el).paddingLeft))).toBe("16px");
    expect(await page.frameLocator('iframe[title="Message body"]').locator("body").evaluate(el => getComputedStyle(el).paddingLeft)).toBe("12px");
    const bar = reader.locator(".reader-toolbar");
    expect((await bar.boundingBox())!.y).toBeGreaterThan(760);
    await bar.getByRole("button", { name: "More actions", exact: true }).click();
    await expect(page.getByRole("menu", { name: "More message actions" })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(bar.getByRole("button", { name: "More actions", exact: true })).toBeFocused();
  } else {
    await expect(reader.getByRole("button", { name: "Compose", exact: true })).toBeVisible();
    await expect(reader.getByRole("button", { name: "Check for more messages", exact: true })).toBeVisible();
  }
  expect(await page.evaluate(() => document.documentElement.scrollWidth - innerWidth)).toBeLessThanOrEqual(1);
  await screenshot(page, `reader-${width}${long ? "-long" : ""}`);
  if (width <= 600) {
    const last = page.frameLocator('iframe[title="Message body"]').locator("p").last();
    await last.evaluate(el => el.scrollIntoView({ block: "center" }));
    const bottom = (await last.boundingBox())!;
    expect(bottom.y + bottom.height).toBeLessThanOrEqual((await reader.locator(".reader-toolbar").boundingBox())!.y);
    await screenshot(page, `reader-end-${width}${long ? "-long" : ""}`);
  }
  expect(state.writes).toEqual([]);
  if (width <= 600) {
    await page.locator(".conversation").evaluate(el => { el.scrollTop = el.scrollHeight; });
    await reader.getByRole("button", { name: "Reply", exact: true }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
  }
});
for (const failure of ["expired", "provider", "snapshot"] as const) test(`conversation retry recovers ${failure} without mail writes`, async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const state = await fixture(page, { failure });
  await open(page);
  await expect(page.getByText("Couldn’t check for more messages.", { exact: true })).toBeVisible();
  await screenshot(page, `conversation-${failure}-failure`);
  const starts = state.starts, provider = state.provider, expired = state.expired;
  state.recover = true;
  await page.getByRole("button", { name: "Retry conversation", exact: true }).click();
  await expect(page.getByText("Couldn’t check for more messages.", { exact: true })).toHaveCount(0);
  if (failure === "expired") { expect(state.starts).toBe(starts + 1); expect(state.expired).toBeGreaterThanOrEqual(1); expect(state.expired).toBe(expired); }
  if (failure === "provider") { expect(state.starts).toBe(starts); expect(state.provider).toBe(provider + 1); }
  if (failure === "snapshot") { expect(state.starts).toBe(starts); expect(state.provider).toBe(provider); }
  expect(state.writes).toEqual([]);
  await expect(page.locator(".conversation-message")).toHaveCount(1);
});
for (const action of ["Archive", "Move to trash", "Star selected messages"] as const) test(`mobile selection applies ${action} to exact targets`, async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  const state = await fixture(page);
  await page.getByRole("tab", { name: "List", exact: true }).click();
  const checks = page.locator(".message-select input");
  await checks.nth(0).check(); await checks.nth(1).check();
  const bar = page.getByRole("group", { name: "Selected message actions", exact: true });
  if (action === "Star selected messages") await bar.getByRole("button", { name: "More actions", exact: true }).click();
  await page.getByRole(action === "Star selected messages" ? "menuitem" : "button", { name: action, exact: true }).click();
  await expect.poll(() => state.writes.length).toBe(2);
  expect(state.writes.map(op => op.target_id).sort()).toEqual(["m0", "m1"]);
  expect(state.writes.map(op => op.change.kind)).toEqual(Array(2).fill(action === "Archive" ? "archive" : action === "Move to trash" ? "trash" : "star"));
});
for (const width of [320, 390, 600, 1480]) test(`selection actions ${width}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 844 });
  const state = await fixture(page);
  await page.getByRole("tab", { name: "List", exact: true }).click();
  const checks = page.locator(".message-select input");
  await checks.nth(0).check(); await checks.nth(1).check();
  const bar = page.getByRole("group", { name: "Selected message actions", exact: true });
  await expect(bar).toBeVisible();
  const clear = page.getByRole("button", { name: width <= 600 ? "Clear selection (2 selected)" : "2 selected · Clear", exact: true }).filter({ visible: true });
  await expect(clear).toBeVisible();
  if (width <= 600) {
    await page.locator(".app").evaluate(el => (el as HTMLElement).style.setProperty("--safe-bottom", "24px"));
    await expect(page.getByRole("button", { name: "Compose", exact: true })).toBeHidden();
    const box = (await bar.boundingBox())!;
    expect(box.x).toBe(12); expect(box.width).toBe(width - 24);
    expect(box.y + box.height).toBe(844 - 36);
    expect((await clear.boundingBox())!.height).toBeGreaterThanOrEqual(44);
    expect((await clear.boundingBox())!.width).toBeGreaterThanOrEqual(44);
    for (const name of ["Archive", "Mark unread", "Move to trash", "More actions"]) {
      const button = (await bar.getByRole("button", { name, exact: true }).boundingBox())!;
      expect(button.width).toBeGreaterThanOrEqual(44); expect(button.height).toBeGreaterThanOrEqual(44);
      expect(button.x).toBeGreaterThanOrEqual(box.x); expect(button.x + button.width).toBeLessThanOrEqual(box.x + box.width);
    }
    await screenshot(page, `selection-${width}`);
    const lastRow = page.locator(".message-row").last();
    await lastRow.scrollIntoViewIfNeeded();
    const end = (await lastRow.boundingBox())!;
    expect(end.y + end.height).toBeLessThanOrEqual(box.y);
    await screenshot(page, `selection-end-${width}`);
    await bar.getByRole("button", { name: "More actions", exact: true }).click();
    const menu = page.getByRole("menu", { name: "More message actions" });
    await expect(menu).toBeVisible();
    expect((await menu.boundingBox())!.y + (await menu.boundingBox())!.height).toBeLessThan((await bar.getByRole("button", { name: "More actions", exact: true }).boundingBox())!.y);
    await screenshot(page, `selection-${width}-more`);
    await page.keyboard.press("Escape");
    await expect(bar.getByRole("button", { name: "More actions", exact: true })).toBeFocused();
  } else expect((await bar.boundingBox())!.y).toBeLessThan(200);
  await clear.click();
  await expect(page.locator(".message-select input:checked")).toHaveCount(0);
  await expect(bar).toHaveCount(0);
  await checks.nth(0).check(); await checks.nth(1).check();
  await bar.getByRole("button", { name: "Mark unread", exact: true }).click();
  await expect.poll(() => state.writes.length).toBe(2);
  expect(state.writes.map(op => op.target_id).sort()).toEqual(["m0", "m1"]);
  expect(state.writes.every(op => op.change.kind === "read" && op.change.value === false)).toBe(true);
});

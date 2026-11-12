import { test, expect, type Page } from "@playwright/test";
import type { Operation } from "@dakia/api-types";
import { accounts, content, page as mail } from "./fixtures";

async function fixture(page: Page, failed = false) {
  const rows = [0, 1, 2].map(i => ({ ...mail.rows[0], id: `notice-${i}`, thread_id: `notice-${i}`,
    subject: `Notice ${i}`, account_id: "work", category: "Notifications", unread: true }));
  const operations: Operation[] = [];
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    const respond = (value: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "read-section-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond([accounts[0]]);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/operations")) {
      const op: Operation = { ...route.request().postDataJSON(), status: failed ? "failed" : "applied", revision: 2 };
      operations.push(op);
      const row = rows.find(row => row.id === op.target_id);
      if (row && !failed && op.change.kind === "read") row.unread = !op.change.value;
      return respond(op);
    }
    if (path.includes("/operations/")) return respond(operations.find(op => path.endsWith(op.id)));
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows, continuation: null });
    const row = rows.find(row => path.includes(row.id)) || rows[0];
    if (path.endsWith("/conversation")) return respond({ rows: [row], complete: true, continuation: null });
    if (path.endsWith("/unsubscribe")) return respond(null);
    if (path.includes("/messages/")) return respond({ ...content, message: row });
    return respond([]);
  });
  await page.goto("/");
  await expect(page.locator(".message-row")).toHaveCount(3);
  return operations;
}
const notice = (page: Page, i: number) => page.locator(".message-row").filter({ hasText: `Notice ${i}` });
for (const width of [390, 1480]) test(`reading keeps mail in place without Seen groups at ${width}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 844 });
  const operations = await fixture(page);
  const order = await page.locator(".message-row").evaluateAll(rows => rows.map(row => row.getAttribute("data-message-id") || row.textContent));
  await notice(page, 0).locator(".message-open").click();
  await expect.poll(() => operations.length).toBe(1);
  expect(operations[0].change).toEqual({ kind: "read", value: true });
  if (width === 390) { await page.goBack(); await expect(page.locator(".app")).not.toHaveClass(/reading/); }
  await expect(notice(page, 0)).not.toHaveClass(/unread/);
  await notice(page, 1).locator(".message-open").click();
  if (width === 390) { await page.goBack(); await expect(page.locator(".app")).not.toHaveClass(/reading/); }
  await expect(notice(page, 0)).toBeVisible();
  await expect(page.locator(".message-row")).toHaveCount(3);
  await expect(page.locator(".group-heading")).toHaveCount(0);
  await expect(notice(page, 0)).not.toHaveClass(/leaving-section/);
  expect(await page.locator(".message-row").evaluateAll(rows => rows.map(row => row.getAttribute("data-message-id") || row.textContent))).toEqual(order);
});

test("failed reads remain unread and visible", async ({ page }) => {
  await fixture(page, true);
  await notice(page, 0).locator(".message-open").click();
  await expect(page.getByRole("status").filter({ hasText: "Still marked unread" })).toBeVisible();
  await notice(page, 1).locator(".message-open").click();
  await expect(notice(page, 0)).toHaveClass(/unread/);
  await expect(page.locator(".message-row")).toHaveCount(3);
});

test("switching tabs preserves the selected reader after reading", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await fixture(page);
  await notice(page, 0).locator(".message-open").click();
  await expect(notice(page, 0)).not.toHaveClass(/unread/);
  await page.getByRole("tab", { name: "All", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Notice 0", exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "Personal", exact: true }).click();
  await expect(notice(page, 0)).toHaveClass(/selected/);
  await expect(page.locator(".message-row")).toHaveCount(3);
});

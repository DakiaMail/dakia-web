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
const section = (page: Page, label: string) => page.getByRole("button", { name: new RegExp(`^${label} ·`) });

test("read mail stays in Notifications until another email opens, then animates into Seen", async ({ page }) => {
  const operations = await fixture(page);
  const first = notice(page, 0);
  await first.locator(".message-open").click();
  await expect.poll(() => operations.length).toBe(1);
  expect(operations[0].change).toEqual({ kind: "read", value: true });
  await expect(first).not.toHaveClass(/unread/);
  await expect(section(page, "notifications")).toHaveAttribute("aria-label", "notifications · 3 loaded messages");
  await expect(section(page, "seen")).toHaveCount(0);
  const position = await first.boundingBox();
  await page.waitForTimeout(350);
  expect(await first.boundingBox()).toEqual(position);
  await notice(page, 1).locator(".message-open").click();
  await expect(first).toHaveClass(/leaving-section/);
  await expect(first).toHaveCSS("animation-name", "read-section-exit");
  await expect(section(page, "notifications")).toHaveAttribute("aria-label", "notifications · 2 loaded messages");
  await expect(section(page, "seen")).toHaveAttribute("aria-label", "seen · 1 loaded messages");
  await expect(first).not.toHaveClass(/leaving-section/);
  await expect(page.locator(".message-row")).toHaveCount(3);
});

test("mobile returning to the list holds the read row until opening another message", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await fixture(page);
  await notice(page, 0).locator(".message-open").click();
  await expect(page.locator(".app")).toHaveClass(/reading/);
  await page.goBack();
  await expect(page.locator(".app")).not.toHaveClass(/reading/);
  await expect(notice(page, 0)).toBeVisible();
  await expect(section(page, "seen")).toHaveCount(0);
  await notice(page, 1).locator(".message-open").click();
  await page.goBack();
  await expect(section(page, "seen")).toHaveAttribute("aria-label", "seen · 1 loaded messages");
});

test("failed reads remain unread in their category", async ({ page }) => {
  await fixture(page, true);
  await notice(page, 0).locator(".message-open").click();
  await expect(page.getByRole("status").filter({ hasText: "Still marked unread" })).toBeVisible();
  await notice(page, 1).locator(".message-open").click();
  await expect(notice(page, 0)).toHaveClass(/unread/);
  await expect(section(page, "notifications")).toHaveAttribute("aria-label", "notifications · 3 loaded messages");
  await expect(section(page, "seen")).toHaveCount(0);
});

test("reduced motion disables the departure animation", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await fixture(page);
  await notice(page, 0).locator(".message-open").click();
  await notice(page, 1).locator(".message-open").click();
  await expect(notice(page, 0)).toHaveClass(/leaving-section/);
  await expect(notice(page, 0)).toHaveCSS("animation-name", "none");
  await expect(section(page, "seen")).toBeVisible();
});

test("reopening a departing email cancels its removal", async ({ page }) => {
  await fixture(page);
  await notice(page, 0).locator(".message-open").click();
  await notice(page, 1).locator(".message-open").click();
  await expect(notice(page, 0)).toHaveClass(/leaving-section/);
  // Keyboard activation also covers returning before the departure completes.
  await notice(page, 0).locator(".message-open").press("Enter");
  await expect(notice(page, 0)).not.toHaveClass(/leaving-section/);
  await page.waitForTimeout(350);
  await expect(section(page, "notifications")).toHaveAttribute("aria-label", "notifications · 2 loaded messages");
  await expect(notice(page, 0)).toHaveClass(/selected/);
  await expect(page.locator(".message-row")).toHaveCount(3);
});

test("switching views keeps the selected mail without duplicates", async ({ page }) => {
  await fixture(page);
  await notice(page, 0).locator(".message-open").click();
  await expect(notice(page, 0)).not.toHaveClass(/unread/);
  await page.getByRole("tab", { name: "List", exact: true }).click();
  await expect(section(page, "notifications")).toHaveCount(0);
  await page.getByRole("tab", { name: "Smart", exact: true }).click();
  await expect(notice(page, 0)).toHaveClass(/selected/);
  await expect(page.locator(".message-row")).toHaveCount(3);
});

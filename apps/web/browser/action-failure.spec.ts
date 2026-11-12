import { test, expect } from "@playwright/test";
import { accounts, content, page as mail } from "./fixtures";
import type { Operation } from "@dakia/api-types";

for (const width of [390, 1480]) test(`opening unread mail identifies failed automatic read action ${width}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 980 });
  const message = { ...mail.rows[0], unread: true, subject: "Synthetic newsletter", revision: 1 };
  const operations = new Map<string, Operation>();
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    const respond = (value: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "failure-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/operations")) {
      const op = { ...route.request().postDataJSON(), status: "pending", revision: 2 } as Operation;
      operations.set(op.id, op); return respond(op);
    }
    if (path.includes("/operations/")) return respond({ ...operations.get(path.split("/").at(-1)!), status: "failed", revision: 3 });
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows: [message], continuation: null });
    if (path.endsWith("/conversation")) return respond({ rows: [message], complete: true, continuation: null });
    if (path.endsWith("/unsubscribe")) return respond({ kind: "unavailable" });
    if (path.includes("/messages/")) return respond({ ...content, message });
    return respond([]);
  });
  await page.goto("/");
  await page.getByRole("tab", { name: "All", exact: true }).click();
  await page.locator(".message-open").filter({ hasText: message.subject }).click();
  await expect(page.getByRole("status").filter({ hasText: "Still marked unread" })).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect([...operations.values()].map(op => op.change)).toEqual([{ kind: "read", value: true }]);
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  await page.screenshot({ path: `artifacts/action-failure-${width}.png`, fullPage: true });
  await page.keyboard.press("Escape");
  await page.locator(".message-open").filter({ hasText: message.subject }).click();
  await expect(page.getByRole("status").filter({ hasText: "Still marked unread" })).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(operations.size).toBe(1);
});

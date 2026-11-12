import { test, expect, type Page } from "@playwright/test";
import type { Operation } from "@dakia/api-types";
import { accounts, content, page as mail } from "./fixtures";

const message = { ...mail.rows[0], unread: false, subject: "Release action fixture" };
async function setup(page: Page, failure: "rejected" | "failed") {
  const writes: Operation[] = [], reads: string[] = [], unexpected: string[] = [];
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    if (path === "/api/session") return route.fulfill({ json: { user_id: "release-fixture", csrf_token: "fixture-csrf" } });
    if (path === "/api/accounts") return route.fulfill({ json: accounts });
    if (route.request().method() === "GET" && path === "/api/local-access") return route.fulfill({ json: null });
    if (route.request().method() === "GET" && path === "/api/translation/settings") return route.fulfill({ json: { mode: "host", can_manage: false } });
    if (route.request().method() === "GET" && (path === "/api/rules" || path.endsWith("/snoozes"))) return route.fulfill({ json: [] });
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": fixture\n\n" });
    if (path.endsWith("/sync-active")) return route.fulfill({ json: false });
    if (path.endsWith("/operations")) {
      expect(route.request().headers()["x-csrf-token"]).toBe("fixture-csrf");
      const op = route.request().postDataJSON() as Operation; writes.push(op);
      return failure === "rejected"
        ? route.fulfill({ status: 409, json: { code: "conflict", message: "Mail changed. Refresh and try again." } })
        : route.fulfill({ json: { ...op, status: "pending", revision: 2 } });
    }
    if (path.includes("/operations/")) {
      reads.push(path);
      const op = writes.find(op => path.endsWith(`/${op.id}`));
      expect(op).toBeTruthy();
      return route.fulfill({ json: { ...op, status: "failed", revision: 3 } });
    }
    if (path === "/api/search" || path.endsWith("/messages")) return route.fulfill({ json: { ...mail, rows: [message], continuation: null, pending: false } });
    if (path.endsWith("/conversation")) return route.fulfill({ json: { rows: [message], complete: true, continuation: null } });
    if (path.endsWith("/unsubscribe")) return route.fulfill({ json: { kind: "unavailable" } });
    if (path.endsWith(`/messages/${message.id}`)) return route.fulfill({ json: { ...content, message, attachments: [] } });
    if (path.endsWith("/sender-cleanups") || path.endsWith("/scheduled") || path.endsWith("/submissions") || path.endsWith("/drafts")) return route.fulfill({ json: [] });
    unexpected.push(`${route.request().method()} ${path}`);
    return route.fulfill({ status: 500, json: { code: "unexpected_fixture_request", message: path } });
  });
  await page.goto("/");
  await page.getByRole("tab", { name: "All", exact: true }).click();
  await page.locator(".message-open").filter({ hasText: message.subject }).click();
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  return { writes, reads, unexpected };
}

for (const width of [390, 1480]) {
  for (const [label, kind] of [["Archive", "archive"], ["Move to trash", "trash"], ["Mark as spam", "spam"]]) {
    for (const failure of ["rejected", "failed"] as const) {
      test(`${label} ${failure} is visible and never replays at ${width}px`, async ({ page }) => {
        await page.setViewportSize({ width, height: 980 });
        const observed = await setup(page, failure);
        const reader = page.getByRole("region", { name: "Reading pane" });
        if (kind === "spam") await reader.getByRole("button", { name: "More actions" }).click();
        await page.getByRole(kind === "spam" ? "menuitem" : "button", { name: label, exact: true }).click();
        await expect(page.getByRole("alert")).toContainText(failure === "rejected" ? "Refresh and try again" : "could not be verified");
        expect(observed.writes).toHaveLength(1);
        expect(observed.writes[0]).toMatchObject({ account_id: message.account_id, target_id: message.id, change: { kind } });
        expect(observed.reads).toHaveLength(failure === "failed" ? 1 : 0);
        await page.getByRole("button", { name: "Dismiss mail action error" }).click();
        await expect(page.getByRole("alert")).toHaveCount(0);
        // Moving the clock across multiple retry intervals must not send mail actions again.
        await page.clock.install();
        await page.clock.runFor(10_000);
        expect(observed.writes).toHaveLength(1);
        expect(observed.unexpected).toEqual([]);
        await expect(page.locator(".message-open").filter({ hasText: message.subject })).toBeVisible();
      });
    }
  }
}

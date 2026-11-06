import { test, expect } from "@playwright/test";
import { accounts, content, page as mail } from "./fixtures";

// Reproduce the combined failure shown in the owner report using synthetic mail.
// Background failures stay secondary to readable content and never replay writes.
for (const width of [390, 1480]) test(`opening mail remains readable during simultaneous background failures at ${width}`, async ({ page }, testInfo) => {
  await page.setViewportSize({ width, height: 980 });
  const message = { ...mail.rows[0], unread: true, subject: "Synthetic combined failure" };
  const operations: Record<string, unknown>[] = [];
  const calls = { conversation: 0, image: 0, content: 0 };
  const unexpected: string[] = [];
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    const respond = (json: unknown) => route.fulfill({ json });
    if (path === "/api/session") return respond({ user_id: "fixture", csrf_token: "fixture" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/sync-active")) return respond(false);
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows: [message], pending: false, continuation: null });
    if (path.endsWith("/operations")) {
      const op = route.request().postDataJSON(); operations.push(op);
      return respond({ ...op, status: "pending", revision: 2 });
    }
    if (path.includes("/operations/")) return respond({ ...operations[0], status: "failed", revision: 3 });
    if (path.endsWith("/conversation")) {
      calls.conversation++;
      return route.fulfill({ status: 502, json: { code: "provider_unavailable", message: "Fixture provider unavailable" } });
    }
    if (path.endsWith(`/messages/${message.id}`)) {
      calls.content++;
      return respond({ ...content, message, attachments: [], detected_language: "en", html: '<h1>Your saved message is readable</h1><p>Synthetic content only.</p><img src="https://assets.example.invalid/one.png"><img src="https://assets.example.invalid/two.png">' });
    }
    if (path.endsWith("/image")) {
      calls.image++;
      return route.fulfill({ status: 502, json: { code: "remote_image_unavailable", message: "Fixture image unavailable" } });
    }
    if (path.endsWith("/unsubscribe")) return respond({ kind: "unavailable" });
    if (["/drafts", "/scheduled", "/submissions", "/sender-cleanups"].some(suffix => path.endsWith(suffix))) return respond([]);
    unexpected.push(path);
    return route.fulfill({ status: 500, json: { code: "unexpected_fixture_request", message: path } });
  });
  await page.goto("/");
  await page.getByRole("tab", { name: "List", exact: true }).click();
  await page.locator(".message-open").filter({ hasText: message.subject }).click();
  const body = page.frameLocator('iframe[title="Message body"]').locator("body");
  await expect(body).toContainText("Your saved message is readable");
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByRole("status").filter({ hasText: "Still marked unread" })).toBeVisible();
  await expect(page.locator(".reader-status").filter({ hasText: "Couldn’t check" })).toContainText("Couldn’t check for more messages");
  await expect(page.locator("details summary")).toHaveText("2 images could not be loaded.");
  await expect(page.locator("details")).not.toHaveAttribute("open", "");
  expect(operations.map(op => op.change)).toEqual([{ kind: "read", value: true }]);
  expect(calls).toEqual({ conversation: 2, image: 2, content: 1 });
  await page.screenshot({ path: testInfo.outputPath("combined-reader-failures.png"), fullPage: true });
  await page.clock.install();
  await page.clock.runFor(120_000);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(body).toContainText("Your saved message is readable");
  expect(operations).toHaveLength(1);
  expect(calls).toEqual({ conversation: 2, image: 2, content: 1 });
  await page.getByRole("button", { name: "Retry loading images" }).click();
  await expect.poll(() => calls.image).toBe(4);
  expect(operations).toHaveLength(1);
  expect(unexpected).toEqual([]);
});

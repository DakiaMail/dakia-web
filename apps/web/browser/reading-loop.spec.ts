import { test, expect } from "@playwright/test";
import { accounts, content, messages, page as mail } from "./fixtures";

test("measures the keyboard reading loop with delayed bodies", async ({ page }, info) => {
  const rows = messages.slice(0, 4).map(m => ({ ...m, unread: false }));
  const bodies: string[] = [];
  const operations = new Map<string, unknown>();
  await page.addInitScript(() => {
    (window as unknown as { readingLongTasks: number[] }).readingLongTasks = [];
    new PerformanceObserver(list => {
      for (const entry of list.getEntries()) (window as unknown as { readingLongTasks: number[] }).readingLongTasks.push(entry.duration);
    }).observe({ type: "longtask", buffered: true });
  });
  await page.route("**/api/**", async route => {
    const p = new URL(route.request().url()).pathname;
    const respond = (value: unknown) => route.fulfill({ json: value });
    if (p === "/api/session") return respond({ user_id: "fixture", csrf_token: "fixture" });
    if (p === "/api/accounts") return respond(accounts);
    if (p.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (p === "/api/search" || p.endsWith("/messages")) return respond({ ...mail, rows });
    if (p.includes("/drafts/") && route.request().method() === "PUT") return respond({ ...route.request().postDataJSON(), id: "draft-fixture", revision: 1 });
    if (p.endsWith("/operations")) {
      const op = { ...route.request().postDataJSON(), status: "pending", revision: 2 };
      operations.set(op.id, op); return respond(op);
    }
    if (p.includes("/operations/")) return respond(operations.get(p.split("/").at(-1)!));
    const m = rows.find(m => p.includes(`/messages/${m.id}`));
    if (p.endsWith("/conversation")) return respond({ rows: [m], complete: true, continuation: null });
    if (p.endsWith("/unsubscribe")) return respond(null);
    if (m) {
      bodies.push(m.id);
      await new Promise(resolve => setTimeout(resolve, 180));
      return respond({ ...content, message: m, html: `<p>Reading body ${m.id}</p>`, text: `Reading body ${m.id}`, attachments: [] });
    }
    if (p.endsWith("/drafts") || p.endsWith("/scheduled") || p.endsWith("/submissions") || p.endsWith("/recipients")) return respond([]);
    return respond(null);
  });
  await page.goto("/");
  await expect(page.locator(".message-open").first()).toBeVisible();
  const samples: { action: string; ms: number }[] = [];
  for (const m of rows) {
    const button = page.locator(`[data-message-id="${m.account_id}/${m.id}"] .message-open`);
    await button.focus();
    await page.waitForTimeout(450); // Deliberate reading/focus dwell, identical for baseline and candidate.
    const start = performance.now();
    await page.keyboard.press("Enter");
    await expect(page.frameLocator(".conversation iframe").locator("body")).toContainText(`Reading body ${m.id}`);
    samples.push({ action: "open", ms: performance.now() - start });
    const replyStart = performance.now();
    await page.getByRole("button", { name: "Reply", exact: true }).focus();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("dialog")).toBeVisible();
    samples.push({ action: "reply", ms: performance.now() - replyStart });
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.locator(".reader").focus();
    const backStart = performance.now();
    await page.keyboard.press("Escape");
    await expect(page.locator(".message-list")).toBeFocused();
    samples.push({ action: "return", ms: performance.now() - backStart });
  }
  await page.locator(".message-open").first().focus();
  await page.keyboard.press("Enter");
  await page.locator(".reader").focus();
  const nextStart = performance.now();
  await page.keyboard.press("j");
  await expect(page.locator(".conversation-title h2")).toHaveText(rows[1].subject);
  await expect(page.frameLocator(".conversation iframe").locator("body")).toContainText(`Reading body ${rows[1].id}`);
  samples.push({ action: "next", ms: performance.now() - nextStart });
  const archiveStart = performance.now();
  await page.keyboard.press("e");
  await expect(page.locator(".message-list")).toBeFocused();
  await expect(page.locator(`[data-message-id="${rows[1].account_id}/${rows[1].id}"]`)).toHaveCount(0);
  samples.push({ action: "archive", ms: performance.now() - archiveStart });
  const longTasks = await page.evaluate(() => (window as unknown as { readingLongTasks: number[] }).readingLongTasks);
  await info.attach("reading-loop.json", { body: JSON.stringify({ samples, bodies, longTasks }, null, 2), contentType: "application/json" });
  console.log(JSON.stringify({ samples, bodies, longTasks }));
});

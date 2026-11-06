import { test, expect } from "@playwright/test";
import { accounts, content, messages, page as mail } from "./fixtures";

test("focus warms bodies without mail actions, images, or discovery; open reuses it and warms only the next row", async ({ page }) => {
  const rows = messages.slice(0, 5).map(m => ({ ...m, unread: false }));
  const bodies: string[] = [], other: string[] = [];
  await page.route("**/api/**", async route => {
    const p = new URL(route.request().url()).pathname;
    const respond = (value: unknown) => route.fulfill({ json: value });
    if (p === "/api/session") return respond({ user_id: "fixture", csrf_token: "fixture" });
    if (p === "/api/accounts") return respond(accounts);
    if (p.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (p === "/api/search") return respond({ ...mail, rows });
    const m = rows.find(m => p.includes(`/messages/${m.id}`));
    if (m && p.endsWith(`/messages/${m.id}`)) {
      bodies.push(m.id);
      return respond({ ...content, message: m, html: '<p>Prefetched mail</p><img src="https://tracker.invalid/pixel">', attachments: [] });
    }
    if (p.endsWith("/conversation")) { other.push("conversation"); return respond({ rows: [m], complete: true, continuation: null }); }
    if (p.endsWith("/unsubscribe")) return respond(null);
    if (p.includes("/images") || route.request().method() !== "GET") other.push(p);
    return respond([]);
  });
  await page.goto("/");
  const first = page.locator(`[data-message-id="${rows[0].account_id}/${rows[0].id}"] .message-open`);
  await first.focus();
  await expect.poll(() => bodies).toEqual([rows[0].id]);
  expect(other).toEqual([]);
  await page.waitForTimeout(300);
  expect(bodies).toHaveLength(1);
  await page.keyboard.press("Enter");
  await expect(page.locator(".conversation iframe")).toBeVisible();
  await expect.poll(() => bodies.length).toBe(2);
  expect(bodies).toEqual([rows[0].id, rows[1].id]);
  await page.waitForTimeout(350);
  expect(bodies).toHaveLength(2);
});

test("rapid focus changes discard old intent and offline navigation stays local", async ({ page }) => {
  const rows = messages.slice(0, 3).map(m => ({ ...m, unread: false }));
  const bodies: string[] = [];
  await page.route("**/api/**", async route => {
    const p = new URL(route.request().url()).pathname;
    const respond = (value: unknown) => route.fulfill({ json: value });
    if (p === "/api/session") return respond({ user_id: "fixture", csrf_token: "fixture" });
    if (p === "/api/accounts") return respond(accounts);
    if (p === "/api/search") return respond({ ...mail, rows });
    if (p.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    const m = rows.find(m => p.endsWith(`/messages/${m.id}`));
    if (m) { bodies.push(m.id); return respond({ ...content, message: m }); }
    return respond([]);
  });
  await page.goto("/");
  await page.locator(".message-open").first().focus();
  await page.keyboard.press("Tab");
  await page.waitForTimeout(250);
  expect(bodies).toEqual([]);
  await page.context().setOffline(true);
  await page.locator(".message-open").nth(1).focus();
  await page.waitForTimeout(250);
  expect(bodies).toEqual([]);
});

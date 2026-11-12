import { test, expect } from "@playwright/test";
import { accounts, content, messages, page as mail } from "./fixtures";

test("warms every visible row without images, mail actions, discovery or overscan; scrolling replaces the queue", async ({ page }) => {
  const rows = Array.from({ length: 60 }, (_, i) => ({ ...messages[0], id: `viewport-${i}`, subject: `Viewport ${i}`, unread: false }));
  const bodies: string[] = [], other: string[] = [];
  await page.route("**/api/**", async route => {
    const p = new URL(route.request().url()).pathname;
    const respond = (value: unknown) => route.fulfill({ json: value });
    if (p === "/api/session") return respond({ user_id: "fixture", csrf_token: "fixture" });
    if (p === "/api/accounts") return respond([accounts[0]]);
    if (p.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (p === "/api/search") return respond({ ...mail, rows });
    const m = rows.find(m => p.endsWith(`/messages/${m.id}`));
    if (m) { bodies.push(m.id); return respond({ ...content, message: m, html: '<p>Prefetched mail</p><img src="https://tracker.invalid/pixel">', attachments: [] }); }
    if (p.endsWith("/conversation")) other.push("conversation");
    if (p.includes("/images") || route.request().method() !== "GET") other.push(p);
    return respond([]);
  });
  await page.goto("/");
  await page.getByRole("tab", { name: "All", exact: true }).focus();
  await page.keyboard.press("Enter");
  const visible = () => page.locator(".message-list").evaluate(root => {
    const rect = root.getBoundingClientRect();
    return [...root.querySelectorAll<HTMLElement>(".message-row")].filter(el => { const r = el.getBoundingClientRect(); return r.bottom > rect.top && r.top < rect.bottom; }).map(el => el.dataset.messageId!.split("/").at(-1)!);
  });
  await expect.poll(async () => { const ids = await visible(); return ids.length > 3 && ids.every(id => bodies.includes(id)); }).toBe(true);
  const initial = await visible();
  expect(bodies.every(id => initial.includes(id))).toBe(true);
  expect(other).toEqual([]);
  expect(bodies.length).toBeLessThan(rows.length);
  await page.locator(".message-list").focus();
  await page.keyboard.press("End");
  await expect.poll(async () => { const ids = await visible(); return ids.some(id => !initial.includes(id)) && ids.every(id => bodies.includes(id)); }).toBe(true);
  expect(new Set(bodies).size).toBe(bodies.length);
  expect(other).toEqual([]);
});

test("offline viewport changes do not fetch bodies", async ({ page }) => {
  const bodies: string[] = [];
  await page.route("**/api/**", async route => {
    const p = new URL(route.request().url()).pathname;
    if (p === "/api/session") return route.fulfill({ json: { user_id: "fixture", csrf_token: "fixture" } });
    if (p === "/api/accounts") return route.fulfill({ json: accounts });
    if (p === "/api/search") return route.fulfill({ json: { ...mail, rows: messages.map(m => ({ ...m, unread: false })) } });
    if (p.includes("/messages/")) bodies.push(p);
    return route.fulfill({ json: [] });
  });
  await page.goto("/");
  await expect(page.locator(".message-open").first()).toBeVisible();
  await page.context().setOffline(true);
  const before = bodies.length;
  await page.locator(".message-open").first().focus();
  await page.waitForTimeout(350);
  expect(bodies).toHaveLength(before);
});

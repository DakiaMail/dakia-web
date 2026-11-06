import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { accounts, content, page as mail } from "./fixtures";
import type { Message } from "@dakia/api-types";

const rows: Message[] = Array.from({ length: 80 }, (_, i) => ({
  ...mail.rows[i % mail.rows.length],
  id: `density-${i}`, provider_id: `density-${i}`, thread_id: `thread-${i}`,
  subject: ["A calmer inbox", "Quarterly planning and a very long subject ".repeat(8), "", "مرحبا Alex 2026: 更新と打ち合わせ", "October receipt"][i % 5],
  sender: { name: ["Maya Chen", "A very long sender name ".repeat(8), "", "فريق التصميم Design team", "Maya Chen"][i % 5], email: i % 5 === 2 ? "" : "sender@example.invalid" },
  preview: i % 2 ? "" : "A short preview to help you decide what needs your attention. ".repeat(5),
  unread: i % 2 === 0,
  received_at: ["2026-10-29T08:10:00Z", "2025-01-01T00:00:00Z", "2026-09-30T22:30:00Z"][i % 3],
  starred: i === 4,
}));
async function fixture(page: Page) {
  const operations: unknown[] = [];
  await page.route("**/api/**", route => {
    const path = new URL(route.request().url()).pathname;
    const respond = (value: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "density-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts.map(a => ({ ...a, display_name: a.id === "work" ? "Work · Product & Design" : "Personal mailbox" })));
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows });
    if (path.endsWith("/conversation")) return respond({ rows: [rows[0]], continuation: null, complete: true });
    if (path.endsWith("/unsubscribe")) return respond(null);
    if (path.includes("/messages/")) return respond({ ...content, message: rows[0] });
    if (path.endsWith("/operations")) {
      const op = route.request().postDataJSON(); operations.push(op);
      return respond({ ...op, status: "applied", revision: 2 });
    }
    return respond([]);
  });
  await page.goto("/");
  await expect(page.locator(".message-row").first()).toBeVisible();
  return operations;
}

for (const [width, dark] of [[320, false], [390, false], [390, true], [600, false], [768, true], [1024, false], [1480, false], [1480, true], [1920, false], [2560, false], [2560, true]] as const) {
  test(`compact aligned list ${width} ${dark ? "dark" : "light"}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    if (dark) await page.addInitScript(() => localStorage.setItem("dakia-theme", "dark"));
    await fixture(page);
    // Continuous-row density is a List-mode invariant. Smart View inserts
    // section headers and reveal controls between its capped groups.
    await page.getByRole("tab", { name: "List", exact: true }).click();
    await page.evaluate(() => document.fonts.ready);
    const measurements = await page.locator(".message-row").evaluateAll(elements => elements.map(el => {
      const box = el.getBoundingClientRect();
      const children = [...el.querySelectorAll(".message-select, .message-open, .star, .sender, .subject, .preview, .account-label, time")].map(c => c.getBoundingClientRect());
      const name = el.querySelector(".sender-name")!.getBoundingClientRect();
      return { top: box.top, bottom: box.bottom, height: box.height, nameX: name.x, contained: children.every(c => c.left >= box.left && c.right <= box.right + 1 && c.top >= box.top && c.bottom <= box.bottom + 1) };
    }));
    expect(measurements.length).toBeGreaterThan(5);
    for (const m of measurements) { expect(m.contained).toBe(true); expect(m.height).toBeLessThanOrEqual(64); }
    for (let i = 1; i < measurements.length; i++) {
      expect(Math.abs(measurements[i].top - measurements[i - 1].bottom)).toBeLessThan(1);
      expect(measurements[i].nameX).toBe(measurements[0].nameX);
    }
    expect(await page.locator(".mailbox").evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await expect(page.locator(".subject").filter({ hasText: "(No subject)" }).first()).toBeVisible();
    await expect(page.locator(".sender-name").filter({ hasText: "Unknown sender" }).first()).toBeVisible();
    if (width === 390 || width === 1480 || width === 2560) {
      const dir = resolve("../../docs/screenshots/list-density"); mkdirSync(dir, { recursive: true });
      await page.screenshot({ path: `${dir}/${width}-${dark ? "dark" : "light"}.png`, fullPage: true });
    }
    await page.locator(".message-list").evaluate(el => el.scrollTop = el.scrollHeight);
    await expect(page.locator(".message-open").last()).toBeVisible();
    // Measured virtual rows remain adjacent after scrolling into later data.
    await expect.poll(() => page.locator(".message-row").last().locator(".message-open").textContent()).toContain(rows.at(-1)!.subject!);
    if (width <= 600) {
      const last = (await page.locator(".message-row").last().boundingBox())!;
      const compose = (await page.locator(".list-compose").boundingBox())!;
      expect(last.y + last.height).toBeLessThanOrEqual(compose.y);
    }
  });
}

test("keyboard selection, bulk state, stars and opening preserve separate targets", async ({ page }) => {
  const operations = await fixture(page);
  const first = page.locator(".message-row").first();
  const checkbox = first.getByRole("checkbox");
  await checkbox.focus(); await page.keyboard.press("Space");
  await expect(checkbox).toBeChecked();
  await expect(page.getByRole("button", { name: "1 selected · Clear" })).toBeVisible();
  await page.locator(".message-row").nth(1).getByRole("checkbox").check();
  await expect(page.getByRole("button", { name: "2 selected · Clear" })).toBeVisible();
  await page.getByRole("button", { name: "2 selected · Clear" }).click();
  await expect(checkbox).not.toBeChecked();
  await first.getByRole("button", { name: "Star message", exact: true }).click();
  await expect.poll(() => operations.length).toBe(1);
  await first.locator(".message-open").focus(); await page.keyboard.press("Enter");
  await expect(page.locator(".conversation-message")).toHaveCount(1);
});

test("resizing actual list pane switches layout and remeasures virtual offsets", async ({ page }) => {
  await page.setViewportSize({ width: 1480, height: 980 });
  await fixture(page);
  await page.getByRole("tab", { name: "List", exact: true }).click();
  await expect(page.locator(".message-row").first()).toHaveCSS("height", "64px");
  await expect(page.getByRole("slider", { name: "Message list width" })).toHaveCount(0);
  await expect(page.getByLabel("Loaded messages")).toHaveCount(0);
  await page.setViewportSize({ width: 1920, height: 980 });
  await expect(page.locator(".message-row").first()).toHaveCSS("height", "52px");
  const values = await page.locator(".message-row").evaluateAll(els => els.map(el => {
    const r = el.getBoundingClientRect(), sender = el.querySelector(".sender")!.getBoundingClientRect(), summary = el.querySelector(".message-summary")!.getBoundingClientRect();
    return { top: r.top, bottom: r.bottom, centered: Math.abs(sender.y + sender.height / 2 - summary.y - summary.height / 2) < 1 };
  }));
  for (let i = 1; i < values.length; i++) { expect(values[i].top).toBe(values[i - 1].bottom); expect(values[i].centered).toBe(true); }
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.locator(".message-row").first()).toHaveCSS("height", "64px");
  expect(await page.locator(".mailbox").evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
});

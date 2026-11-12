import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import type { Message } from "@dakia/api-types";
import { accounts, page as mail } from "./fixtures";

const make = (i: number, values: Partial<Message> = {}): Message => ({
  ...mail.rows[0], id: `mail-${i}`, subject: `Fictional mail ${i}`, provider_id: `mail-${i}`, thread_id: `mail-${i}`,
  received_at: new Date(Date.UTC(2026, 9, 30, 0, -i)).toISOString(), unread: false, ...values,
});
async function fixture(page: Page, options: { size?: number; empty?: boolean; loop?: boolean; mixedRead?: boolean; fail?: boolean; delay?: number } = {}) {
  const calls: string[] = [], writes: string[] = [];
  let failing = !!options.fail;
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url()), path = url.pathname;
    const respond = (json: unknown) => route.fulfill({ json });
    if (route.request().method() !== "GET") writes.push(path);
    if (path === "/api/session") return respond({ user_id: "infinite-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) {
      if (url.searchParams.get("attention") === "true") return respond({ ...mail, rows: [], continuation: null });
      const cursor = url.searchParams.get("continuation") || "initial";
      calls.push(cursor);
      if (cursor !== "initial") {
        await new Promise(resolve => setTimeout(resolve, options.delay ?? 180));
        if (failing) return route.fulfill({ status: 503, json: { error: "Synthetic offline failure" } });
      }
      const index = cursor === "initial" ? 0 : Number(cursor);
      const size = options.size ?? 24;
      let rows = options.empty ? [] : Array.from({ length: size }, (_, i) => make(index * size + i,
        options.mixedRead && i < 10 ? { unread: true, category: "People" } : {}));
      if (index && !options.empty) rows.push(make(0)); // Duplicate across provider pages.
      if (!options.empty) rows.push(make(0, { account_id: "personal", subject: "Personal twin" }));
      const selected = url.searchParams.get("accounts")?.split(",");
      rows = rows.filter(m => !selected || selected.includes(m.account_id));
      return respond({ ...mail, rows, continuation: options.loop && index ? "1" : index < 2 ? String(index + 1) : null });
    }
    return respond([]);
  });
  await page.goto("/");
  await expect(page.locator(".message-list")).toBeVisible();
  return { calls, writes, recover: () => { failing = false; } };
}
async function bottom(page: Page) {
  await page.locator(".message-list").evaluate(el => { el.scrollTop = el.scrollHeight; });
}
async function top(page: Page) {
  await page.locator(".message-list").evaluate(el => { el.scrollTop = 0; });
  await page.waitForTimeout(50);
}
const check = (page: Page, name: string) => page.getByRole("checkbox", { name: `Select ${name}`, exact: true });
for (const width of [390, 1480]) {
  test(`automatic provider pages remain bounded and deduplicated at ${width}`, async ({ page }) => {

    await page.setViewportSize({ width, height: 780 });
    const state = await fixture(page);
    // The first page fills the viewport without fetching more provider mail.
    expect(state.calls).toEqual(["initial"]);
    for (let i = 0; i < 18; i++) {
      await bottom(page); await page.waitForTimeout(240);
    }

    await expect.poll(() => state.calls).toEqual(["initial", "1", "2"]);
    await top(page);
    const names = new Set<string>();
    for (let offset = 0; offset < 7000; offset += 250) {
      await page.locator(".message-list").evaluate((el, offset) => { el.scrollTop = offset; }, offset);
      await page.waitForTimeout(20);
      const visible = await page.locator(".message-row input").evaluateAll(els => els.map(el => el.getAttribute("aria-label")!));
      expect(new Set(visible).size).toBe(visible.length);
      visible.forEach(name => names.add(name));
    }
    expect(names.size).toBe(73);
    await expect(page.getByRole("status").filter({ hasText: "End of loaded mail" })).toBeVisible();
    const dir = resolve("../../verification/infinite-loading"); mkdirSync(dir, { recursive: true });
    await page.screenshot({ path: `${dir}/${width}-personal-end.png`, fullPage: true });
    await page.getByRole("tab", { name: "All", exact: true }).click();
      for (let i = 0; i < 3; i++) { await bottom(page); await page.waitForTimeout(240); }
    await bottom(page);
    await expect(check(page, "Fictional mail 71")).toBeVisible();
    await page.screenshot({ path: `${dir}/${width}-all-end.png`, fullPage: true });
    const beforeRefresh = state.calls.length;
    await page.locator(".header-refresh").click();
    await expect.poll(() => state.calls.length).toBe(beforeRefresh + 3);
    await page.getByRole("combobox", { name: "Account filter" }).selectOption("personal");
    await expect(check(page, "Personal twin")).toBeVisible();
    await expect(check(page, "Fictional mail 0")).toHaveCount(0);
    expect(state.writes).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  });
}

test("network failure keeps rows and requires accessible explicit retry", async ({ page }) => {
  const state = await fixture(page, { fail: true });
  await bottom(page);
  await expect(page.getByRole("button", { name: "Retry loading mail" })).toBeVisible();
  const dir = resolve("../../verification/infinite-loading"); mkdirSync(dir, { recursive: true });
  await page.screenshot({ path: `${dir}/retry.png`, fullPage: true });
  await expect(check(page, "Fictional mail 23")).toBeVisible();
  await top(page); await bottom(page); await page.waitForTimeout(300);
  expect(state.calls).toEqual(["initial", "1"]);
  state.recover();
  await page.getByRole("button", { name: "Retry loading mail" }).focus();
  await page.keyboard.press("Enter");
  await expect.poll(() => state.calls).toEqual(["initial", "1", "1"]);
  await expect(page.getByRole("button", { name: "Retry loading mail" })).toHaveCount(0);
  expect(state.writes).toEqual([]);
});

test("short empty pages never cascade; native fallback reaches the end", async ({ page }) => {
  const state = await fixture(page, { empty: true });
  await expect.poll(() => state.calls).toEqual(["initial", "1"]);
  await page.waitForTimeout(500);
  expect(state.calls).toEqual(["initial", "1"]);
  const fallback = page.getByRole("button", { name: "Load more", exact: true });
  await fallback.focus(); await page.keyboard.press("Space");
  await expect.poll(() => state.calls).toEqual(["initial", "1", "2"]);
  await expect(page.getByText("No personal mail", { exact: true })).toBeVisible();
});

test("provider cursor cycle stops after one progress page", async ({ page }) => {
  const state = await fixture(page, { empty: true, loop: true });
  await expect.poll(() => state.calls).toEqual(["initial", "1"]);
  await expect(page.getByRole("status").filter({ hasText: "Paging stopped" })).toBeVisible();
  await page.waitForTimeout(500);
  expect(state.calls).toEqual(["initial", "1"]);
});

test("read and unread mail stay in the same chronology while pages grow", async ({ page }) => {
  const state = await fixture(page, { size: 40, mixedRead: true });
  await expect(check(page, "Fictional mail 0")).toBeVisible();
  for (let i = 0; i < 5; i++) { await bottom(page); await page.waitForTimeout(240); }
  await top(page);
  await expect(check(page, "Fictional mail 0")).toBeVisible();
  await expect(page.locator(".group-heading")).toHaveCount(0);
  expect(state.calls).toEqual(["initial", "1", "2"]);
  expect(state.writes).toEqual([]);
});

test("keyboard scroll uses the list container and coalesces pending provider requests", async ({ page }) => {
  const state = await fixture(page, { delay: 900 });
  await page.waitForTimeout(200);
  expect(state.calls).toEqual(["initial"]);
  const list = page.getByRole("region", { name: "Messages", exact: true });
  await list.focus();
  await page.keyboard.press(process.platform === "darwin" ? "Meta+ArrowDown" : "Control+End");
  await expect.poll(() => state.calls).toEqual(["initial", "1"]);
  await expect(list).toHaveAttribute("aria-busy", "true");
  await expect(page.getByRole("button", { name: "Load more", exact: true })).toBeDisabled();
  const dir = resolve("../../verification/infinite-loading"); mkdirSync(dir, { recursive: true });
  await page.screenshot({ path: `${dir}/loading.png`, fullPage: true });
  for (let i = 0; i < 8; i++) await page.keyboard.press("PageDown");
  expect(state.calls).toEqual(["initial", "1"]);
  await expect(list).toHaveAttribute("aria-busy", "false");
  expect(await page.evaluate(() => document.scrollingElement?.scrollTop)).toBe(0);
  expect(state.writes).toEqual([]);
});

test("viewport resize keeps cached rows without draining provider pages", async ({ page }) => {
  await page.setViewportSize({ width: 1480, height: 480 });
  const state = await fixture(page, { size: 12 });
  await expect(check(page, "Fictional mail 11")).toHaveCount(1);
  expect(state.calls).toEqual(["initial"]);
  await page.setViewportSize({ width: 1480, height: 1200 });
  await expect(check(page, "Fictional mail 11")).toHaveCount(1);
  await expect.poll(() => state.calls).toEqual(["initial", "1"]);
  await page.waitForTimeout(400);
  expect(state.calls).toEqual(["initial", "1"]);
  expect(state.writes).toEqual([]);
});

test("switching accounts during provider paging cannot leak the previous page", async ({ page }) => {
  const state = await fixture(page, { delay: 900 });
  await bottom(page);
  await expect.poll(() => state.calls.includes("1")).toBe(true);
  await page.getByRole("combobox", { name: "Account filter" }).selectOption("personal");
  await expect(check(page, "Personal twin")).toBeVisible();
  await page.waitForTimeout(1200);
  await expect(page.locator(".message-row")).toHaveCount(1);
  await expect(check(page, "Fictional mail 24")).toHaveCount(0);
  expect(state.writes).toEqual([]);
});

for (const nav of ["Starred", "Snoozed", "Sent", "Trash", "Search"]) {
  test(`incremental scroll pagination applies to ${nav}`, async ({ page }) => {
    const state = await fixture(page);
    if (nav === "Search") {
      await page.getByRole("textbox", { name: "Search all mail" }).fill("fictional");
    } else await page.getByRole("button", { name: nav, exact: true }).click();
    await expect.poll(() => state.calls.filter(c => c === "initial").length).toBe(2);
    await bottom(page);
    await expect.poll(() => state.calls.includes("1")).toBe(true);
    expect(state.writes).toEqual([]);
  });
}

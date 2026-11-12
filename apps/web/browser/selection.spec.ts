import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import type { Message, Operation } from "@dakia/api-types";
import { accounts, content, page as mail } from "./fixtures";

const initial: Message[] = Array.from({ length: 8 }, (_, i) => ({
  ...mail.rows[i], id: `range-${i}`, provider_id: `range-${i}`,
  subject: `Range message ${i}`, category: i % 2 ? "People" : "Newsletters",
  category_status: "ready", unread: false,
}));
const key = (m: Message) => `${m.account_id}/${m.id}`;
const checkbox = (page: Page, i: number) => page.getByRole("checkbox", { name: `Select Range message ${i}`, exact: true });
const row = (page: Page, i: number) => page.locator(".message-row").filter({ has: checkbox(page, i) });
async function selected(page: Page, ids: number[]) {
  await expect(page.locator('.message-select input:checked')).toHaveCount(ids.length);
  for (const i of ids) await expect(checkbox(page, i)).toBeChecked();
  if (ids.length) await expect(page.getByRole("button", { name: `${ids.length} selected · Clear`, exact: true }).or(page.getByRole("button", { name: `Clear selection (${ids.length} selected)`, exact: true })).filter({ visible: true })).toBeVisible();
  else await expect(page.getByRole("button", { name: /selected · Clear/ })).toHaveCount(0);
}
async function fixture(page: Page, source = initial) {
  let rows = source.map(m => ({ ...m }));
  const operations: Operation[] = [];
  const discoveries: string[] = [];
  // Visible bodies may now be prefetched. Selection must still never open a
  // reader, discover a conversation or submit a read-marking operation.
  const bodyReads: string[] = [];
  await page.route("**/api/**", route => {
    const url = new URL(route.request().url()), path = url.pathname;
    const respond = (value: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "range-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/operations")) {
      const op: Operation = route.request().postDataJSON(); operations.push(op);
      if (op.change.kind === "archive") rows = rows.filter(m => key(m) !== `${op.account_id}/${op.target_id}`);
      return respond({ ...op, status: "applied", revision: 2 });
    }
    if (path === "/api/search" || path.endsWith("/messages")) {
      const account = path.match(/accounts\/([^/]+)/)?.[1] || url.searchParams.get("accounts");
      let matches = rows.filter(m => !account || account.includes(m.account_id));
      const category = url.searchParams.get("category");
      if (category) matches = matches.filter(m => m.category?.toLowerCase() === category);
      return respond({ ...mail, rows: matches });
    }
    if (path.endsWith("/conversation")) {
      discoveries.push(path);
      const m = rows.find(m => path.includes(`/messages/${m.id}/`)) || rows[0];
      return respond({ rows: [m], continuation: null, complete: true });
    }
    if (path.includes("/messages/")) {
      bodyReads.push(path);
      return respond({ ...content, message: rows.find(m => path.endsWith(m.id)) || rows[0] });
    }
    return respond([]);
  });
  await page.goto("/");
  await expect(page.locator(".message-row").first()).toBeVisible();
  return { operations, discoveries, bodyReads, replace: (next: Message[]) => { rows = next; } };
}
async function list(page: Page) { await page.getByRole("tab", { name: "All", exact: true }).click(); }

test("inclusive forward range, stable anchor, reverse range and deselection", async ({ page }) => {
  const state = await fixture(page); await list(page);
  await checkbox(page, 2).click();
  await checkbox(page, 5).click({ modifiers: ["Shift"] });
  await selected(page, [2, 3, 4, 5]);
  await checkbox(page, 0).click({ modifiers: ["Shift"] });
  await selected(page, [0, 1, 2, 3, 4, 5]);
  await checkbox(page, 4).click({ modifiers: ["Shift"] });
  await selected(page, [0, 1, 5]);
  await checkbox(page, 7).click();
  await checkbox(page, 5).click({ modifiers: ["Shift"] });
  await selected(page, [0, 1]);
  expect(state.operations).toEqual([]); expect(state.discoveries).toEqual([]);
  await expect(page.locator(".conversation")).toHaveCount(0);
});

test("first Shift-click and clear reset have no stale anchor", async ({ page }) => {
  await fixture(page); await list(page);
  await checkbox(page, 4).click({ modifiers: ["Shift"] }); await selected(page, [4]);
  await page.getByRole("button", { name: "1 selected · Clear" }).click();
  await checkbox(page, 1).click({ modifiers: ["Shift"] }); await selected(page, [1]);
  await page.locator("h1").click(); await page.keyboard.press("Escape");
  await checkbox(page, 6).click({ modifiers: ["Shift"] }); await selected(page, [6]);
});

test("chronological tabs preserve available checkmarks and reset the range anchor", async ({ page }) => {
  await fixture(page, initial.map(m => ({ ...m, unread: true })));
  await checkbox(page, 3).click();
  await checkbox(page, 2).click({ modifiers: ["Shift"] });
  await selected(page, [2, 3]);
  await list(page); await selected(page, [2, 3]);
  await checkbox(page, 6).click({ modifiers: ["Shift"] });
  await selected(page, [2, 3, 6]);
});

test("checkbox label hit area preserves Shift and leaves star/open separate", async ({ page }) => {
  const state = await fixture(page); await list(page);
  await row(page, 1).locator(".message-select").click({ position: { x: 2, y: 2 } });
  await row(page, 4).locator(".message-select").click({ position: { x: 2, y: 2 }, modifiers: ["Shift"] });
  await selected(page, [1, 2, 3, 4]);
  expect(state.discoveries).toEqual([]);
  await expect(page.locator(".conversation")).toHaveCount(0); expect(state.operations).toEqual([]);
  await row(page, 6).getByRole("button", { name: "Star message", exact: true }).click({ modifiers: ["Shift"] });
  await expect.poll(() => state.operations.length).toBe(1);
  await selected(page, [1, 2, 3, 4]);
});

test("ordinary open anchors Shift-row selection without opening another reader or marking mail read", async ({ page }) => {
  const state = await fixture(page, initial.map((m, i) => ({ ...m, unread: i === 4 }))); await list(page);
  await row(page, 1).locator(".message-open").click();
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  await row(page, 4).locator(".message-open").click({ modifiers: ["Shift"] });
  await selected(page, [1, 2, 3, 4]);
  expect(state.discoveries.some(path => path.includes("/messages/range-4"))).toBe(false);
  await expect(page.locator(".message-row.selected")).toContainText("Range message 1");
  expect(state.operations).toEqual([]);
});

test("keyboard Space, Shift+Space and Shift+Enter select without opening", async ({ page }) => {
  const state = await fixture(page); await list(page);
  await checkbox(page, 1).focus(); await page.keyboard.press("Space");
  await checkbox(page, 4).focus(); await page.keyboard.press("Shift+Space");
  await selected(page, [1, 2, 3, 4]);
  await checkbox(page, 3).focus(); await page.keyboard.press("Shift+Space");
  await selected(page, [4]);
  await row(page, 6).locator(".message-open").focus(); await page.keyboard.press("Shift+Enter");
  await selected(page, [1, 2, 3, 4, 5, 6]);
  await expect(row(page, 6).locator(".message-open")).toBeFocused();
  expect(state.discoveries).toEqual([]);
  await expect(page.locator(".conversation")).toHaveCount(0); expect(state.operations).toEqual([]);
});

test("account, mailbox and search changes clear selection and anchor", async ({ page }) => {
  await fixture(page); await list(page);
  await checkbox(page, 0).click();
  await page.getByRole("combobox", { name: "Account filter" }).selectOption("personal");
  await selected(page, []);
  await checkbox(page, 5).click({ modifiers: ["Shift"] }); await selected(page, [5]);
  await page.getByRole("button", { name: "Starred", exact: true }).click();
  await selected(page, []);
  await page.getByRole("button", { name: "Unified inbox", exact: true }).click();
  await page.getByRole("combobox", { name: "Account filter" }).selectOption("all");
  await checkbox(page, 0).click();
  await page.getByRole("textbox", { name: "Search all mail" }).fill("range");
  await expect(page.locator(".mailbox h1")).toHaveText("Search");
  await selected(page, []);
  await checkbox(page, 5).click({ modifiers: ["Shift"] }); await selected(page, [5]);
});

test("refresh removes stale checkmarks and anchor; reordered rows use current order", async ({ page }) => {
  const state = await fixture(page); await list(page);
  await checkbox(page, 2).click();
  state.replace(initial.filter(m => m.id !== "range-2"));
  await page.getByRole("button", { name: "Refresh mail", exact: true }).click();
  await expect(checkbox(page, 2)).toHaveCount(0); await selected(page, []);
  await checkbox(page, 4).click({ modifiers: ["Shift"] }); await selected(page, [4]);
  state.replace([initial[4], initial[1], initial[7], initial[0]]);
  await page.getByRole("button", { name: "Refresh mail", exact: true }).click();
  await expect(checkbox(page, 6)).toHaveCount(0);
  await checkbox(page, 7).click({ modifiers: ["Shift"] }); await selected(page, [4, 1, 7]);
});

test("bulk action affects precisely the selected cross-account range, not its conversation", async ({ page }) => {
  const state = await fixture(page); await list(page);
  await checkbox(page, 1).click(); await checkbox(page, 4).click({ modifiers: ["Shift"] });
  await page.getByRole("button", { name: "Archive", exact: true }).click();
  await expect.poll(() => state.operations.length).toBe(4);
  expect(state.operations.map(op => `${op.account_id}/${op.target_id}`).sort()).toEqual(initial.slice(1, 5).map(key).sort());
  expect(state.operations.every(op => op.change.kind === "archive")).toBe(true);
  await selected(page, []);
});

test("virtualized ranges include offscreen loaded messages", async ({ page }) => {
  const many = Array.from({ length: 100 }, (_, i) => ({ ...initial[i % 8], id: `range-${i}`, subject: `Range message ${i}` }));
  const state = await fixture(page, many); await list(page);
  await checkbox(page, 2).click();
  await page.locator(".message-list").evaluate(el => el.scrollTop = el.scrollHeight);
  await checkbox(page, 98).click({ modifiers: ["Shift"] });
  await expect(page.getByRole("button", { name: "97 selected · Clear", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Mark unread", exact: true }).click();
  await expect.poll(() => state.operations.length).toBe(97);
  expect(state.operations.map(op => `${op.account_id}/${op.target_id}`).sort()).toEqual(many.slice(2, 99).map(key).sort());
});

for (const [width, dark] of [[1480, false], [1480, true], [390, false], [390, true]] as const) {
  test(`range selection evidence ${width} ${dark ? "dark" : "light"}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    if (dark) await page.addInitScript(() => localStorage.setItem("dakia-theme", "dark"));
    const state = await fixture(page); await list(page);
    await checkbox(page, 1).click(); await checkbox(page, 5).click({ modifiers: ["Shift"] });
    await selected(page, [1, 2, 3, 4, 5]);
    await page.evaluate(() => document.fonts.ready);
    const dir = resolve("../../docs/screenshots/shift-selection"); mkdirSync(dir, { recursive: true });
    await page.screenshot({ path: `${dir}/${width}-${dark ? "dark" : "light"}.png`, fullPage: true });
    expect(state.operations).toEqual([]); expect(state.discoveries).toEqual([]);
    await expect(page.locator(".conversation")).toHaveCount(0);
  });
}

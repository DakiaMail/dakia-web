import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import type { Message, Operation } from "@dakia/api-types";
import { accounts, content, page as mail } from "./fixtures";

const make = (id: string, values: Partial<Message> = {}): Message => ({
  ...mail.rows[0], id, provider_id: id, thread_id: id, subject: id,
  category: "People", unread: true, ...values,
});
const initial = [
  ...Array.from({ length: 10 }, (_, i) => make(`Person ${i}`)),
  make("Personal twin", { id: "Person 0", account_id: "personal" }),
  make("Notification", { category: "Notifications" }),
  make("Read person", { unread: false }),
  make("Awaiting classification", { category_status: "pending" }),
  ...Array.from({ length: 9 }, (_, i) => make(`Read ${i}`, { unread: false })),
];
async function fixture(page: Page, source = initial, paginated = false, pageSize = 7) {
  let rows = source.map(m => ({ ...m }));
  const operations: Operation[] = [];
  const pages: string[] = [];
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url()), path = url.pathname;
    const respond = (value: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "smart-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/operations")) {
      const op: Operation = route.request().postDataJSON(); operations.push(op);
      rows = rows.map(m => m.account_id === op.account_id && m.id === op.target_id && op.change.kind === "read"
        ? { ...m, unread: !op.change.value, revision: 2 } : m);
      return respond({ ...op, status: "applied", revision: 2 });
    }
    if (path === "/api/search" || path.endsWith("/messages")) {
      const account = url.searchParams.get("accounts");
      const cursor = url.searchParams.get("continuation");
      pages.push(cursor || "initial");
      const matches = rows.filter(m => !account || account.split(",").includes(m.account_id));
      if (cursor) await new Promise(resolve => setTimeout(resolve, 250));
      return respond({ ...mail, rows: paginated ? cursor ? [...matches.slice(pageSize), matches[0]] : matches.slice(0, pageSize) : matches,
        continuation: paginated && !cursor ? "next-fixture-page" : null });
    }
    if (path.endsWith("/conversation")) {
      const m = rows.find(m => path.includes(`/messages/${encodeURIComponent(m.id)}/`)) || rows[0];
      return respond({ rows: [m], continuation: null, complete: true });
    }
    if (path.includes("/messages/")) return respond({ ...content, message: rows[0] });
    return respond([]);
  });
  await page.goto("/");
  await expect(page.locator(".group-heading").first()).toBeVisible();
  return { operations, pages };
}
const heading = (page: Page, label: string) => page.getByRole("button", { name: new RegExp(`^${label} ·`) });
const check = (page: Page, subject: string) => page.getByRole("checkbox", { name: `Select ${subject}`, exact: true });
async function bottom(page: Page) {
  await page.locator(".message-list").evaluate(el => el.scrollTop = el.scrollHeight);
}

for (const [width, theme] of [[390, "light"], [390, "dark"], [1480, "light"], [1480, "dark"]] as const) {
  test(`Smart View collapse, automatic cached reveal and screenshots at ${width} ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    await page.addInitScript(theme => localStorage.setItem("dakia-theme", theme), theme);
    const state = await fixture(page);
    await expect(heading(page, "people")).toHaveAttribute("aria-expanded", "true");
    await expect(check(page, "Person 0")).toHaveCount(1);
    await expect(check(page, "Person 8")).toHaveCount(1);
    await heading(page, "people").focus(); await page.keyboard.press("Enter");
    await expect(heading(page, "people")).toHaveAttribute("aria-expanded", "false");
    await expect(check(page, "Person 0")).toHaveCount(0);
    await expect(heading(page, "people")).toBeFocused();
    await page.keyboard.press("Space");
    await expect(check(page, "Person 0")).toHaveCount(1);
    await expect(check(page, "Person 8")).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Load more people messages", exact: true })).toHaveCount(0);
    await bottom(page);
    await bottom(page);
    await expect(check(page, "Read 8")).toBeVisible();
    await heading(page, "seen").click();
    await expect(check(page, "Read 8")).toHaveCount(0);
    await page.locator(".message-list").evaluate(el => el.scrollTop = 0);
    await heading(page, "people").click();
    await expect(heading(page, "people")).toHaveAttribute("aria-expanded", "false");
    await expect(heading(page, "notifications")).toHaveAttribute("aria-expanded", "true");
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    const dir = resolve("../../docs/screenshots/smart-view"); mkdirSync(dir, { recursive: true });
    await page.screenshot({ path: `${dir}/${width}-${theme}-collapsed.png`, fullPage: true });
    await heading(page, "people").click();
    await page.screenshot({ path: `${dir}/${width}-${theme}-expanded.png`, fullPage: true });
    expect(state.operations).toEqual([]);
  });
}

test("provider pagination appends once, keeps collapse state and account-local identities", async ({ page }) => {
  const state = await fixture(page, initial, true);
  await heading(page, "people").click();
  await bottom(page);
  await expect.poll(() => state.pages.includes("next-fixture-page")).toBe(true);
  await page.locator(".message-list").evaluate(el => el.scrollTop = 0);
  await expect(heading(page, "people")).toHaveAttribute("aria-expanded", "false");
  await expect(heading(page, "people")).toHaveAccessibleName("people · 11 loaded messages");
  await heading(page, "people").click();
  await expect(check(page, "Person 0")).toHaveCount(1);
  await expect(check(page, "Personal twin")).toHaveCount(1);
  await page.getByRole("combobox", { name: "Account filter" }).selectOption("personal");
  await expect(heading(page, "people")).toHaveAccessibleName("people · 1 loaded messages");
  await expect(check(page, "Personal twin")).toHaveCount(1);
  await expect(check(page, "Person 0")).toHaveCount(0);
  await expect(heading(page, "seen")).toHaveCount(0);
  expect(state.operations).toEqual([]);
});

test("read mail stays until switching emails, then read and unread move it exactly once", async ({ page }) => {
  await fixture(page, [make("Only person"), make("Already read", { unread: false })]);
  await page.locator(".message-open").filter({ hasText: "Only person" }).click();
  await expect(page.locator(".message-row").filter({ hasText: "Only person" })).not.toHaveClass(/unread/);
  await expect(heading(page, "people")).toHaveAccessibleName("people · 1 loaded messages");
  await expect(heading(page, "seen")).toHaveAccessibleName("seen · 1 loaded messages");
  await page.locator(".message-open").filter({ hasText: "Already read" }).click();
  await expect(heading(page, "people")).toHaveCount(0);
  await expect(heading(page, "seen")).toBeVisible();
  await expect(check(page, "Only person")).toHaveCount(1);
  await check(page, "Only person").check();
  await page.getByRole("button", { name: "Mark unread", exact: true }).click();
  await expect(heading(page, "people")).toBeVisible();
  await expect(heading(page, "seen")).toHaveAccessibleName("seen · 1 loaded messages");
  await expect(check(page, "Only person")).toHaveCount(1);
});

test("List and search remain ungrouped and are not capped by Smart View sections", async ({ page }) => {
  await fixture(page, Array.from({ length: 10 }, (_, i) => make(`List ${i}`)));
  await page.getByRole("tab", { name: "List", exact: true }).click();
  await expect(page.locator(".group-heading")).toHaveCount(0);
  await expect(check(page, "List 9")).toHaveCount(1);
  await page.getByRole("tab", { name: "Smart", exact: true }).click();
  await expect(check(page, "List 9")).toHaveCount(1);
  await page.getByRole("textbox", { name: "Search all mail" }).fill("List");
  await expect(page.locator(".group-heading")).toHaveCount(0);
  await expect(check(page, "List 9")).toHaveCount(1);
});

for (const [width, theme] of [[390, "light"], [390, "dark"], [1480, "light"], [1480, "dark"]] as const) {
  test(`automatic cached overflow precedes provider mail at ${width} ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    await page.addInitScript(theme => localStorage.setItem("dakia-theme", theme), theme);
    const source = [
      ...Array.from({ length: 10 }, (_, i) => make(`Person ${i}`)),
      make("Personal twin", { id: "Person 0", account_id: "personal" }),
      ...Array.from({ length: 19 }, (_, i) => make(`Seen ${i}`, { unread: false })),
      ...Array.from({ length: 3 }, (_, i) => make(`Later ${i}`, { unread: false })),
    ];
    const state = await fixture(page, source, true, 30);
    const footer = page.getByRole("button", { name: "Load more mail", exact: true });
    await expect(check(page, "Person 9")).toHaveCount(1);
    await expect(heading(page, "notifications")).toHaveCount(0);
    await bottom(page);
    await expect(page.getByRole("button", { name: "Load more seen messages", exact: true })).toHaveCount(0);
    const dir = resolve("../../docs/screenshots/load-more"); mkdirSync(dir, { recursive: true });
    await page.screenshot({ path: `${dir}/${width}-${theme}-overflow-and-pagination.png`, fullPage: true });
    await bottom(page);
    await expect(check(page, "Seen 15")).toBeVisible();
    expect(state.pages).toEqual(["initial"]);
    for (let i = 0; i < 4; i++) { await bottom(page); await page.waitForTimeout(100); }
    await expect.poll(() => state.pages).toEqual(["initial", "next-fixture-page"]);
    await expect(footer).toHaveCount(0);
    await bottom(page);
    await expect(check(page, "Later 2")).toBeVisible();
    await page.screenshot({ path: `${dir}/${width}-${theme}-exhausted.png`, fullPage: true });
    await page.locator(".message-list").evaluate(el => el.scrollTop = 0);
    await expect(check(page, "Person 9")).toHaveCount(1);
    // Collect identities across virtualized rows, including the duplicate
    // provider row and the same message ID belonging to a second account.
    const subjects = new Set<string>();
    for (let top = 0; top <= 3000; top += 350) {
      await page.locator(".message-list").evaluate((el, top) => el.scrollTop = top, top);
      await page.waitForTimeout(30);
      const names = await page.locator(".message-row input").evaluateAll(els => els.map(el => el.getAttribute("aria-label") || ""));
      expect(new Set(names).size).toBe(names.length);
      for (const name of names) subjects.add(name.slice(7));
    }
    expect([...subjects].sort()).toEqual(source.map(m => m.subject).sort());
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    expect(state.operations).toEqual([]);
  });
}

test("a sole overflowing section shares the footer and collapsed sections do not reveal mail", async ({ page }) => {
  const state = await fixture(page, Array.from({ length: 40 }, (_, i) => make(`Only ${i}`)), true, 30);
  const footer = page.getByRole("button", { name: "Load more mail", exact: true });
  await expect(page.locator(".section-more")).toHaveCount(0);
  await expect(check(page, "Only 9")).toBeVisible();
  expect(state.pages).toEqual(["initial"]);
  await heading(page, "people").click();
  await page.waitForTimeout(100);
  expect(state.pages).toEqual(["initial"]);
  await expect(heading(page, "people")).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(".message-row")).toHaveCount(0);
  // Explicit fallback can fetch even when every category is collapsed.
  await footer.click();
  await expect(footer).toHaveCount(0);
  await expect(heading(page, "people")).toHaveAttribute("aria-expanded", "false");
  await expect(page.locator(".message-row")).toHaveCount(0);
  await heading(page, "people").click();
  for (let i = 0; i < 6; i++) { await bottom(page); await page.waitForTimeout(80); }
  await expect(check(page, "Only 39")).toBeVisible();
  await expect(footer).toHaveCount(0);
  expect(state.pages).toEqual(["initial", "next-fixture-page"]);
  expect(state.operations).toEqual([]);
});

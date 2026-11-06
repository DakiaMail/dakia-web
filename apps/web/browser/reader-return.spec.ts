import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { accounts, content, page as mail } from "./fixtures";
import type { Operation } from "@dakia/api-types";

async function fixture(page: Page, options: { thread?: boolean; direct?: boolean; reject?: boolean } = {}) {
  const rows = Array.from({ length: 35 }, (_, i) => ({ ...mail.rows[0], id: `return-${i}`, subject: `Return fixture ${i}`, unread: false, revision: 1 }));
  const operations = new Map<string, Operation>();
  let reads = 0;
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url()), path = url.pathname;
    const respond = (value: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "return-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/operations")) {
      if (options.reject) return route.fulfill({ status: 503, contentType: "application/json", body: JSON.stringify({ code: "fixture_failure", message: "Disposable provider unavailable" }) });
      const op: Operation = { ...route.request().postDataJSON(), status: "pending", revision: 2 };
      operations.set(op.id, op); return respond(op);
    }
    if (path.includes("/operations/")) return respond(operations.get(path.split("/").at(-1)!));
    if (path === "/api/search" || path.endsWith("/messages")) {
      reads++;
      return respond({ ...mail, rows: rows.filter(m => ![...operations.values()].some(op => op.target_id === m.id && op.status === "applied")), continuation: null });
    }
    const message = rows.find(m => path.includes(`/messages/${m.id}`)) || rows[0];
    if (path.endsWith("/conversation")) return respond({ rows: options.thread ? [message, rows[34]] : [message], complete: true, continuation: null });
    if (path.includes("/messages/")) return respond({ ...content, message });
    return respond([]);
  });
  if (options.direct) await page.goto("/?previous=1");
  await page.goto(options.direct ? `/?account=${accounts[0].id}&message=return-0&keep=1` : "/?keep=1");
  return { operations, reads: () => reads, settle(status: "applied" | "failed") {
    for (const [id, op] of operations) operations.set(id, { ...op, status, revision: 3 });
  } };
}
const list = (page: Page) => page.locator(".message-list");
const reader = (page: Page) => page.getByRole("region", { name: "Reading pane" });
async function capture(page: Page, name: string) {
  mkdirSync("artifacts/reader-return", { recursive: true });
  await page.screenshot({ path: `artifacts/reader-return/${name}.png`, fullPage: true });
}

for (const width of [1480, 390]) for (const action of ["Archive", "Move to trash"]) for (const thread of [false, true]) {
  test(`return to unchanged list ${width} ${action} thread=${thread}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    const state = await fixture(page, { thread });
    // Change account, folder and search before opening the reader.
    if (width === 390) await page.getByRole("button", { name: "Open navigation" }).click();
    await page.getByRole("combobox", { name: "Account filter" }).selectOption(accounts[0].id);
    await page.getByRole("button", { name: "Sent", exact: true }).click();
    if (width === 390) await page.getByRole("button", { name: "Open navigation" }).click();
    await page.getByRole("textbox", { name: "Search all mail" }).fill("Return");
    await page.getByRole("textbox", { name: "Search all mail" }).press("Enter");
    await list(page).evaluate(el => { el.scrollTop = 450; });
    await expect.poll(() => list(page).evaluate(el => el.scrollTop)).toBeGreaterThan(300);
    const scroll = await list(page).evaluate(el => el.scrollTop);
    const row = page.locator(".message-open").filter({ visible: true }).filter({ hasText: "Return fixture 8" });
    await row.click();
    await expect(page.locator(".conversation-message")).toHaveCount(thread ? 2 : 1);
    const history = await page.evaluate(() => window.history.length);
    await capture(page, `${width}-${action}-${thread}-before`);
    await reader(page).getByRole("button", { name: action, exact: true }).click();
    // Close immediately while provider work is pending, without claiming success.
    await expect(page.locator(".app")).not.toHaveClass(/reading/);
    await expect(list(page)).toBeVisible();
    await expect(page.locator(".conversation-message")).toHaveCount(0);
    await expect.poll(() => state.operations.size).toBe(1);
    expect([...state.operations.values()][0]).toMatchObject({ account_id: accounts[0].id, target_id: "return-8", change: { kind: action === "Archive" ? "archive" : "trash" }, status: "pending" });
    expect(await page.evaluate(() => window.history.length)).toBe(history);
    expect(new URL(page.url()).searchParams.get("keep")).toBe("1");
    expect(await list(page).evaluate(el => el.scrollTop)).toBe(scroll);
    state.settle("applied");
    await expect.poll(state.reads).toBeGreaterThan(3);
    await expect(page.getByRole("alert")).toHaveCount(0);
    await capture(page, `${width}-${action}-${thread}-after`);
    if (width === 390) await page.getByRole("button", { name: "Open navigation" }).click();
    await expect(page.getByRole("combobox", { name: "Account filter" })).toHaveValue(accounts[0].id);
    await expect(page.getByRole("textbox", { name: "Search all mail" })).toHaveValue("Return");
  });
}
for (const width of [1480, 390]) for (const action of ["Archive", "Move to trash"]) {
  test(`pending failure stays visible across later actions ${width} ${action}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    const state = await fixture(page);
    await page.getByRole("tab", { name: "List", exact: true }).click();
    await page.locator(".message-open").first().click();
    await reader(page).getByRole("button", { name: action, exact: true }).click();
    await expect.poll(() => state.operations.size).toBe(1);
    state.settle("failed");
    await expect(page.getByRole("alert")).toHaveText(`${action} could not be verified for “Return fixture 0”. Refresh mail and check its current state before trying again.`);
    await expect(page.getByRole("alert")).toBeVisible();
    await expect(page.getByRole("checkbox", { name: "Select Return fixture 0", exact: true })).toBeVisible();
    await page.locator(".message-open").first().click();
    await reader(page).getByRole("button", { name: "Mark unread", exact: true }).click();
    await expect(page.getByRole("alert")).toBeVisible();
    await capture(page, `${width}-${action}-queued-error`);
    state.settle("applied");
    await page.getByRole("button", { name: "Dismiss mail action error" }).click();
    await expect(page.getByRole("alert")).toHaveCount(0);
  });
  test(`direct link safe fallback and immediate error ${width} ${action}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    await fixture(page, { direct: true, reject: true });
    await expect(page.locator(".conversation-message")).toHaveCount(1);
    const history = await page.evaluate(() => window.history.length);
    await reader(page).getByRole("button", { name: action, exact: true }).click();
    await expect(list(page)).toBeVisible();
    await expect(page.getByRole("alert")).toContainText("Disposable provider unavailable");
    await expect(page.getByRole("alert")).toBeVisible();
    const url = new URL(page.url());
    expect(url.searchParams.has("account")).toBe(false);
    expect(url.searchParams.has("message")).toBe(false);
    expect(url.searchParams.get("keep")).toBe("1");
    expect(await page.evaluate(() => window.history.length)).toBe(history);
    await page.reload();
    await expect(list(page)).toBeVisible();
    await expect(page.locator(".conversation-message")).toHaveCount(0);
    await page.goBack();
    expect(new URL(page.url()).searchParams.get("previous")).toBe("1");
    await page.goForward();
    await expect(list(page)).toBeVisible();
    await expect(page.locator(".conversation-message")).toHaveCount(0);
  });
}
test("bulk with a different preview does not close or expand targets", async ({ page }) => {
  const state = await fixture(page);
  await page.getByRole("tab", { name: "List", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select Return fixture 1", exact: true }).check();
  await page.getByRole("checkbox", { name: "Select Return fixture 2", exact: true }).check();
  await page.locator(".message-open").first().click();
  await reader(page).getByRole("button", { name: "Archive", exact: true }).click();
  await expect.poll(() => state.operations.size).toBe(2);
  expect([...state.operations.values()].map(op => op.target_id).sort()).toEqual(["return-1", "return-2"]);
  await expect(page.locator(".app")).toHaveClass(/reading/);
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  state.settle("applied");
});

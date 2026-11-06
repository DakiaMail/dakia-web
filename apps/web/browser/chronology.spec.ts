import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import type { Message, Operation } from "@dakia/api-types";
import { accounts, content, page as mail } from "./fixtures";

const make = (id: string, received_at: string, values: Partial<Message> = {}): Message => ({
  ...mail.rows[0], id, subject: id, thread_id: id, provider_id: id,
  received_at, unread: false, ...values,
});
// Individually sorted federated pages: the older account's first batch ends
// before the newer account's next batch begins. Same provider ID in both accounts.
const first = [make("October 29", "2026-10-29T08:00:00Z", { account_id: "personal" }),
  make("September 05", "2026-09-05T08:00:00Z"),
  make("Unread October 30", "2026-10-30T08:00:00Z", { unread: true })];
const second = [make("October 28", "2026-10-28T10:00:00+02:00", { id: "shared", account_id: "personal" }),
  make("September 02", "2026-09-02T08:00:00Z", { id: "shared" }), first[0]];
const third = [make("Timezone earlier", "2026-10-28T10:30:00+03:00", { account_id: "personal" }),
  make("Same instant", "2026-10-28T08:00:00Z"),
  make("September 01", "2026-09-01T08:00:00Z")];
const expected = ["October 29", "October 28", "Same instant", "Timezone earlier", "September 05", "September 02", "September 01"];
async function fixture(page: Page) {
  let refreshed = false;
  const read = new Map<string, boolean>();
  const calls: string[] = [];
  const operations: Operation[] = [];
  await page.route("**/api/**", async route => {
    const request = route.request(), url = new URL(request.url()), path = url.pathname;
    const respond = (json: unknown) => route.fulfill({ json });
    if (path === "/api/session") return respond({ user_id: "chronology-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/operations")) {
      const op: Operation = request.postDataJSON(); operations.push(op);
      if (op.change.kind === "read") read.set(`${op.account_id}/${op.target_id}`, !op.change.value);
      return respond({ ...op, status: "applied", revision: 2 });
    }
    if (path === "/api/search") {
      const cursor = url.searchParams.get("continuation") || "initial";
      calls.push(cursor);
      const rows = cursor === "second" ? second : cursor === "third" ? third : first;
      const selected = url.searchParams.get("accounts")?.split(",");
      return respond({ ...mail, rows: [
        ...(refreshed && cursor === "initial" ? [make("New arrival", "2026-10-31T08:00:00Z", { account_id: "personal" })] : []),
        ...rows,
      ].filter(m => !selected || selected.includes(m.account_id)).map(m => ({ ...m, unread: read.get(`${m.account_id}/${m.id}`) ?? m.unread })),
      continuation: cursor === "initial" ? "second" : cursor === "second" ? "third" : null });
    }
    if (path.endsWith("/conversation")) return respond({ rows: [first[2]], continuation: null, complete: true });
    if (path.includes("/messages/")) return respond({ ...content, message: first[2] });
    return respond([]);
  });
  await page.goto("/");
  await expect(page.locator(".group-heading")).toHaveCount(2);
  return { calls, operations, refresh: () => { refreshed = true; } };
}

// Read the rendered rows, not API payload order or an independent comparator.
const displayed = (page: Page) => page.locator(".message-row input").evaluateAll(els => els.map(el => el.getAttribute("aria-label")!.slice(7)));

for (const width of [390, 1480]) {
  test(`chronological Seen across three account pages, refresh and read transitions at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 1200 });
    const state = await fixture(page);
    const footer = page.getByRole("button", { name: "Load more mail", exact: true });
    await expect.poll(() => state.calls).toEqual(["initial", "second"]);
    await footer.click();
    await expect.poll(() => state.calls).toEqual(["initial", "second", "third"]);
    await expect(footer).toHaveCount(0);
    await expect.poll(() => displayed(page)).toEqual(["Unread October 30", ...expected]);
    await page.locator(".message-open").filter({ hasText: "Unread October 30" }).click();
    if (width === 390) await page.getByRole("button", { name: "Back to inbox", exact: true }).click();
    await expect(page.locator(".group-heading")).toHaveCount(2);
    await expect.poll(() => displayed(page)).toEqual(["Unread October 30", ...expected]);
    const dir = resolve("../../docs/screenshots/chronology"); mkdirSync(dir, { recursive: true });
    await page.screenshot({ path: `${dir}/${width}-seen.png`, fullPage: true });
    state.refresh();
    await page.locator(".header-refresh").click();
    // Read status and refreshed pages must not move the email being read.
    await expect(page.getByRole("button", { name: /^seen ·/ })).toHaveAccessibleName("seen · 8 loaded messages");
    await expect.poll(() => displayed(page)).toEqual(["Unread October 30", "New arrival", ...expected]);
    await page.locator(".message-open").filter({ hasText: "October 29" }).click();
    if (width === 390) await page.getByRole("button", { name: "Back to inbox", exact: true }).click();
    await expect(page.locator(".group-heading")).toHaveCount(1);
    await expect(page.getByRole("button", { name: /^seen ·/ })).toHaveAccessibleName("seen · 9 loaded messages");
    await expect.poll(() => displayed(page)).toEqual(["New arrival", "Unread October 30", ...expected]);
    await page.locator(".message-open").filter({ hasText: "Unread October 30" }).click();
    await page.getByRole("button", { name: "Mark unread", exact: true }).click();
    if (width === 390) await page.getByRole("button", { name: "Back to inbox", exact: true }).click();
    await expect.poll(() => displayed(page)).toEqual(["Unread October 30", "New arrival", ...expected]);
    await page.getByRole("tab", { name: "List", exact: true }).click();
    await expect.poll(() => displayed(page)).toEqual(["New arrival", "Unread October 30", ...expected]);
    expect(state.operations.every(op => op.account_id === "work" && op.target_id === "Unread October 30" && op.change.kind === "read")).toBe(true);
    expect(new Set(await displayed(page)).size).toBe(9);
    await page.getByRole("combobox", { name: "Account filter" }).selectOption("personal");
    await expect.poll(() => displayed(page)).toEqual(["New arrival", "October 29", "October 28"]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  });
}

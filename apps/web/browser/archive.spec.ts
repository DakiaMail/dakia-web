import { test, expect, type Page } from "@playwright/test";
import type { Message, Operation } from "@dakia/api-types";
import { accounts, page as mail } from "./fixtures";

const initial: Message[] = Array.from({ length: 6 }, (_, i) => ({
  ...mail.rows[0], id: `archive-${i}`, account_id: i === 3 ? accounts[1].id : accounts[0].id,
  subject: `Archive fixture ${i}`, unread: false, revision: 1, folder_ids: ["INBOX", "Important"],
}));
const checkbox = (page: Page, i: number) => page.getByRole("checkbox", { name: `Select Archive fixture ${i}`, exact: true });
async function fixture(page: Page) {
  const operations = new Map<string, Operation>();
  let stalePending = true, paginated = false, reads = 0;
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url()), path = url.pathname;
    const respond = (value: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "archive-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/operations")) {
      const op: Operation = { ...route.request().postDataJSON(), revision: 2, status: "pending" };
      operations.set(op.id, op); return respond(op);
    }
    if (path.includes("/operations/")) return respond(operations.get(path.split("/").at(-1)!));
    if (path === "/api/search" || path.endsWith("/messages")) {
      reads++;
      const scope = path.match(/accounts\/([^/]+)/)?.[1] || url.searchParams.get("accounts") || "";
      const rows = initial.filter(m => scope.includes(m.account_id) && ![...operations.values()].some(op =>
        op.account_id === m.account_id && op.target_id === m.id && (op.status === "applied" || (!stalePending && op.status === "pending"))));
      const next = url.searchParams.has("continuation");
      return respond({ ...mail, rows: paginated ? (next ? rows.slice(2) : rows.slice(0, 3)) : rows, continuation: paginated && !next ? "next" : null });
    }
    return respond([]);
  });
  await page.goto("/");
  await page.getByRole("tab", { name: "List", exact: true }).click();
  await expect(checkbox(page, 0)).toBeVisible();
  return {
    operations, reads: () => reads,
    paginate: () => { paginated = true; },
    projectPending: () => { stalePending = false; },
    settle: (i: number, status: "applied" | "failed") => {
      const op = [...operations.values()].find(op => op.target_id === `archive-${i}`)!;
      operations.set(op.id, { ...op, status, revision: 3 });
    },
  };
}
async function archive(page: Page, i: number) {
  await checkbox(page, i).click();
  await page.getByRole("button", { name: "Archive", exact: true }).click();
  await expect(checkbox(page, i)).toHaveCount(0);
}

test("queued sequential archives stay hidden through unrelated actions, repeated stale refresh and out-of-order completion", async ({ page }) => {
  const state = await fixture(page);
  await archive(page, 0); await archive(page, 1);
  await expect.poll(() => state.operations.size).toBe(2);
  // Deliberately return pre-worker INBOX snapshots, including during pagination.
  for (let i = 0; i < 2; i++) {
    const before = state.reads();
    await page.getByRole("button", { name: "Refresh mail", exact: true }).click();
    await expect.poll(state.reads).toBeGreaterThan(before);
    await expect(checkbox(page, 0)).toHaveCount(0); await expect(checkbox(page, 1)).toHaveCount(0);
  }
  state.settle(1, "applied");
  await expect.poll(state.reads).toBeGreaterThan(3);
  await expect(checkbox(page, 0)).toHaveCount(0);
  state.settle(0, "applied");
  await page.waitForTimeout(1200);
  await page.reload();
  await expect(checkbox(page, 0)).toHaveCount(0); await expect(checkbox(page, 1)).toHaveCount(0);
  await expect(checkbox(page, 2)).toBeVisible();
});

test("provider failure rolls back only that archive while a concurrent cross-account archive stays hidden", async ({ page }) => {
  const state = await fixture(page);
  await checkbox(page, 0).click(); await checkbox(page, 3).click();
  await page.getByRole("button", { name: "Archive", exact: true }).click();
  await expect.poll(() => state.operations.size).toBe(2);
  state.settle(0, "failed");
  await expect(checkbox(page, 0)).toBeVisible();
  await expect(page.getByRole("alert")).toHaveText("Archive could not be verified for “Archive fixture 0”. Refresh mail and check its current state before trying again.");
  await expect(checkbox(page, 3)).toHaveCount(0);
  state.settle(3, "applied");
  await page.waitForTimeout(1200);
  await page.getByRole("combobox", { name: "Account filter" }).selectOption(accounts[0].id);
  await expect(checkbox(page, 0)).toBeVisible();
  await expect(checkbox(page, 3)).toHaveCount(0);
});

test("pending server projection protects reload and paginated refetch without resurrecting duplicates", async ({ page }) => {
  const state = await fixture(page);
  state.paginate();
  await page.getByRole("button", { name: "Refresh mail", exact: true }).click();
  await archive(page, 0);
  state.projectPending();
  await page.reload();
  await expect(checkbox(page, 0)).toHaveCount(0);
  await expect(checkbox(page, 1)).toBeVisible();
  // The short first page now automatically loads its continuation.
  await expect(checkbox(page, 5)).toBeVisible();
  await expect(checkbox(page, 3)).toHaveCount(1);
  state.settle(0, "applied");
  await page.getByRole("button", { name: "Refresh mail", exact: true }).click();
  await expect(checkbox(page, 0)).toHaveCount(0);
});

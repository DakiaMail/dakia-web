import { test, expect, type Page } from "@playwright/test";
import type { Message } from "@dakia/api-types";
import { accounts, content, page as mail } from "./fixtures";
import type { CleanupIntent, CleanupJob, CleanupRequest } from "../src/cleanupJobs";

type Options = { count?: number; cleanup?: "held" | "lost" | "rejected" | "blocked" | "unaccepted"; slowSearch?: boolean; unsubscribe?: string };
async function fixture(page: Page, options: Options = {}) {
  const rows: Message[] = Array.from({ length: options.count ?? 48 }, (_, i) => ({ ...mail.rows[0], id: `cleanup-${i}`, provider_id: `p${i}`, unread: false, subject: `List email ${i}`, folder_ids: ["INBOX"] }));
  const outsiders: Message[] = [{ ...rows[0], id: "other-account", account_id: "personal", subject: "Same sender other account" }, { ...rows[0], id: "substring", sender: { email: "xsender0@example.org", name: "Other sender" }, subject: "Other sender" }];
  const batches: CleanupRequest[] = [];
  const jobs = new Map<string, CleanupJob>();
  let sends = 0, reads = 0, status: string | null = null;
  let accept!: () => void, release!: () => void;
  const gate = new Promise<void>(resolve => { accept = resolve; });
  const searchGate = new Promise<void>(resolve => { release = resolve; });
  let settled = false;
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url()), path = url.pathname, method = route.request().method();
    const respond = (value: unknown, status = 200) => route.fulfill({ status, contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "cleanup-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/sender-cleanups")) {
      if (method === "GET") { reads++; return respond([...jobs.values()].filter(job => path.includes(`/${job.account_id}/`))); }
      const request: CleanupRequest = route.request().postDataJSON();
      batches.push(request);
      expect(path).toBe("/api/accounts/work/sender-cleanups");
      expect(route.request().headers()["x-csrf-token"]).toBe("synthetic");
      for (const target of request.targets) {
        const row = rows.find(row => row.id === target.id)!;
        expect(target.expected_location).toEqual({ provider_id: row.provider_id, folder_ids: ["INBOX"], sender_email: row.sender.email });
      }
      if (options.cleanup === "rejected" || options.cleanup === "blocked") return respond({ code: options.cleanup === "blocked" ? "cleanup_move_unverified" : "revision_conflict" }, 409);
      if (options.cleanup === "unaccepted") return route.abort("failed");
      const selected = rows.filter(row => request.targets.some(t => t.id === row.id));
      const job: CleanupJob = { request, account_id: "work", sender: rows[0].sender.email, created_at: new Date().toISOString(), dismissed: false, applied: 0, failed: 0, pending: selected.length, failed_targets: [], projected_rows: selected.map(row => ({ ...row, folder_ids: ["Trash"] })) };
      jobs.set(request.id, job);
      if (options.cleanup === "held") await gate;
      if (options.cleanup === "lost") return route.abort("failed");
      return respond(job);
    }
    if (path.endsWith("/sender-mail")) {
      const start = Number(url.searchParams.get("after") || 0);
      return respond({ rows: rows.slice(start, start + 100), count: rows.length, cutoff: new Date().toISOString(), after: start + 100 < rows.length ? String(start + 100) : null });
    }
    if (path.endsWith("/unsubscribe")) {
      if (method === "GET") return respond({ kind: "one_click", url: "https://newsletter.example.org/unsubscribe", attempt_status: status });
      sends++;
      status = options.unsubscribe || "request_accepted";
      if (options.unsubscribe === "lost") { status = "request_accepted"; return route.abort("failed"); }
      if (status !== "request_accepted") return respond({ code: "unsubscribe_not_confirmed" }, 502);
      return route.fulfill({ status: 204 });
    }
    if (path === "/api/search" || path.endsWith("/messages")) {
      const senderSearch = url.searchParams.get("exclude_trash") === "true";
      if (senderSearch) {
        expect(url.searchParams.get("accounts")).toBe("work");
        if (options.slowSearch) await searchGate;
        return respond({ ...mail, rows: [...rows, ...outsiders], coverage: [{ account_id: "work", state: "complete", folder_ids: ["INBOX"] }], continuation: null });
      }
      const trash = url.searchParams.get("folder") === "Trash" || url.searchParams.get("q") === "folder:Trash";
      // Deliberately stale provider/list/search responses while work is pending.
      return respond({ ...mail, rows: [...rows, ...outsiders].filter(row => trash ? row.folder_ids.includes("Trash") : settled ? !row.folder_ids.includes("Trash") : true), continuation: null });
    }
    if (path.endsWith("/conversation")) return respond({ rows: rows.slice(0, 2), complete: true, continuation: null });
    if (path.includes("/messages/")) {
      const row = rows.find(row => path.endsWith(`/${row.id}`)) || rows[0];
      return respond({ ...content, message: row, attachments: [] });
    }
    if (method !== "GET") throw new Error(`Unexpected mutation: ${path}`);
    return respond([]);
  });
  await page.goto("/");
  await page.getByRole("tab", { name: "List", exact: true }).click();
  await page.getByRole("button", { name: /List email 0/ }).click();
  await page.getByRole("button", { name: "Unsubscribe", exact: true }).first().click();
  return { rows, jobs, batches, accept, release, sends: () => sends, reads: () => reads,
    settle(fail = false) {
      settled = true;
      for (const job of jobs.values()) {
        job.pending = 0; job.failed = fail ? 1 : 0; job.applied = job.request.targets.length - job.failed;
        job.failed_targets = fail ? [job.request.targets.at(-1)!.id] : [];
        job.projected_rows = [];
        for (const row of rows) if (job.request.targets.some(t => t.id === row.id) && !job.failed_targets.includes(row.id)) row.folder_ids = ["Trash"];
      }
    },
  };
}
async function silent(page: Page) {
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("complementary", { name: "Background sender cleanup" })).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByText(/pending in background|not verified|Saving background cleanup|Cleanup status unavailable|moved to Trash\./)).toHaveCount(0);
}
async function navigate(page: Page, name: string) {
  if (await page.getByRole("button", { name: "Open navigation" }).isVisible()) await page.getByRole("button", { name: "Open navigation" }).click();
  await page.getByRole("navigation").getByRole("button", { name, exact: true }).click();
}
for (const width of [390, 1480]) test(`48 selected emails move locally before acknowledgement, silent navigation and reload ${width}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 900 });
  await page.addInitScript(() => localStorage.setItem("dakia-theme", "dark"));
  const f = await fixture(page, { cleanup: "held" });
  await expect(page.getByRole("dialog")).toContainText("48 previous emails");
  expect(f.batches).toHaveLength(0);
  await page.getByRole("button", { name: "Move 48 emails to Trash" }).click();
  await silent(page);
  await expect(page.getByRole("article", { name: "Conversation", exact: true })).toHaveCount(0);
  await expect(page.locator(".message-open").filter({ hasText: "List email" })).toHaveCount(0);
  await expect(page.locator(".message-open").filter({ hasText: "Other sender" })).toBeVisible();
  await page.screenshot({ path: `artifacts/silent-cleanup/inbox-pending-${width}.png` });
  f.accept();
  await expect.poll(() => f.batches.length).toBe(1);
  await expect.poll(f.reads).toBe(3); // one acceptance reconciliation, no interval
  const reads = f.reads(); await page.waitForTimeout(2200); expect(f.reads()).toBe(reads);
  await navigate(page, "Trash");
  await expect(page.locator(".message-open").filter({ hasText: "List email 0" })).toBeVisible();
  await page.reload(); await silent(page);
  await navigate(page, "Trash");
  await expect(page.locator(".message-open").filter({ hasText: "List email 0" })).toBeVisible();
  await page.screenshot({ path: `artifacts/silent-cleanup/trash-pending-${width}.png` });
  await navigate(page, "Unified inbox");
  await page.locator(".header-refresh").click();
  await expect(page.locator(".message-open").filter({ hasText: "List email" })).toHaveCount(0);
  f.settle(); await page.reload(); await silent(page);
  expect(f.sends()).toBe(1); expect(f.batches).toHaveLength(1); expect(f.batches[0].targets).toHaveLength(48);
});
for (const mode of ["lost", "rejected", "blocked", "unaccepted"] as const) test(`${mode} cleanup retains internal evidence without alerts or replay`, async ({ page }) => {
  const f = await fixture(page, { count: 2, cleanup: mode });
  await page.getByRole("button", { name: "Move 2 emails to Trash" }).click();
  await silent(page); await expect.poll(() => f.batches.length).toBe(1);
  await page.waitForTimeout(200);
  const intents: CleanupIntent[] = await page.evaluate(() => JSON.parse(sessionStorage.getItem("dakia-cleanup-intents:cleanup-fixture") || "[]"));
  expect(intents).toHaveLength(1);
  if (mode === "rejected" || mode === "blocked") expect(intents[0].rejected).toBe(true);
  else expect(intents[0].error).toBe("acceptance_unknown");
  await page.reload(); await silent(page);
  if (mode === "rejected" || mode === "blocked") await expect(page.locator(".message-open").filter({ hasText: "List email 0" })).toBeVisible();
  else await expect(page.locator(".message-open").filter({ hasText: "List email" })).toHaveCount(0);
  expect(f.batches).toHaveLength(1); expect(f.sends()).toBe(1);
});
test("partial failure restores only failed mail, remains silent and never replays", async ({ page }) => {
  const f = await fixture(page, { count: 2 });
  await page.getByRole("button", { name: "Move 2 emails to Trash" }).click();
  await expect.poll(() => f.jobs.size).toBe(1); f.settle(true);
  await page.reload(); await silent(page);
  await expect(page.locator(".message-open").filter({ hasText: "List email 1" })).toBeVisible();
  await expect(page.locator(".message-open").filter({ hasText: "List email 0" })).toHaveCount(0);
  await page.reload(); expect(f.batches).toHaveLength(1);
});
test("200 cached targets paginate for read-only review and freeze before late search", async ({ page }) => {
  const f = await fixture(page, { count: 200, slowSearch: true });
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("button", { name: "Move 200 found emails to Trash" })).toBeEnabled();
  await dialog.getByRole("button", { name: "Review emails", exact: true }).click();
  await expect(dialog).toContainText("Page 1 of 20");
  await dialog.getByRole("button", { name: "Next page" }).click();
  await expect(dialog).toContainText("Page 2 of 20");
  await dialog.getByRole("listitem").first().getByRole("button").click();
  await expect(dialog.locator("pre")).toContainText("Hi Alex");
  expect(f.batches).toHaveLength(0);
  await page.getByRole("button", { name: "Move 200 found emails to Trash" }).click();
  f.release(); await silent(page);
  await expect.poll(() => f.batches.length).toBe(1); expect(f.batches[0].targets).toHaveLength(200);
});
for (const outcome of ["request_rejected", "not_sent", "result_unknown", "lost"]) test(`unsubscribe ${outcome} remains separate from cleanup consent`, async ({ page }) => {
  const f = await fixture(page, { count: 2, unsubscribe: outcome });
  if (outcome === "lost") await expect(page.getByRole("dialog")).toContainText("2 previous emails");
  else { await expect(page.getByRole("alert")).toBeVisible(); await expect(page.getByRole("button", { name: /Move 2/ })).toHaveCount(0); }
  expect(f.batches).toHaveLength(0); expect(f.sends()).toBe(1);
});

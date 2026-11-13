import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import type { Operation } from "@dakia/api-types";
import { accounts, content, page as mail } from "./fixtures";

async function fixture(page: Page, unread = false) {
  const rows = accounts.map(a => ({ ...mail.rows[0], account_id: a.id, id: "same", subject: `Quiet ${a.id}`, unread, starred: true }));
  const ops = new Map<string, Operation>();
  let releasePost!: () => void, releaseBody!: () => void;
  const postGate = new Promise<void>(r => releasePost = r);
  const bodyGate = new Promise<void>(r => releaseBody = r);
  const state = { holdPost: true, holdBody: false, failBody: false, loseResponse: false, projectPending: false, posts: 0, bodyReads: 0, reads: 0 };
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url()), path = url.pathname;
    const respond = (value: unknown, status = 200) => route.fulfill({ status, contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "quiet-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": fixture\n\n" });
    if (path.endsWith("/operations")) {
      state.posts++;
      const op = { ...route.request().postDataJSON(), status: "pending", revision: 2 } as Operation;
      ops.set(op.id, op);
      if (state.holdPost) await postGate;
      if (state.loseResponse) return route.abort("failed");
      return respond(op);
    }
    if (path.includes("/operations/")) return respond(ops.get(path.split("/").at(-1)!));
    if (path === "/api/search" || path.endsWith("/messages")) {
      state.reads++;
      return respond({ ...mail, rows: rows.filter(m => m.folder_ids.includes("INBOX") && (!state.projectPending || ![...ops.values()].some(op => op.account_id === m.account_id && op.status === "pending"))), continuation: null });
    }
    const message = rows.find(m => path.includes(`/accounts/${m.account_id}/`)) || rows[0];
    if (path.endsWith("/conversation")) return respond({ rows: [message], complete: true, continuation: null });
    if (path.endsWith("/unsubscribe")) return respond(null);
    if (path.includes("/messages/")) {
      state.bodyReads++;
      if (state.holdBody) await bodyGate;
      if (state.failBody) return respond({ message: "Message unavailable. Try again." }, 503);
      return respond({ ...content, message, html: "<p>Available synthetic message body.</p>", attachments: [] });
    }
    return respond([]);
  });
  await page.goto("/");
  await expect(page.locator(".message-open")).toHaveCount(2);
  return { state, rows, ops, releasePost, releaseBody,
    settle(account: string, status: "applied" | "failed") {
      for (const [id, op] of ops) if (op.account_id === account) {
        ops.set(id, { ...op, status, revision: 3 });
        if (status === "applied") {
          const row = rows.find(m => m.account_id === account)!;
          if (op.change.kind === "read") row.unread = !op.change.value;
          else row.folder_ids = [op.change.kind];
        }
      }
    },
  };
}
async function quiet(page: Page) {
  await expect(page.getByText(/Applying|messages? loaded|Checking for more messages|Loading message…/)).toHaveCount(0);
}
async function capture(page: Page, name: string) {
  mkdirSync("../../verification/optimistic-mail", { recursive: true });
  await page.screenshot({ path: `../../verification/optimistic-mail/${name}.png`, fullPage: true });
}
for (const width of [390, 1480]) {
  for (const action of ["Archive", "Move to trash"]) test(`immediate ${action} before response and independent partial failure ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    const f = await fixture(page);
    for (const a of accounts) await page.getByRole("checkbox", { name: `Select Quiet ${a.id}` }).check();
    await page.getByRole("button", { name: action, exact: true }).click();
    await expect.poll(() => f.state.posts).toBe(2);
    await expect(page.locator(".message-open")).toHaveCount(0);
    await expect(page.getByRole("button", { name: /selected · Clear/ })).toHaveCount(0);
    await quiet(page);
    await capture(page, `${width}-${action}-before-response`);
    f.releasePost();
    f.state.projectPending = true;
    const reads = f.state.reads;
    await page.locator(".header-refresh").click();
    await expect.poll(() => f.state.reads).toBeGreaterThan(reads);
    f.settle("work", "failed");
    await expect(page.getByRole("checkbox", { name: "Select Quiet work" })).toBeChecked();
    await expect(page.getByRole("button", { name: action, exact: true })).toBeEnabled();
    await expect(page.locator(".message-open").filter({ hasText: "Quiet personal" })).toHaveCount(0);
    await expect(page.getByRole("alert").filter({hasText:"Refresh mail and check"})).toBeVisible();
    await expect(page.locator(".dakia-toast-container")).toHaveAttribute("role", "alert");
    await expect(page.locator(".dakia-toast-container")).toContainText("couldn’t be updated");
    // Work is restored while Personal is still pending, not after Promise.all.
    expect([...f.ops.values()].find(o => o.account_id === "personal")!.status).toBe("pending");
    await capture(page, `${width}-${action}-partial-failure`);
    f.settle("personal", "applied");
    await expect.poll(() => page.evaluate(() => JSON.parse(sessionStorage.getItem("dakia-pending-operations:quiet-fixture") || "[]").length)).toBe(0);
    await page.reload();
    await expect(page.locator(".message-open")).toHaveCount(1);
    expect(f.state.posts).toBe(2);
  });
  test(`reader body appears immediately, slow and failed body remain actionable ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    const f = await fixture(page);
    f.state.holdBody = true;
    await page.locator(".message-open").filter({ hasText: "Quiet work" }).click();
    await expect(page.getByRole("status", { name: "Loading message body" })).toBeVisible();
    await quiet(page); await capture(page, `${width}-reader-loading`);
    await expect(page.getByText("This message is taking longer to load.")).toBeVisible({ timeout: 15000 });
    f.releaseBody();
    await expect(page.locator(".mail-body")).toBeVisible();
    await expect(page.getByText("This message is taking longer to load.")).toHaveCount(0);
    await quiet(page); await capture(page, `${width}-reader-ready`);
    // Reload drops cached content; explicit retry recovers a real body error.
    f.state.failBody = true;
    await page.reload();
    await page.locator(".message-open").filter({ hasText: "Quiet work" }).click();
    await expect(page.getByRole("alert")).toContainText("Message unavailable");
    await expect(page.getByRole("button", { name: "Retry message", exact: true })).toBeVisible();
    f.state.failBody = false;
    await page.getByRole("button", { name: "Retry message", exact: true }).click();
    await expect(page.locator(".mail-body")).toBeVisible();
    await expect(page.getByRole("alert")).toHaveCount(0);
  });
  test(`unknown acceptance restores rows and reload reads without replay ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    const f = await fixture(page);
    f.state.loseResponse = true;
    await page.getByRole("checkbox", { name: "Select Quiet work" }).check();
    await page.getByRole("button", { name: "Archive", exact: true }).click();
    await expect(page.locator(".message-open")).toHaveCount(1);
    f.releasePost();
    await expect(page.getByRole("alert").filter({hasText:"could not be verified"})).toBeVisible();
    await expect(page.locator(".message-open")).toHaveCount(2);
    await page.reload();
    await expect(page.locator(".message-open")).toHaveCount(1);
    f.settle("work", "applied");
    await expect.poll(() => page.evaluate(() => JSON.parse(sessionStorage.getItem("dakia-pending-operations:quiet-fixture") || "[]").length)).toBe(0);
    expect(f.state.posts).toBe(1);
    await quiet(page);
  });
}

test("body timeout offers a working explicit retry and never hides available content", async ({ page }) => {
  await page.clock.install();
  const f = await fixture(page);
  f.state.holdBody = true;
  await page.locator(".message-open").filter({ hasText: "Quiet work" }).click();
  await expect(page.getByRole("status", { name: "Loading message body" })).toBeVisible();
  await page.clock.fastForward(31000);
  await expect(page.getByRole("alert")).toContainText("This message couldn’t be loaded");
  f.state.holdBody = false;
  f.releaseBody();
  await page.getByRole("button", { name: "Retry message", exact: true }).click();
  await expect(page.locator(".mail-body")).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

for (const nav of ["Starred", "Search"]) test(`removal is immediate outside Inbox: ${nav}`, async ({ page }) => {
  const f = await fixture(page);
  if (nav === "Search") {
    await page.getByRole("textbox", { name: "Search all mail" }).fill("Quiet");
    await expect.poll(() => f.state.reads).toBeGreaterThan(1);
  } else await page.getByRole("button", { name: "Starred", exact: true }).click();
  await page.getByRole("checkbox", { name: "Select Quiet work" }).check();
  await page.getByRole("button", { name: "Move to trash", exact: true }).click();
  await expect(page.locator(".message-open").filter({ hasText: "Quiet work" })).toHaveCount(0);
  await expect(page.locator(".message-open").filter({ hasText: "Quiet personal" })).toBeVisible();
  f.releasePost(); f.settle("work", "applied");
  await expect.poll(() => page.evaluate(() => JSON.parse(sessionStorage.getItem("dakia-pending-operations:quiet-fixture") || "[]").length)).toBe(0);
});

test("automatic read does not block immediate archive or let older settlement restore it", async ({ page }) => {
  const f = await fixture(page, true);
  await page.locator(".message-open").filter({ hasText: "Quiet work" }).click();
  await expect.poll(() => f.state.posts).toBe(1);
  await page.getByRole("button", { name: "Archive", exact: true }).click();
  await expect(page.locator(".message-open").filter({ hasText: "Quiet work" })).toHaveCount(0);
  await quiet(page);
  f.releasePost();
  f.settle("work", "applied");
  await expect.poll(() => f.state.posts).toBe(2);
  await expect(page.locator(".message-open").filter({ hasText: "Quiet work" })).toHaveCount(0);
  f.settle("work", "applied");
  await expect.poll(() => page.evaluate(() => JSON.parse(sessionStorage.getItem("dakia-pending-operations:quiet-fixture") || "[]").length)).toBe(0);
  await expect(page.locator(".message-open").filter({ hasText: "Quiet work" })).toHaveCount(0);
});

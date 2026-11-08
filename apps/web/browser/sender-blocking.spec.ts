import { test, expect, type Page } from "@playwright/test";
import type { MailRule, Message } from "@dakia/api-types";
import { accounts, content, page as mail } from "./fixtures";
import type { CleanupJob, CleanupRequest } from "../src/cleanupJobs";

async function fixture(page: Page, bodyFails = false, previousCleanup = false) {
  let rules: MailRule[] = [];
  const requests: { account: string; request: CleanupRequest }[] = [];
  const jobs: CleanupJob[] = previousCleanup ? [{ request: { id: "previous-cleanup", source_id: "old", search_complete: true, targets: [{ id: "old", expected_revision: 1 }] }, account_id: "work", sender: mail.rows[0].sender.email, created_at: "2026-01-01T00:00:00Z", dismissed: false, pending: 0, applied: 1, failed: 0, failed_targets: [] }] : [];
  const base = { ...mail.rows[0], unread: false };
  const rows: Message[] = [base, { ...base, id: "personal-match", account_id: "personal", sender: { email: base.sender.email.toUpperCase(), name: "Same sender" }, subject: "Older personal email" }];
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url()), path = url.pathname, method = route.request().method();
    const respond = (value: unknown, status = 200) => route.fulfill({ status, contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "blocking-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path === "/api/rules") {
      if (method === "GET") return respond(rules);
      const request = route.request().postDataJSON();
      expect(request).toEqual({ preset: "block_sender", scope: "all_mailboxes", condition: { kind: "sender_is", email: base.sender.email }, actions: ["trash", "suppress_notifications"] });
      rules = [{ ...request, id: "rule-fixture", enabled: true, activated_at: new Date().toISOString() }]; return respond(rules[0]);
    }
    if (path === "/api/rules/rule-fixture") { rules[0].enabled = route.request().postDataJSON().enabled; return respond(rules[0]); }
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/sender-cleanups")) {
      const account = path.split("/")[3];
      if (method === "GET") return respond(jobs.filter(job => job.account_id === account));
      const request: CleanupRequest = route.request().postDataJSON(); requests.push({ account, request });
      if (account === "personal") return respond({ code: "cleanup_move_unverified", message: "A previous move could not be verified." }, 409);
      const job: CleanupJob = { request, account_id: account, sender: base.sender.email, created_at: new Date().toISOString(), dismissed: false, pending: request.targets.length, applied: 0, failed: 0, failed_targets: [] };
      jobs.push(job); return respond(job);
    }
    if (path === "/api/search" || path.endsWith("/messages")) {
      if (url.searchParams.get("exclude_trash") === "true") {
        const account = url.searchParams.get("accounts")!;
        return respond({ ...mail, rows: [...rows, { ...base, id: "substring", sender: { email: `x${base.sender.email}`, name: "Other" } }], coverage: [{ account_id: account, folder_ids: ["INBOX"], state: account === "personal" ? "limited" : "complete" }] });
      }
      return respond({ ...mail, rows });
    }
    if (path.endsWith("/conversation")) return respond({ rows: [path.includes("/personal/") ? rows[1] : base], complete: true, continuation: null });
    if (rows.some(row => path.endsWith(`/${row.id}`))) return respond(bodyFails ? { code: "unavailable", message: "Message unavailable" } : { ...content, message: base, attachments: [] }, bodyFails ? 503 : 200);
    if (path.endsWith("/unsubscribe")) return respond(null);
    return respond([]);
  });
  return { requests, getRules: () => rules };
}

async function senderMenu(page: Page) {
  await page.getByRole("button", { name: /^More actions for / }).first().press("Enter");
}

test("blocking works before body loading succeeds and does not clean up implicitly", async ({ page }) => {
  const state = await fixture(page, true); await page.goto("/");
  await page.locator(".message-open").first().click();
  await senderMenu(page);
  await page.getByRole("button", { name: "Block sender", exact: true }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Block sender", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Sender blocked", exact: true })).toBeVisible();
  expect(state.getRules()[0].enabled).toBe(true); expect(state.requests).toEqual([]);
});

test("older-mail review isolates exact senders and accounts and preserves partial failures", async ({ page }) => {
  const state = await fixture(page); await page.goto("/"); await page.locator(".message-open").first().click();
  await senderMenu(page);
  await page.getByRole("button", { name: "Block sender", exact: true }).click();
  const dialog = page.getByRole("dialog"); await dialog.getByRole("button", { name: "Block sender", exact: true }).click();
  await dialog.getByRole("button", { name: "Review older emails", exact: true }).click();
  const work = dialog.getByRole("region", { name: "Older emails in Work", exact: true });
  const personal = dialog.getByRole("region", { name: "Older emails in Personal", exact: true });
  await expect(work.getByRole("button", { name: "Move 1 email to Trash", exact: true })).toBeEnabled();
  await expect(personal.getByText("Search incomplete. Other emails may remain.")).toBeVisible();
  await work.getByRole("button", { name: "Move 1 email to Trash", exact: true }).click();
  await personal.getByRole("button", { name: "Move 1 email to Trash", exact: true }).click();
  await expect(personal.getByText("Cleanup was rejected. No move was accepted.")).toBeVisible();
  await expect(work.getByText("1 pending · 0 moved · 0 failed")).toBeVisible();
  expect(state.requests.map(row => [row.account, row.request.targets.map(t => t.id)])).toEqual([["work", ["m0"]], ["personal", ["personal-match"]]]);
  expect(state.getRules()[0].enabled).toBe(true);
  await page.keyboard.press("Escape"); await page.reload(); await page.locator(".message-open").first().click();
  await senderMenu(page);
  await expect(page.getByRole("button", { name: "Unblock sender", exact: true })).toBeVisible();
});

test("a previous completed cleanup does not prevent cleaning up newer emails", async ({ page }) => {
  const state = await fixture(page, false, true); await page.goto("/"); await page.locator(".message-open").first().click();
  await senderMenu(page);
  await page.getByRole("button", { name: "Block sender", exact: true }).click();
  const dialog = page.getByRole("dialog"); await dialog.getByRole("button", { name: "Block sender", exact: true }).click();
  await dialog.getByRole("button", { name: "Review older emails", exact: true }).click();
  const work = dialog.getByRole("region", { name: "Older emails in Work", exact: true });
  await work.getByRole("button", { name: "Move 1 email to Trash", exact: true }).click();
  await expect(work.getByText("1 pending · 0 moved · 0 failed")).toBeVisible();
  expect(state.requests[0].request.targets.map(row => row.id)).toEqual(["m0"]);
});

for (const theme of ["light", "dark"]) for (const viewport of [{ width: 1480, height: 900 }, { width: 390, height: 568 }, { width: 1024, height: 400 }]) {
  test(`sender actions stay in More with keyboard focus ${theme} ${viewport.width}x${viewport.height}`, async ({ page }, info) => {
    const state = await fixture(page, true);
    await page.setViewportSize(viewport);
    await page.addInitScript(theme => localStorage.setItem("dakia-theme", theme), theme);
    await page.goto("/"); await page.locator(".message-open").first().click();
    const more = page.getByRole("button", { name: /^More actions for / }).first();
    await expect(page.getByRole("button", { name: "Block sender", exact: true })).toHaveCount(0);
    // Reach the sender menu using Tab, then use only keys for the whole dialog journey.
    for (let n = 0; n < 100 && !await more.evaluate(el => el === document.activeElement); n++) await page.keyboard.press("Tab");
    await expect(more).toBeFocused(); await page.keyboard.press("Enter");
    const block = page.getByRole("button", { name: "Block sender", exact: true });
    await expect(block).toBeFocused();
    await page.screenshot({ path: info.outputPath("sender-more.png") });
    const bounds = await block.boundingBox(); expect(bounds!.x).toBeGreaterThanOrEqual(0); expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(viewport.width);
    await page.keyboard.press("ArrowDown"); await expect(block).toBeFocused();
    await page.keyboard.press("Escape"); await expect(more).toBeFocused();
    await page.keyboard.press("Enter"); await page.keyboard.press("Enter");
    const dialog = page.getByRole("dialog"); await expect(dialog.getByRole("button", { name: "Close", exact: true })).toBeFocused();
    await page.screenshot({ path: info.outputPath("sender-confirm.png") });
    await page.keyboard.press("e"); expect(state.getRules()).toEqual([]);
    await page.keyboard.press("Tab"); await page.keyboard.press("Tab"); await page.keyboard.press("Enter");
    await expect(dialog.getByRole("heading", { name: "Sender blocked", exact: true })).toBeVisible();
    await page.keyboard.press("Escape"); await expect(more).toBeFocused();
    await expect(page.getByRole("button", { name: "Unblock sender", exact: true })).toHaveCount(0);
    await page.reload(); await page.locator(".message-open").first().click(); await senderMenu(page);
    await expect(page.getByRole("button", { name: "Unblock sender", exact: true })).toBeFocused();
    await page.keyboard.press("Enter"); await page.keyboard.press("Tab"); await page.keyboard.press("Tab"); await page.keyboard.press("Enter");
    await expect(dialog.getByRole("heading", { name: "Sender unblocked", exact: true })).toBeVisible();
    await page.keyboard.press("Escape"); await expect(more).toBeFocused();
  });
}

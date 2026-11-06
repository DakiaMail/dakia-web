import { test, expect, type Page } from "@playwright/test";
import { accounts, content, messages, page as mail } from "./fixtures";
async function setup(page: Page) {
  const row = { ...messages[0], unread: false, subject: "Saved inbox fixture" };
  const sent = { ...row, id: "sent-cache", subject: "Saved sent fixture", folder_ids: ["Sent"] };
  let releaseLists = () => {}, releaseBodies = () => {}, releaseSession = () => {};
  const state = {
    authenticated: true, owner: "alice", authorized: [accounts[0]], freshSubject: row.subject,
    listGate: Promise.resolve(), bodyGate: Promise.resolve(), sessionGate: Promise.resolve(), lists: 0, bodies: 0,
    holdLists() { state.listGate = new Promise<void>(r => releaseLists = r); },
    holdBodies() { state.bodyGate = new Promise<void>(r => releaseBodies = r); },
    holdSession() { state.sessionGate = new Promise<void>(r => releaseSession = r); },
    releaseSession() { releaseSession(); },
    release() { releaseLists(); releaseBodies(); releaseSession(); },
  };
  page.on("close", state.release);
  await page.context().route("**/api/**", async route => {
    const url = new URL(route.request().url()), p = url.pathname;
    const respond = (value: unknown) => route.fulfill({ json: value });
    if (p === "/api/session") { await state.sessionGate; if (!state.authenticated) return route.fulfill({ status: 401, json: { code: "unauthorized", message: "Sign in" } }); return respond({ user_id: state.owner, csrf_token: "fixture-secret-not-for-storage" }); }
    if (p === "/api/accounts") return respond(state.authorized);
    if (p === "/api/logout") { state.authenticated = false; return respond(null); }
    if (p.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (p === "/api/search" || p.endsWith("/messages")) {
      state.lists++; await state.listGate;
      const isSent = url.searchParams.get("folder") === "Sent" || url.searchParams.get("q") === "folder:Sent";
      return respond({ ...mail, rows: isSent ? [sent] : [{ ...row, subject: state.freshSubject }], continuation: null });
    }
    if (p.endsWith("/conversation")) return respond({ rows: [row], complete: true, continuation: null });
    if (p.endsWith("/unsubscribe")) return respond(null);
    if (p.includes("/messages/")) { state.bodies++; await state.bodyGate; return respond({ ...content, message: row, html: "<p>Saved body fixture</p>", text: "Saved body fixture", attachments: [] }); }
    if (p.includes("/drafts/")) return respond({ ...route.request().postDataJSON(), revision: 1 });
    return respond([]);
  });
  await page.goto("/");
  await expect(page.getByText(row.subject, { exact: true })).toBeVisible();
  await expect.poll(() => state.bodies).toBeGreaterThan(0);
  await expect.poll(() => countSaved(page)).toBeGreaterThanOrEqual(2);
  return state;
}
async function countSaved(page: Page) {
  return page.evaluate(() => new Promise<number>(resolve => {
    const r = indexedDB.open("dakia-local-mail-v1");
    r.onsuccess = () => { const db = r.result; const count = db.transaction("entries").objectStore("entries").count(); count.onsuccess = () => { resolve(count.result); db.close(); }; };
  }));
}
test("reopening shows persisted inbox and body before held server responses; fresh data replaces the snapshot", async ({ page }, info) => {
  const state = await setup(page);
  state.holdLists(); state.holdBodies();
  const start = performance.now();
  await page.reload();
  await expect(page.getByText("Saved inbox fixture", { exact: true })).toBeVisible();
  const inboxMs = performance.now() - start;
  await page.locator(".message-open").first().focus(); await page.keyboard.press("Enter");
  await expect(page.frameLocator(".conversation iframe").locator("body")).toContainText("Saved body fixture");
  const bodyMs = performance.now() - start;
  state.freshSubject = "Fresh server subject"; state.release();
  await page.locator(".reader").focus(); await page.keyboard.press("Escape");
  await expect(page.getByText("Fresh server subject", { exact: true })).toBeVisible();
  await info.attach("persistent-reopen.json", { body: JSON.stringify({ inboxMs, bodyMs, serverResponsesHeldUntilVisible: true }), contentType: "application/json" });
});
test("saved folders switch immediately after reload while both folder requests are held", async ({ page }, info) => {
  const state = await setup(page);
  await page.getByRole("button", { name: "Sent", exact: true }).focus(); await page.keyboard.press("Enter");
  await expect(page.getByText("Saved sent fixture", { exact: true })).toBeVisible();
  await expect.poll(() => countSaved(page)).toBeGreaterThanOrEqual(3);
  state.holdLists(); await page.reload();
  await expect(page.getByText("Saved inbox fixture", { exact: true })).toBeVisible();
  const start = performance.now();
  await page.getByRole("button", { name: "Sent", exact: true }).focus(); await page.keyboard.press("Enter");
  await expect(page.getByText("Saved sent fixture", { exact: true })).toBeVisible();
  await info.attach("cached-folder.json", { body: JSON.stringify({ keyboardSwitchMs: performance.now() - start }), contentType: "application/json" });
  state.release();
});
test("saved mail is hidden until live authentication and never hydrates into another owner", async ({ page }) => {
  const state = await setup(page);
  state.holdSession(); state.holdLists(); await page.reload();
  await expect(page.getByText("Opening Dakia…", { exact: true })).toBeVisible();
  expect(await page.locator(".message-open").count()).toBe(0);
  state.owner = "bob"; state.releaseSession();
  await expect(page.getByRole("heading", { name: "Inbox", exact: true })).toBeVisible();
  await expect(page.getByText("Saved inbox fixture", { exact: true })).toHaveCount(0);
  state.release();
});
test("sign-out clears disk mail and notifies another open tab", async ({ page, context }) => {
  const state = await setup(page);
  const other = await context.newPage();
  await other.goto("/");
  await expect(other.getByText("Saved inbox fixture", { exact: true })).toBeVisible();
  await other.evaluate(() => {
    document.documentElement.dataset.revoked = "false";
    const channel = new BroadcastChannel("dakia-local-mail");
    channel.onmessage = () => { document.documentElement.dataset.revoked = "true"; };
  });
  await context.route("**/auth/login", route => route.fulfill({ body: "Signed out fixture" }));
  await page.getByRole("button", { name: "Sign out", exact: true }).focus(); await page.keyboard.press("Enter");
  await expect(page.getByText("Signed out fixture")).toBeVisible();
  expect(await countSaved(page)).toBe(0);
  await expect(other.getByRole("link", { name: "Sign in", exact: true })).toBeVisible();
  await page.waitForTimeout(300);
  expect(await countSaved(page)).toBe(0);
  state.release();
});
test("a mounted session can read its saved body offline without a network request", async ({ page }) => {
  await setup(page); await page.reload();
  await expect(page.locator(".message-open").first()).toBeVisible();
  await page.context().setOffline(true);
  await page.locator(".message-open").first().focus(); await page.keyboard.press("Enter");
  await expect(page.frameLocator(".conversation iframe").locator("body")).toContainText("Saved body fixture");
});

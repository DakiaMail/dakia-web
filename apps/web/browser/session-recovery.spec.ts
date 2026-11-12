import { test, expect, type Page } from "@playwright/test";
import { accounts, content, page as mail } from "./fixtures";

type Failure = 0 | 401 | 522 | "network";
async function fixture(page: Page, initial: Failure = 0) {
  let failure = initial, requests = 0;
  const message = { ...mail.rows[0], unread: false };
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    const respond = (json: unknown) => route.fulfill({ json });
    if (path === "/api/session") {
      requests++;
      if (failure === "network") return route.abort("connectionfailed");
      if (failure === 522) return route.fulfill({ status: 522, contentType: "text/html", body: "<h1>Connection timed out</h1>" });
      if (failure === 401) return route.fulfill({ status: 401, json: { code: "unauthorized", message: "Sign in" } });
      return respond({ user_id: "session-fixture", csrf_token: "synthetic" });
    }
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/sync-active")) return respond(false);
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows: [message], pending: false, continuation: null });
    if (path.endsWith("/conversation")) return respond({ rows: [message], complete: true, continuation: null });
    if (path.endsWith(`/messages/${message.id}`)) return respond({ ...content, message, attachments: [], html: "<p>Your open message stays here.</p>" });
    if (path.includes("/drafts/") && route.request().method() === "PUT") return respond({ ...route.request().postDataJSON(), revision: 1 });
    return respond([]);
  });
  return { message, fail(value: Failure) { failure = value; }, requests: () => requests };
}
async function recheckSession(page: Page) {
  await page.clock.install();
  await page.clock.fastForward(16000);
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
    window.dispatchEvent(new Event("visibilitychange"));
  });
}

for (const width of [390, 1480]) {
  test(`startup 522 offers connection recovery instead of sign-in at ${width}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    const state = await fixture(page, 522);
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "Can’t reach Dakia" })).toBeVisible();
    await expect(page.getByText(/Check your internet connection or VPN/)).toBeVisible();
    await expect(page.getByRole("link", { name: "Sign in", exact: true })).toHaveCount(0);
    await expect(page.getByText("Request failed (522)")).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath("connection-recovery.png"), fullPage: true });
    state.fail(0);
    await page.getByRole("button", { name: "Retry connection" }).click();
    await expect(page.getByRole("button", { name: "Compose", exact: true })).toBeVisible();
    expect(state.requests()).toBe(2);
  });
  test(`background 522 preserves an open reader and recovers at ${width}`, async ({ page }, testInfo) => {
    await page.setViewportSize({ width, height: 900 });
    const state = await fixture(page);
    await page.goto("/");
    await page.getByRole("tab", { name: "All", exact: true }).click();
    await page.locator(".message-open").filter({ hasText: state.message.subject }).click();
    const body = page.frameLocator('iframe[title="Message body"]').locator("body");
    await expect(body).toContainText("Your open message stays here.");
    state.fail(522);
    await recheckSession(page);
    await expect.poll(state.requests).toBe(2);
    await expect(page.getByRole("button", { name: "Retry connection" })).toBeVisible();
    await expect(body).toContainText("Your open message stays here.");
    await expect(page.getByRole("link", { name: "Sign in", exact: true })).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath("reader-connection-recovery.png"), fullPage: true });
    state.fail(0);
    await page.getByRole("button", { name: "Retry connection" }).click();
    await expect(page.getByRole("button", { name: "Retry connection" })).toHaveCount(0);
    await expect(body).toContainText("Your open message stays here.");
  });
}

test("a failed network request offers retry and recovers without authentication", async ({ page }) => {
  const state = await fixture(page, "network");
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Retry connection" })).toBeVisible();
  await expect(page.getByText(/Failed to fetch|Request failed/)).toHaveCount(0);
  state.fail(0);
  await page.getByRole("button", { name: "Retry connection" }).click();
  await expect(page.getByRole("button", { name: "Compose", exact: true })).toBeVisible();
});

test("offline startup explains the connection and resumes on reconnect", async ({ page }) => {
  const state = await fixture(page, "network");
  // A cached shell can open offline. Fail its API reads and expose offline state.
  await page.addInitScript(() => { Object.defineProperty(navigator, "onLine", { configurable: true, get: () => false }); });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "You’re offline" })).toBeVisible();
  await expect(page.getByRole("link", { name: "Sign in", exact: true })).toHaveCount(0);
  state.fail(0);
  await page.evaluate(() => {
    Object.defineProperty(navigator, "onLine", { configurable: true, get: () => true });
    window.dispatchEvent(new Event("online"));
  });
  await expect(page.getByRole("button", { name: "Compose", exact: true })).toBeVisible();
});

for (const cached of [false, true]) test(`confirmed 401 requires sign-in with cached session=${cached}`, async ({ page }) => {
  const state = await fixture(page, cached ? 0 : 401);
  await page.goto("/");
  if (cached) {
    await expect(page.getByRole("button", { name: "Compose", exact: true })).toBeVisible();
    state.fail(401); await recheckSession(page);
  }
  await expect(page.getByRole("link", { name: "Sign in", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Retry connection" })).toHaveCount(0);
  await expect(page.locator(".app")).toHaveCount(0);
});

test("a background connection failure preserves an unfinished draft", async ({ page }) => {
  const state = await fixture(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Compose", exact: true }).click();
  const subject = page.getByRole("textbox", { name: "Subject", exact: true });
  await subject.fill("Unfinished draft remains editable");
  state.fail(522); await recheckSession(page);
  await expect.poll(state.requests).toBe(2);
  await expect(subject).toHaveValue("Unfinished draft remains editable");
  await subject.fill("Still editing after connection loss");
  await expect(subject).toHaveValue("Still editing after connection loss");
  await expect(page.getByRole("link", { name: "Sign in", exact: true })).toHaveCount(0);
});

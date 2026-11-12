import { test, expect, type Page } from "@playwright/test";
import { accounts, content, page as mail } from "./fixtures";
async function fixture(page: Page, standalone = true) {
  await page.setViewportSize({ width: 390, height: 844 });
  if (standalone) await page.addInitScript(() => Object.defineProperty(navigator, "standalone", { configurable: true, value: true }));
  let saveFails = false, writes = 0;
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    const message = { ...mail.rows[0], unread: false };
    const respond = (json: unknown) => route.fulfill({ json });
    if (path === "/api/session") return respond({ user_id: "back-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/sync-active")) return respond(false);
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows: [message], pending: false, continuation: null });
    if (path.endsWith("/conversation")) return respond({ rows: [message], complete: true, continuation: null });
    if (path.endsWith(`/messages/${message.id}`)) return respond({ ...content, message, attachments: [], html: "<p>Back navigation fixture</p>" });
    if (path.includes("/drafts/") && route.request().method() === "PUT") {
      if (saveFails) return route.abort("connectionfailed");
      return respond({ ...route.request().postDataJSON(), revision: 1 });
    }
    if (route.request().method() === "POST") writes++;
    return respond([]);
  });
  return { saveFails(value: boolean) { saveFails = value; }, writes: () => writes };
}
const inbox = (page: Page) => page.locator(".mailbox");
const reader = (page: Page) => page.getByRole("region", { name: "Reading pane" });
async function openMessage(page: Page) {
  await page.getByRole("tab", { name: "All", exact: true }).click();
  await page.locator(".message-open").first().click();
  await expect(reader(page)).toBeVisible();
}
test("Back returns from a reader and Forward restores it without mail writes", async ({ page }) => {
  const state = await fixture(page); await page.goto("/"); await openMessage(page);
  await page.goBack(); await expect(inbox(page)).toBeVisible();
  await page.goForward(); await expect(reader(page)).toBeVisible();
  expect(state.writes()).toBe(0);
  expect(await page.evaluate(() => JSON.stringify(history.state))).not.toContain(mail.rows[0].subject);
});
test("installed root Back stays in Dakia without growing history", async ({ page }) => {
  await fixture(page); await page.goto("/"); await expect(inbox(page)).toBeVisible();
  const length = await page.evaluate(() => history.length);
  for (let i = 0; i < 4; i++) {
    await page.goBack(); await expect(inbox(page)).toBeVisible();
    await expect.poll(() => page.evaluate(() => history.state?.dakiaNavigation?.id)).not.toBe("boundary");
    expect(await page.evaluate(() => history.length)).toBe(length);
  }
});
test("notification deep link has an app list behind it, including after reload", async ({ page }) => {
  await fixture(page); await page.goto(`/?account=${accounts[0].id}&message=${mail.rows[0].id}&keep=1`);
  await expect(reader(page)).toBeVisible();
  const length = await page.evaluate(() => history.length);
  await page.reload(); await expect(reader(page)).toBeVisible();
  expect(await page.evaluate(() => history.length)).toBe(length);
  await page.goBack(); await expect(inbox(page)).toBeVisible();
  expect(new URL(page.url()).searchParams.has("message")).toBe(false);
  expect(new URL(page.url()).searchParams.get("keep")).toBe("1");
  await page.reload(); await expect(inbox(page)).toBeVisible();
});
test("Back closes mobile navigation before returning through mailbox and workspace views", async ({ page }) => {
  await fixture(page); await page.goto("/");
  await page.getByRole("button", { name: "Open navigation", exact: true }).click();
  await page.goBack(); await expect(page.locator(".app")).not.toHaveClass(/nav-open/);
  await page.getByRole("button", { name: "Open navigation", exact: true }).click();
  await page.getByRole("button", { name: "Sent", exact: true }).click();
  await expect(page.locator(".mailbox h1")).toHaveText("Sent");
  await page.goBack(); await expect(page.locator(".mailbox h1")).toHaveText("Inbox");
  await page.getByRole("button", { name: "Open navigation", exact: true }).click();
  await page.getByRole("button", { name: "Manage workspace", exact: true }).click();
  await expect(page.locator(".app")).toHaveClass(/workspace-view/);
  await page.goBack(); await expect(page.locator(".app")).not.toHaveClass(/workspace-view/);
});
test("Back preserves a draft whose save fails and retries only its save on close", async ({ page }) => {
  const state = await fixture(page); await page.goto("/");
  await page.getByRole("button", { name: "Compose", exact: true }).click();
  state.saveFails(true);
  await page.getByRole("textbox", { name: "Subject", exact: true }).fill("Keep my unfinished draft");
  await page.goBack();
  await expect(page.getByRole("textbox", { name: "Subject", exact: true })).toHaveValue("Keep my unfinished draft");
  await expect(page.getByRole("dialog").getByRole("alert")).toBeVisible();
  state.saveFails(false);
  await page.goBack(); await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(state.writes()).toBe(0);
});
test("normal browser history can leave the app after internal navigation is exhausted", async ({ page }) => {
  await fixture(page, false); await page.goto("/manifest.webmanifest"); await page.goto("/");
  await openMessage(page); await page.goBack(); await expect(inbox(page)).toBeVisible();
  await page.goBack(); await expect(page).toHaveURL(/manifest.webmanifest$/);
});
test("submitted search creates one Back step and restores the previous inbox", async ({ page }) => {
  await fixture(page); await page.goto("/");
  await page.getByRole("button", { name: "Open navigation", exact: true }).click();
  await page.getByRole("textbox", { name: "Search all mail" }).fill("Back navigation");
  await page.getByRole("textbox", { name: "Search all mail" }).press("Enter");
  await expect(page.locator(".mailbox h1")).toHaveText("Search");
  await page.clock.install(); await page.clock.runFor(1000);
  await page.goBack(); await expect(page.locator(".mailbox h1")).toHaveText("Inbox");
  await page.clock.runFor(1000); await expect(page.locator(".mailbox h1")).toHaveText("Inbox");
});

test("search debounce keeps mobile navigation open while typing", async ({ page }) => {
  await fixture(page); await page.goto("/");
  await page.getByRole("button", { name: "Open navigation", exact: true }).click();
  await page.clock.install();
  const search = page.getByRole("textbox", { name: "Search all mail" });
  await search.fill("First words"); await page.clock.runFor(400);
  await expect(page.locator(".app")).toHaveClass(/nav-open/);
  await expect(search).toBeFocused();
  await search.fill("First words continued"); await page.clock.runFor(400);
  await search.press("Enter"); await expect(page.locator(".app")).not.toHaveClass(/nav-open/);
  await expect(page.locator(".mailbox h1")).toHaveText("Search");
  // Each debounced query was committed while the navigation layer stayed open.
  // Back restores the preceding committed search, then the original inbox.
  await page.goBack(); await expect(page.getByRole("textbox", { name: "Search all mail" })).toHaveValue("First words");
  await page.goBack(); await expect(page.locator(".mailbox h1")).toHaveText("Inbox");
});

test("a late failed deep-link lookup cannot add an error after manual navigation", async ({ page }) => {
  await fixture(page);
  let pending: import("@playwright/test").Route | undefined;
  await page.route(`**/api/accounts/${accounts[0].id}/messages/${mail.rows[0].id}`, route => {
    if (!pending) { pending = route; return; }
    return route.fallback();
  });
  await page.goto(`/?account=${accounts[0].id}&message=${mail.rows[0].id}`);
  await expect.poll(() => !!pending).toBe(true);
  await openMessage(page);
  const failed = page.waitForEvent("requestfailed", request => request === pending!.request());
  await pending!.abort("connectionfailed"); await failed;
  await page.clock.install(); await page.clock.runFor(1000);
  await expect(reader(page)).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

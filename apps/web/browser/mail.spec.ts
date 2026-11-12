import { test, expect, type Page } from "@playwright/test";
import { Buffer } from "node:buffer";
import type { Submission, ScheduledMessage } from "@dakia/api-types";
import { accounts, content, page as mail, type FixtureState } from "./fixtures";
test.use({ timezoneId: "Europe/Tallinn" });
async function fixture(page: Page): Promise<FixtureState> {
  const state: FixtureState = { drafts: new Map(), submissions: [] };
  await page.route("**/api/**", async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;
    const method = route.request().method();
    const respond = (data: unknown, status = 200) =>
      route.fulfill({
        status,
        contentType: "application/json",
        body: JSON.stringify(data),
      });
    if (path === "/api/session")
      return respond({ user_id: "fixture-user", csrf_token: "fixture-csrf" });
    if (path === "/api/translation/settings")
      return respond({ mode: "host", can_manage: false });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events"))
      return route.fulfill({
        contentType: "text/event-stream",
        body: ": connected\n\n",
      });
    if (path === "/api/search" || path.endsWith("/messages"))
      return respond(url.searchParams.get("attention") === "true" ? { ...mail, rows: [], continuation: null } : mail);
    if (path.endsWith("/unsubscribe")) return respond(null);
    if (path.endsWith("/conversation")) return respond({ rows: [mail.rows.find(m => path.includes(`/messages/${m.id}/`)) || content.message], continuation: null, complete: true });
    if (path.includes("/messages/"))
      return respond({
        ...content,
        message_id: "<fixture-rfc@example.org>",
        references: ["<parent@example.org>"],
        cc: [{ email: "colleague@example.org", name: "Colleague" }],
        message: mail.rows.find((m) => path.endsWith(m.id)) || content.message,
      });
    if (path.endsWith("/operations")) {
      const op = route.request().postDataJSON();
      return respond({
        ...op,
        status: "applied",
        revision: op.expected_revision + 1,
      });
    }
    if (path.includes("/drafts/")) {
      const id = path.split("/").at(-1)!;
      if (method === "PUT") {
        const draft = route.request().postDataJSON();
        state.drafts.set(id, { ...draft, revision: draft.revision + 1 });
        return respond(state.drafts.get(id));
      }
      return respond(state.drafts.get(id));
    }
    if (path.endsWith("/drafts"))
      return respond(
        [...state.drafts.values()].filter((d) =>
          path.includes(`/accounts/${d.account_id}/`),
        ),
      );
    if (path.endsWith("/recipients"))
      return respond([
        { email: "trusted@example.org", name: "Trusted colleague" },
      ]);
    if (path.endsWith("/folders"))
      return respond(["INBOX", "Sent", "Projects", "Trash"]);
    if (path.endsWith("/settings")) return respond(method === "PUT" ? route.request().postDataJSON() : { full_imap_exposure_confirmed: false, sent_folder: null, archive_folder: null, trash_folder: null, received_cache_budget: 1073741824 });
    if (path.endsWith("/attachments"))
      return respond({
        id: "uploaded",
        filename: route.request().headers()["x-filename"],
        content_type: "text/plain",
        size_bytes: route.request().postDataBuffer()?.length || 0,
      });
    if (path.endsWith("/submissions") && method === "GET")
      return respond(state.submissions);
    if (path.endsWith("/scheduled")) return respond([]);
    if (path.endsWith("/submissions")) {
      const body = route.request().postDataJSON();
      const s: Submission = {
        ...body,
        id: "send1",
        account_id: "work",
        draft_revision: body.expected_revision,
        state: body.due_at ? "scheduled" : "claimed",
        filing_status: "pending",
        revision: 1,
      };
      state.submissions.push(s);
      return respond(s);
    }
    if (path.includes("/submissions/"))
      return respond(state.submissions.at(-1));
    return respond(
      { code: "not_found", message: `Fixture missing ${path}` },
      404,
    );
  });
  return state;
}
const discovered = { provider: "imap", imap_host: "imap.migadu.com", imap_port: 993, imap_tls: "implicit", smtp_host: "smtp.migadu.com", smtp_port: 465, smtp_tls: "implicit", username: "alex@example.org", source: "mx" };
test("send later review captures and keyboard, dismissal, preset and custom flows", async ({ page }) => {
  await page.clock.install({ time: new Date("2026-10-29T08:15:00Z") });
  const state = await fixture(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Compose", exact: true }).first().click();
  await page.getByRole("textbox", { name: "To", exact: true }).fill("recipient@example.org");
  await page.getByRole("textbox", { name: "Subject", exact: true }).fill("A few thoughts for tomorrow");
  await page.getByRole("textbox", { name: "Message", exact: true }).fill("Hi Maya,\n\nHere are my notes for tomorrow’s conversation.\n\nBest,\nAlex");
  const trigger = page.getByRole("button", { name: "Send later", exact: true });
  await expect(page.locator('input[type="datetime-local"]')).toHaveCount(0);
  for (const mobile of [false, true]) {
    await page.setViewportSize(mobile ? { width: 390, height: 844 } : { width: 1480, height: 980 });
    for (const theme of ["light", "dark"]) {
      await page.evaluate((theme) => document.documentElement.setAttribute("data-theme", theme), theme);
      const prefix = `artifacts/send-later-${mobile ? "mobile" : "desktop"}-${theme}`;
      await page.screenshot({ path: `${prefix}-closed.png`, fullPage: true, animations: "disabled" });
      await trigger.click();
      const panel = page.getByRole("dialog", { name: "Send later", exact: true });
      await expect(panel).toBeInViewport();
      await expect(panel.getByRole("button", { name: /Later today|This evening|Tomorrow/ }).first()).toBeFocused();
      await page.screenshot({ path: `${prefix}-menu.png`, fullPage: true, animations: "disabled" });
      await panel.getByRole("button", { name: "Pick date & time" }).click();
      await expect(page.getByLabel("Date", { exact: true })).toBeInViewport();
      await page.screenshot({ path: `${prefix}-custom.png`, fullPage: true, animations: "disabled" });
      await page.keyboard.press("Escape");
      await expect(trigger).toBeFocused();
      await expect(page.getByRole("dialog", { name: "New message", exact: true })).toBeVisible();
    }
  }
  await trigger.focus();
  await page.keyboard.press("Enter");
  await page.getByRole("button", { name: /Tomorrow/ }).click();
  expect(state.submissions).toHaveLength(0);
  await expect(page.getByRole("button", { name: "Schedule send", exact: true })).toBeVisible();
  await page.screenshot({ path: "artifacts/send-later-mobile-dark-selected.png", fullPage: true, animations: "disabled" });
  await page.getByRole("button", { name: /Change send time:/ }).click();
  await page.getByRole("button", { name: "Pick date & time", exact: true }).click();
  await page.getByLabel("Date", { exact: true }).fill("2026-10-29");
  await page.getByLabel("Time", { exact: true }).fill("00:01");
  await page.getByRole("button", { name: "Set send time" }).click();
  await expect(page.getByRole("alert")).toContainText("future date and time");
  expect(state.submissions).toHaveLength(0);
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Remove send time" }).click();
  await expect(page.getByRole("button", { name: "Send", exact: true })).toBeVisible();
  await trigger.click();
  await expect(page.getByRole("dialog", { name: "Send later", exact: true }).getByRole("button", { name: /Later today|This evening|Tomorrow/ }).first()).toBeFocused();
  await page.locator(".dakia-datetime-overlay").click({ position: { x: 3, y: 3 } });
  await expect(page.getByRole("dialog", { name: "Send later", exact: true })).toHaveCount(0);
  await expect(trigger).toBeFocused();
});
test("scheduled view shows useful pending mail and hides history and inbox-only controls", async ({ page }) => {
  await fixture(page);
  const submission = (id: string, state: Submission["state"], account = "work"): Submission => ({ id, account_id: account, operation_id: id, draft_id: "d1", draft_revision: 1, state, due_at: "2027-01-01T08:00:00Z", timezone: "Europe/Tallinn", filing_status: "pending", revision: 1 });
  const records: ScheduledMessage[] = [
    { submission: submission("scheduled1", "scheduled"), subject: "Notes for tomorrow", to: [{ name: "Maya", email: "maya@example.org" }] },
    { submission: submission("failed1", "failed"), subject: "Follow-up to review", to: [{ name: null, email: "review@example.org" }] },
  ];
  await page.route("**/api/accounts/*/scheduled", (route) => route.fulfill({ json: route.request().url().includes("/work/") ? records : [] }));
  await page.route("**/api/accounts/work/submissions/scheduled1", (route) => route.fulfill({ json: records[0].submission }));
  await page.goto("/");
  await page.getByRole("navigation", { name: "Mailboxes" }).getByRole("button", { name: "Scheduled", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Notes for tomorrow" })).toBeVisible();
  await expect(page.getByRole("tablist", { name: "Inbox view" })).toHaveCount(0);
  await expect(page.getByLabel("Loaded messages")).toHaveCount(0);
  await expect(page.locator(".scheduled-list")).not.toContainText("not_required");
  await expect(page.locator(".scheduled-list")).toContainText("Jan 1, 2027, 10:00 AM");
  for (const mobile of [false, true]) {
    await page.setViewportSize(mobile ? { width: 390, height: 844 } : { width: 1480, height: 980 });
    for (const theme of ["light", "dark"]) {
      await page.evaluate((theme) => document.documentElement.setAttribute("data-theme", theme), theme);
      await page.screenshot({ path: `artifacts/scheduled-${mobile ? "mobile" : "desktop"}-${theme}.png`, fullPage: true, animations: "disabled" });
    }
  }
  await page.getByRole("button", { name: "Manage schedule", exact: true }).click();
  await expect(page.getByRole("button", { name: "Edit scheduled" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Cancel send" })).toBeVisible();
  await page.getByRole("button", { name: "Dismiss send status" }).click();
  await page.getByRole("combobox", { name: "Account filter" }).selectOption("personal");
  await expect(page.getByRole("heading", { name: "No scheduled messages" })).toBeVisible();
  await page.screenshot({ path: "artifacts/scheduled-mobile-dark-empty.png", fullPage: true });
  await page.setViewportSize({ width: 1480, height: 980 });
  for (const name of ["Starred", "Snoozed", "Sent", "Drafts", "Trash"]) {
    await page.getByRole("navigation", { name: "Mailboxes" }).getByRole("button", { name, exact: true }).click();
    await expect(page.getByRole("tablist", { name: "Inbox view" })).toHaveCount(0);
  }
  await page.getByRole("navigation", { name: "Mailboxes" }).getByRole("button", { name: /Unified inbox/ }).click();
  await expect(page.getByRole("tablist", { name: "Inbox view" })).toBeVisible();
});
test("legacy image blocking is preserved, manual load remains, and preference persists across accounts", async ({ page }) => {
  await fixture(page);
  await page.addInitScript(() => { if (!localStorage.getItem("dakia:fixture-user:always-load-images")) localStorage.setItem("dakia:fixture-user:always-load-images", "false"); });
  let imageRequests = 0;
  const directRequests: string[] = [];
  page.on("request", (request) => { if (request.url().startsWith("https://images.example.org")) directRequests.push(request.url()); });
  await page.route("**/api/accounts/*/messages/*", (route) => {
    const id = new URL(route.request().url()).pathname.split("/").at(-1);
    return route.fulfill({ json: { ...content, message: mail.rows.find((row) => row.id === id), detected_language: "en", html: content.html + '<img src="https://images.example.org/photo.png" alt="Fixture photo"><script>fetch("https://images.example.org/script")</script><div style="background-image:url(https://images.example.org/css)">Safe content</div>' } });
  });
  await page.route("**/api/accounts/*/messages/*/image", (route) => {
    imageRequests++;
    expect(["https://images.example.org/photo.png", "https://images.example.org/css"]).toContain(route.request().postDataJSON().url);
    expect(route.request().headers()["x-csrf-token"]).toBe("fixture-csrf");
    return route.fulfill({ json: "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=" });
  });
  const maya = () => page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
  await page.goto("/");
  await maya();
  await expect(page.getByRole("button", { name: "Load images", exact: true })).toBeVisible();
  expect(imageRequests).toBe(0);
  await page.getByRole("button", { name: "Load images", exact: true }).click();
  await expect(page.frameLocator('iframe[title="Message body"]').locator("img")).toHaveAttribute("src", /^data:image\/png/);
  expect(imageRequests).toBe(2);
  await page.getByRole("button", { name: "Manage workspace" }).click();
  const preference = page.getByRole("checkbox", { name: "Block external images by default" });
  await expect(preference).toBeChecked();
  await preference.uncheck();
  await page.getByRole("button", { name: "Back to mail" }).click();
  await page.getByRole("button", { name: /Daniel Brooks.*Lunch/ }).click();
  await expect.poll(() => imageRequests).toBe(4);
  await page.reload();
  // Reader URLs now restore the selected email. Its allowed image loads once
  // before explicitly opening another message; this is not a duplicate fetch.
  await expect(page.getByRole("heading", { name: "Lunch on Thursday?", exact: true })).toBeVisible();
  await expect.poll(() => imageRequests).toBe(6);
  await maya();
  await expect.poll(() => imageRequests).toBe(8);
  await page.getByRole("button", { name: "Manage workspace" }).click();
  await expect(preference).not.toBeChecked();
  await preference.check();
  await page.reload();
  await maya();
  await expect(page.getByRole("button", { name: "Load images", exact: true })).toBeVisible();
  expect(imageRequests).toBe(8);
  await page.getByRole("button", { name: "Load images", exact: true }).click();
  await expect.poll(() => imageRequests).toBe(10);
  await page.getByRole("button", { name: /Daniel Brooks.*Lunch/ }).click();
  await expect(page.getByRole("button", { name: "Load images", exact: true })).toBeVisible();
  expect(imageRequests).toBe(10);
  expect(directRequests).toEqual([]);
});
test("workspace management consolidates accounts, settings and notification controls", async ({ page }) => {
  await fixture(page);
  await page.goto("/");
  await page.getByRole("navigation", { name: "Accounts" }).getByRole("button", { name: /Work/ }).click();
  await expect(page.locator(".sidebar").getByRole("button", { name: "Mailbox settings" })).toHaveCount(0);
  await expect(page.locator(".sidebar").getByRole("button", { name: "Remove account" })).toHaveCount(0);
  await page.getByRole("button", { name: "Manage workspace" }).click();
  const workspace = page.getByRole("main", { name: "Workspace management" });
  await expect(workspace.getByRole("heading", { name: "Workspace", exact: true })).toBeVisible();
  await expect(workspace.getByRole("article")).toHaveCount(2);
  for (const mobile of [false, true]) {
    await page.setViewportSize(mobile ? { width: 390, height: 844 } : { width: 1480, height: 980 });
    for (const theme of ["light", "dark"]) {
      await page.evaluate((theme) => document.documentElement.setAttribute("data-theme", theme), theme);
      await page.screenshot({ path: `artifacts/workspace-${mobile ? "mobile" : "desktop"}-${theme}.png`, fullPage: true, animations: "disabled" });
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    }
  }
  await workspace.getByRole("button", { name: "Enable notifications" }).click();
  // No browser permission or real subscription is granted by this fixture.
  await expect(workspace.getByRole("region", { name: "Notifications", exact: true }).getByRole("status")).not.toHaveText("Enabling…");
  await workspace.getByRole("button", { name: "Connect account" }).click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await workspace.getByRole("button", { name: "Back to mail" }).click();
  await expect(page.getByRole("combobox", { name: "Account filter" })).toHaveValue("work");
  await expect(page.getByRole("tab", { name: "Personal" })).toBeVisible();
});
test("mobile compact toolbar, FAB and real touch pull refresh have bounded requests", async ({ page, context }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await fixture(page);
  let searches = 0;
  page.on("request", (request) => { const url = new URL(request.url()); if (url.pathname === "/api/search" && !url.searchParams.has("attention")) searches++; });
  await page.goto("/");
  await expect(page.getByRole("tab", { name: "Personal" })).toBeVisible();
  await expect(page.locator(".header-refresh")).toBeVisible();
  await expect(page.getByLabel("Loaded messages", { exact: true })).toBeHidden();
  const menu = await page.getByRole("button", { name: "Open navigation" }).boundingBox();
  const tabs = await page.getByRole("tablist", { name: "Inbox view" }).boundingBox();
  expect(tabs!.y).toBeGreaterThan(menu!.y + menu!.height);
  expect(tabs!.x + tabs!.width).toBeLessThanOrEqual(390);
  const fab = page.locator(".list-compose");
  const box = await fab.boundingBox();
  expect(box!.y).toBeGreaterThan(700);
  for (const theme of ["light", "dark"]) {
    await page.evaluate((theme) => document.documentElement.setAttribute("data-theme", theme), theme);
    await page.screenshot({ path: `artifacts/inbox-compact-mobile-${theme}.png`, fullPage: true, animations: "disabled" });
  }
  const cdp = await context.newCDPSession(page);
  await cdp.send("Emulation.setTouchEmulationEnabled", { enabled: true });
  const list = page.locator(".message-list");
  const bounds = await list.boundingBox();
  const x = 180, y = bounds!.y + 80;
  async function swipe(dy: number, dx = 0) {
    await cdp.send("Input.dispatchTouchEvent", { type: "touchStart", touchPoints: [{ x, y }] });
    for (let i = 1; i <= 8; i++) await cdp.send("Input.dispatchTouchEvent", { type: "touchMove", touchPoints: [{ x: x + dx * i / 8, y: y + dy * i / 8 }] });
    await cdp.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
  }
  const before = searches;
  await swipe(160);
  await expect.poll(() => searches).toBe(before + 1);
  await expect(page.locator(".app")).not.toHaveClass(/reading/);
  await swipe(30);
  await swipe(30, 110);
  await page.waitForTimeout(100);
  expect(searches).toBe(before + 1);
  await list.evaluate((node) => { node.scrollTop = 300; });
  await swipe(160);
  await page.waitForTimeout(100);
  expect(searches).toBe(before + 1);
  await page.getByRole("button", { name: "Open navigation" }).click();
  await page.locator(".sidebar-refresh").click();
  await expect.poll(() => searches).toBe(before + 2);
  await fab.click();
  await expect(page.getByRole("dialog", { name: "New message" })).toBeVisible();
});
test("automatic setup discovers without credentials and permits edits before connecting", async ({ page }) => {
  await fixture(page);
  let sent: Record<string, unknown> | undefined;
  await page.route("**/api/accounts/discover", async (route) => {
    expect(route.request().postDataJSON()).toEqual({ email: "alex@example.org" });
    expect(route.request().headers()["x-csrf-token"]).toBe("fixture-csrf");
    await route.fulfill({ json: discovered });
  });
  await page.route("**/api/accounts", async (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    sent = route.request().postDataJSON();
    await route.fulfill({ json: accounts[0] });
  });
  await page.route("**/probe", (route) => route.fulfill({ json: accounts[0] }));
  await page.goto("/");
  await page.getByRole("button", { name: "Connect account", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Account name", { exact: true }).fill("Personal");
  await dialog.getByLabel("Password", { exact: true }).fill("synthetic-only-password");
  await dialog.getByLabel("Email", { exact: true }).fill("alex@example.org");
  await expect(dialog.getByText("Server settings found")).toBeVisible();
  await expect(dialog.locator('[name="imap_host"]')).toBeHidden();
  await page.waitForTimeout(250);
  await page.screenshot({ path: "artifacts/autodiscovery-light.png", fullPage: true });
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  await page.screenshot({ path: "artifacts/autodiscovery-dark.png", fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: "artifacts/autodiscovery-mobile.png", fullPage: true });
  await expect(dialog.getByRole("button", { name: "Connect account", exact: true })).toBeInViewport();
  await dialog.getByRole("button", { name: "Edit settings" }).click();
  await expect(dialog.locator('[name="imap_host"]')).toHaveValue("imap.migadu.com");
  await dialog.locator('[name="smtp_port"]').fill("587");
  await dialog.locator('[name="smtp_tls"]').selectOption("starttls");
  await dialog.getByRole("button", { name: "Connect account", exact: true }).scrollIntoViewIfNeeded();
  await expect(dialog.locator('[name="smtp_tls"]')).toBeInViewport();
  await expect(dialog.getByRole("button", { name: "Connect account", exact: true })).toBeInViewport();
  await page.screenshot({ path: "artifacts/autodiscovery-manual-mobile.png", fullPage: true });
  await dialog.getByRole("button", { name: "Connect account", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  expect(sent).toMatchObject({ email: "alex@example.org", imap_host: "imap.migadu.com", imap_port: 993, imap_tls: "implicit", smtp_host: "smtp.migadu.com", smtp_port: 587, smtp_tls: "starttls", password: "synthetic-only-password" });
  expect(await page.evaluate(() => JSON.stringify({ localStorage, sessionStorage }))).not.toContain("synthetic-only-password");
});
test("unknown provider and discovery failures expose secure manual fallback", async ({ page }) => {
  await fixture(page);
  await page.route("**/api/accounts/discover", (route) => route.fulfill({ json: null }));
  await page.goto("/");
  await page.getByRole("button", { name: "Connect account", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Email", { exact: true }).fill("alex@unknown.example.org");
  await expect(dialog.getByText("Enter your email provider’s server settings.")).toBeVisible();
  await expect(dialog.locator('[name="imap_host"]')).toBeVisible();
  await expect(dialog.locator('[name="imap_port"]')).toHaveValue("993");
  await expect(dialog.locator('[name="smtp_tls"]')).toHaveValue("implicit");
  await dialog.locator('[name="imap_host"]').fill("manual.example.org");
  await page.waitForTimeout(650);
  await expect(dialog.locator('[name="imap_host"]')).toHaveValue("manual.example.org");
  await page.route("**/api/accounts/discover", (route) => route.abort("failed"));
  await dialog.getByLabel("Email", { exact: true }).fill("alex@failure.example.org");
  await expect(dialog.getByText("Enter your email provider’s server settings.")).toBeVisible();
  await expect(dialog.locator('[name="imap_host"]')).toHaveValue("");
});
test("stale discovery cannot replace a newer email or manual override", async ({ page }) => {
  await fixture(page);
  let finish: (() => void) | undefined;
  await page.route("**/api/accounts/discover", async (route) => {
    if (route.request().postDataJSON().email === "old@example.org") await new Promise<void>((resolve) => { finish = resolve; });
    await route.fulfill({ json: discovered }).catch(() => {});
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Connect account", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Email", { exact: true }).fill("old@example.org");
  await expect.poll(() => !!finish).toBe(true);
  // Manual override while the old request is still in flight.
  await dialog.getByLabel("Email", { exact: true }).fill("new@example.org");
  await expect(dialog.getByText("Server settings found")).toBeVisible();
  await dialog.getByRole("button", { name: "Edit settings" }).click();
  await dialog.locator('[name="imap_host"]').fill("manual.example.org");
  finish!();
  await page.waitForTimeout(650);
  await expect(dialog.getByLabel("Email", { exact: true })).toHaveValue("new@example.org");
  await expect(dialog.locator('[name="imap_host"]')).toHaveValue("manual.example.org");
});
for (const detected of ["en", "et", null, undefined]) {
  test(`translation controls respect reliable detection: ${detected}`, async ({ page }) => {
    await fixture(page);
    await page.addInitScript(() => localStorage.setItem("dakia:fixture-user:always-load-images", "false"));
    await page.route("**/api/accounts/work/messages/m0", (route) => route.fulfill({ json: { ...content, html: content.html + '<img src="https://images.example.org/photo.png">', detected_language: detected } }));
    await page.goto("/");
    await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
    await expect(page.frameLocator('iframe[title="Message body"]').getByText("Hi Alex,")).toBeVisible();
    if (detected !== "et") {
      await expect(page.getByRole("button", { name: "Translate to English", exact: true })).toHaveCount(0);
      await expect(page.getByLabel("Translate from", { exact: true })).toHaveCount(0);
      await expect(page.getByRole("button", { name: "Load images", exact: true })).toBeVisible();
      await page.screenshot({ path: "artifacts/english-reader-light.png", fullPage: true });
    } else {
      await expect(page.getByRole("button", { name: "Translate to English", exact: true })).toBeVisible();
      await expect(page.getByLabel("Translate from", { exact: true })).toHaveValue("et");
    }
  });
}
test("account removal requires exact confirmation and preserves the other account", async ({ page }) => {
  await fixture(page);
  let removed = false;
  let deletes = 0;
  await page.route("**/api/accounts", (route) => route.fulfill({ contentType: "application/json", body: JSON.stringify(removed ? accounts.filter((a) => a.id !== "work") : accounts) }));
  await page.route("**/api/accounts/work", async (route) => {
    expect(route.request().method()).toBe("DELETE");
    expect(route.request().headers()["x-csrf-token"]).toBe("fixture-csrf");
    expect(route.request().postDataJSON()).toEqual({ confirm_email: accounts[0].email });
    deletes++;
    removed = true;
    await route.fulfill({ status: 204 });
  });
  await page.goto("/");
  await page.getByRole("navigation", { name: "Accounts", exact: true }).getByRole("button", { name: /Work/ }).click();
  await page.getByRole("button", { name: "Manage workspace" }).click();
  await page.getByRole("article", { name: "Work", exact: true }).getByRole("button", { name: "Remove account", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Your email account and mail at the provider will not be deleted.");
  await expect(dialog.getByRole("button", { name: "Remove from Dakia" })).toBeDisabled();
  await dialog.getByLabel(`Type ${accounts[0].email} to confirm`).fill("wrong@example.org");
  await expect(dialog.getByRole("button", { name: "Remove from Dakia" })).toBeDisabled();
  await dialog.getByRole("button", { name: "Keep account" }).click();
  expect(deletes).toBe(0);
  await page.getByRole("article", { name: "Work", exact: true }).getByRole("button", { name: "Remove account", exact: true }).click();
  await dialog.getByLabel(`Type ${accounts[0].email} to confirm`).fill(accounts[0].email);
  await page.waitForTimeout(250);
  await page.screenshot({ path: "artifacts/account-removal-light.png", fullPage: true });
  await page.evaluate(() => document.documentElement.setAttribute("data-theme", "dark"));
  await page.screenshot({ path: "artifacts/account-removal-dark.png", fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: "artifacts/account-removal-mobile.png", fullPage: true });
  await expect(dialog.getByRole("button", { name: "Remove from Dakia" })).toBeInViewport();
  await dialog.getByRole("button", { name: "Remove from Dakia" }).click();
  await expect(dialog).toHaveCount(0);
  await page.setViewportSize({ width: 1480, height: 980 });
  await expect(page.getByRole("navigation", { name: "Accounts", exact: true }).getByRole("button", { name: /Work/ })).toHaveCount(0);
  await expect(page.getByRole("navigation", { name: "Accounts", exact: true }).getByRole("button", { name: /Personal/ })).toBeVisible();
  expect(deletes).toBe(1);
});
test("account removal keeps the dialog and account on unresolved delivery", async ({ page }) => {
  await fixture(page);
  await page.route("**/api/accounts/work", (route) => route.fulfill({ status: 409, contentType: "application/json", body: JSON.stringify({ code: "account_has_pending_actions", message: "account has pending actions" }) }));
  await page.goto("/");
  await page.getByRole("navigation", { name: "Accounts", exact: true }).getByRole("button", { name: /Work/ }).click();
  await page.getByRole("button", { name: "Manage workspace" }).click();
  await page.getByRole("article", { name: "Work", exact: true }).getByRole("button", { name: "Remove account", exact: true }).click();
  await page.getByRole("dialog").getByLabel(`Type ${accounts[0].email} to confirm`).fill(accounts[0].email);
  await page.getByRole("button", { name: "Remove from Dakia" }).click();
  await expect(page.getByRole("alert")).toContainText("unresolved delivery");
  await page.getByRole("button", { name: "Keep account" }).click();
  await expect(page.getByRole("navigation", { name: "Accounts", exact: true }).getByRole("button", { name: /Work/ })).toBeVisible();
});
test("desktop light, dark and composer screenshots", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await fixture(page);
  await page.goto("/");
  await expect(
    page.getByRole("heading", { name: "Inbox", exact: true }),
  ).toBeVisible();
  await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
  await expect(
    page.getByRole("heading", { name: "A few thoughts on the launch" }),
  ).toBeVisible();
  await page.screenshot({
    path: "artifacts/chrome-desktop-light.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: "Dark theme", exact: true }).click();
  await page.screenshot({
    path: "artifacts/chrome-desktop-dark.png",
    fullPage: true,
  });
  await page
    .getByRole("button", { name: "Compose", exact: true })
    .first()
    .click();
  await expect(page.getByRole("dialog")).toBeVisible();
  await page.screenshot({
    path: "artifacts/chrome-composer-dark.png",
    fullPage: true,
  });
  expect(errors).toEqual([]);
});
test("mailbox settings map discovered folders and require explicit Gmail exposure", async ({ page }) => {
  await fixture(page);
  await page.goto("/");
  for (const theme of ["light", "dark"]) {
    if (theme === "dark") await page.getByRole("button", { name: "Dark theme", exact: true }).click();
    if (theme === "light") await page.getByRole("button", { name: "Manage workspace" }).click();
    await page.getByRole("article", { name: "Personal", exact: true }).getByRole("button", { name: "Mailbox settings", exact: true }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    await page.getByRole("combobox", { name: "Sent folder", exact: true }).selectOption("Sent");
    await page.getByRole("combobox", { name: "Archive folder", exact: true }).selectOption("Projects");
    await page.getByRole("combobox", { name: "Trash folder", exact: true }).selectOption("Trash");
    const exposure = page.getByRole("checkbox");
    await expect(exposure).not.toBeChecked();
    await exposure.check();
    await page.getByLabel("Received content cache (MiB)", { exact: true }).fill("512");
    await expect(page.getByRole("dialog")).toHaveCSS("opacity", "1");
    await page.screenshot({ path: `artifacts/mailbox-settings-${theme}.png`, fullPage: true, animations: "disabled" });
    const saved = page.waitForRequest((request) => request.url().endsWith("/accounts/personal/settings") && request.method() === "PUT");
    await page.getByRole("button", { name: "Save settings", exact: true }).click();
    const request = await saved;
    expect(request.postDataJSON()).toEqual({ full_imap_exposure_confirmed: true, sent_folder: "Sent", archive_folder: "Projects", trash_folder: "Trash", received_cache_budget: 536870912 });
    expect(request.headers()["x-csrf-token"]).toBe("fixture-csrf");
    await expect(page.getByRole("dialog")).toHaveCount(0);
  }
});
test("compose saves server draft and schedules exact revision", async ({
  page,
}) => {
  const state = await fixture(page);
  await page.goto("/");
  await page
    .getByRole("button", { name: "Compose", exact: true })
    .first()
    .click();
  await page
    .getByRole("textbox", { name: "To", exact: true })
    .fill("recipient@example.org");
  await page
    .getByRole("textbox", { name: "Subject", exact: true })
    .fill("Dakia fixture draft");
  await page
    .getByRole("textbox", { name: "Message", exact: true })
    .fill("A real Squire editor.");
  await expect(
    page.getByRole("status").filter({ hasText: /^Saved$/ }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Send later", exact: true }).click();
  await page.getByRole("button", { name: "Pick date & time", exact: true }).click();
  await page.getByLabel("Date", { exact: true }).fill("2027-01-01");
  await page.getByLabel("Time", { exact: true }).fill("10:00");
  await page.getByRole("button", { name: "Set send time", exact: true }).click();
  await page
    .getByRole("button", { name: "Schedule send", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(state.submissions).toHaveLength(1);
  const draft = [...state.drafts.values()][0];
  expect(draft.to[0].email).toBe("recipient@example.org");
  expect(draft.html).toContain("A real Squire editor.");
  expect(state.submissions[0].draft_revision).toBe(draft.revision);
  expect(state.submissions[0].due_at).toBe("2027-01-01T08:00:00.000Z");
  expect(state.submissions[0].timezone).toBe("Europe/Tallinn");
});
test("mobile list, reader and navigation", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await fixture(page);
  await page.goto("/");
  await page.screenshot({
    path: "artifacts/chrome-mobile-list.png",
    fullPage: true,
  });
  await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
  await expect(
    page.getByRole("heading", { name: "A few thoughts on the launch" }),
  ).toBeVisible();
  await page.screenshot({
    path: "artifacts/chrome-mobile-reader.png",
    fullPage: true,
  });
  expect(
    await page.evaluate(
      () => document.documentElement.scrollWidth <= innerWidth,
    ),
  ).toBe(true);
  await page
    .getByRole("button", { name: "Back to inbox", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Open navigation", exact: true })
    .click();
  await expect(
    page.getByRole("navigation", { name: "Mailboxes" }),
  ).toBeVisible();
});

test("server draft reopen and recipient suggestions", async ({ page }) => {
  const state = await fixture(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Compose", exact: true }).click();
  await page.getByRole("textbox", { name: "To", exact: true }).fill("tru");
  await page.getByRole("option", { name: /Trusted colleague/ }).click();
  await page
    .getByRole("textbox", { name: "Subject", exact: true })
    .fill("Persistent draft");
  await page
    .getByRole("textbox", { name: "Message", exact: true })
    .fill("Draft survives reopening.");
  await expect(
    page.getByRole("status").filter({ hasText: /^Saved$/ }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Close", exact: true }).click();
  await page.getByRole("button", { name: "Drafts", exact: true }).click();
  await page.getByRole("button", { name: /Persistent draft/ }).click();
  await expect(
    page.getByRole("textbox", { name: "Message", exact: true }),
  ).toContainText("Draft survives reopening.");
  expect([...state.drafts.values()][0].to[0].email).toBe("trusted@example.org");
});
test("reply-all preserves RFC threading and inert persisted quote", async ({
  page,
}) => {
  const state = await fixture(page);
  await page.goto("/");
  await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
  await page.getByRole("button", { name: "Reply all", exact: true }).click();
  await expect(
    page.getByRole("textbox", { name: "Cc", exact: true }),
  ).toHaveValue("colleague@example.org");
  await page
    .getByRole("textbox", { name: "Message", exact: true })
    .fill("Thank you.");
  await expect(
    page.getByRole("status").filter({ hasText: /^Saved$/ }),
  ).toBeVisible();
  const draft = [...state.drafts.values()][0];
  expect(draft.account_id).toBe("work");
  expect(draft.in_reply_to).toBe("<fixture-rfc@example.org>");
  expect(draft.references).toEqual([
    "<parent@example.org>",
    "<fixture-rfc@example.org>",
  ]);
  expect(draft.html).toContain('data-dakia-quote="true"');
  expect(draft.text).toContain("The updated screens");
  await expect(
    page.getByRole("textbox", { name: "Message", exact: true }),
  ).not.toContainText("The updated screens");
});
test("forward attachments are deliberate and upload uses CSRF", async ({
  page,
}) => {
  const state = await fixture(page);
  await page.goto("/");
  await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
  await page.getByRole("button", { name: "Forward", exact: true }).click();
  await expect(
    page.getByRole("checkbox", { name: "Launch review.pdf" }),
  ).not.toBeChecked();
  await page.getByRole("checkbox", { name: "Launch review.pdf" }).check();
  const upload = page.waitForRequest(
    (r) => r.url().endsWith("/attachments") && r.method() === "POST",
  );
  await page.getByLabel("Attach files", { exact: true }).setInputFiles({
    name: "test.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("fixture attachment"),
  });
  expect((await upload).headers()["x-csrf-token"]).toBe("fixture-csrf");
  await expect(
    page.getByRole("status").filter({ hasText: /^Saved$/ }),
  ).toBeVisible();
  const d = [...state.drafts.values()].at(-1)!;
  expect(d.attachments.map((a) => a.filename)).toEqual([
    "Launch review.pdf",
    "test.txt",
  ]);
});
test("draft conflict preserves local edits and saves a separate copy", async ({
  page,
}) => {
  const state = await fixture(page);
  let fail = true;
  await page.route("**/drafts/*", async (route) => {
    if (route.request().method() === "PUT" && fail) {
      fail = false;
      return route.fulfill({
        status: 409,
        contentType: "application/json",
        body: JSON.stringify({
          code: "conflict",
          message: "Revision conflict",
        }),
      });
    }
    return route.fallback();
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Compose", exact: true }).click();
  await page
    .getByRole("textbox", { name: "Message", exact: true })
    .fill("Keep this local version.");
  await expect(
    page.getByText(
      "This draft changed in another tab. Your version is still here.",
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("textbox", { name: "Message", exact: true }),
  ).toContainText("Keep this local version.");
  await page
    .getByRole("button", { name: "Save my version as a new draft" })
    .click();
  await expect(
    page.getByRole("status").filter({ hasText: /^Saved$/ }),
  ).toBeVisible();
  expect([...state.drafts.values()][0].html).toContain(
    "Keep this local version.",
  );
});
test("lost submit response retries the same operation ID", async ({ page }) => {
  await fixture(page);
  const operations: string[] = [];
  await page.route("**/submissions", async (route) => {
    if (route.request().method() === "POST") {
      operations.push(route.request().postDataJSON().operation_id);
      if (operations.length === 1) return route.abort("failed");
    }
    return route.fallback();
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Compose", exact: true }).click();
  await page
    .getByRole("textbox", { name: "To", exact: true })
    .fill("recipient@example.org");
  await page
    .getByRole("textbox", { name: "Message", exact: true })
    .fill("One send.");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.getByRole("alert")).toBeVisible();
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(operations).toHaveLength(2);
  expect(operations[0]).toBe(operations[1]);
});
test("search retains pagination without provider diagnostics", async ({
  page,
}) => {
  await fixture(page);
  await page.route("**/api/search?**", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        ...mail,
        continuation: "next-page",
        counts_exact: false,
        classification: { classified: 1, total: 15 },
        coverage: [
          {
            account_id: "work",
            folder_ids: ["INBOX"],
            state: "complete",
            error: null,
          },
          {
            account_id: "personal",
            folder_ids: [],
            state: "error",
            error: "Provider unavailable",
          },
        ],
      }),
    }),
  );
  await page.goto("/");
  await page
    .getByRole("textbox", { name: "Search all mail" })
    .fill("old uncached subject");
  await expect(page.getByLabel("Search category")).toHaveCount(0);
  await expect(page.getByText("Provider search", { exact: true })).toHaveCount(0);
  await expect(page.getByText("Provider unavailable", { exact: false })).toHaveCount(0);
  await expect(page.getByText("Classified 1 of 15")).toHaveCount(0);
  await expect(page.getByLabel("Loaded messages")).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Load more", exact: true }),
  ).toBeVisible();
});
test("received malicious HTML only loads approved raster images through the authenticated proxy", async ({
  page,
}) => {
  await fixture(page);
  const external: string[] = [];
  let imageRequests = 0;
  await page.route("**/messages/m0/image", async (route) => {
    imageRequests++;
    expect(route.request().method()).toBe("POST");
    expect(route.request().headers()["x-csrf-token"]).toBe("fixture-csrf");
    expect(["https://evil.example/pixel", "https://evil.example/bg"]).toContain(route.request().postDataJSON().url);
    await route.fulfill({ contentType: "application/json", body: JSON.stringify("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=") });
  });
  page.on("request", (r) => {
    if (r.url().includes("evil.example")) external.push(r.url());
  });
  await page.route("**/messages/m0", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        ...content,
        html: '<script>parent.document.body.textContent="owned"</script><img src="https://evil.example/pixel"><form action="https://evil.example"><input></form><p style="background:url(https://evil.example/bg)">Safe message</p>',
      }),
    }),
  );
  await page.goto("/");
  await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
  await expect(
    page.frameLocator('iframe[title="Message body"]').getByText("Safe message"),
  ).toBeVisible();
  expect(external).toEqual([]);
  const image = page.frameLocator('iframe[title="Message body"]').locator("img");
  await expect(image).toHaveAttribute("src", /^data:image\/png;base64,/);
  expect(imageRequests).toBe(2);
  expect(external).toEqual([]);
  await page.screenshot({ path: "artifacts/remote-image-opt-in-light.png", fullPage: true });
  await expect(
    page.getByRole("heading", { name: "Inbox", exact: true }),
  ).toBeVisible();
});
test("translation uses server job and switches back to original", async ({
  page,
}) => {
  await fixture(page);
  await page.route("**/api/accounts/work/messages/m0", route => route.fulfill({ json: { ...content, detected_language: "et" } }));
  await page.route("**/messages/m0/translation?**", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify(
        route.request().method() === "POST"
          ? {
              id: "job1",
              account_id: "work",
              message_id: "m0",
              content_revision: 1,
              model_revision: "fixture",
              kind: "translate",
              status: "succeeded",
              deadline: "2027-01-01T00:00:00Z",
            }
          : { ...content, html: "<p>Hello Alex!</p>", text: "Hello Alex!" },
      ),
    }),
  );
  await page.route("**/jobs/job1", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        id: "job1",
        account_id: "work",
        message_id: "m0",
        content_revision: 1,
        model_revision: "fixture",
        kind: "translate",
        status: "succeeded",
        deadline: "2027-01-01T00:00:00Z",
      }),
    }),
  );
  await page.goto("/");
  await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
  await expect(page.getByRole("combobox", {name:"Translate from"})).toHaveValue("et");
  await page.getByRole("button", { name: "Translate to English", exact: true }).click();
  await expect(
    page
      .frameLocator('iframe[title="Translated message"]')
      .getByText("Hello Alex!"),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Show original", exact: true })
    .click();
  await expect(
    page.frameLocator('iframe[title="Message body"]').getByText("Hi Alex,"),
  ).toBeVisible();
});
test("service worker excludes private API from persistent caches", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/");
  await page.evaluate(async () => {
    await navigator.serviceWorker.register("/sw.js");
    await navigator.serviceWorker.ready;
  });
  await page.reload();
  await expect(
    page.getByRole("heading", { name: "Inbox", exact: true }),
  ).toBeVisible();
  const urls = await page.evaluate(async () => {
    const result: string[] = [];
    for (const name of await caches.keys())
      for (const request of await (await caches.open(name)).keys())
        result.push(request.url);
    return result;
  });
  expect(urls.some((url) => new URL(url).pathname.startsWith("/api"))).toBe(
    false,
  );
});
test("setup sends credentials only in CSRF-protected request body", async ({
  page,
}) => {
  await fixture(page);
  let payload: Record<string, unknown> | undefined;
  await page.route("**/api/accounts", async (route) => {
    if (route.request().method() !== "POST") return route.fallback();
    payload = route.request().postDataJSON();
    expect(route.request().url()).not.toContain("test-only-value");
    expect(route.request().headers()["x-csrf-token"]).toBe("fixture-csrf");
    return route.fulfill({
      contentType: "application/json",
      body: JSON.stringify(accounts[0]),
    });
  });
  await page.route("**/probe", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify(accounts[0]),
    }),
  );
  await page.goto("/");
  await page
    .getByRole("button", { name: "Connect account", exact: true })
    .click();
  await page.getByLabel("Account name", { exact: true }).fill("Fixture");
  await page.getByLabel("Email", { exact: true }).fill("owner@example.com");
  await page.getByRole("combobox", { name: "Provider", exact: true }).selectOption("gmail");
  await page
    .getByLabel("App password", { exact: true })
    .fill("test-only-value");
  await page
    .getByRole("button", { name: "Connect account", exact: true })
    .last()
    .click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(payload?.password).toBe("test-only-value");
  expect(await page.evaluate(() => JSON.stringify(localStorage))).not.toContain(
    "test-only-value",
  );
});
test("Gmail reconnect retries and edits the same account after a rejected probe", async ({ page }) => {
  await fixture(page);
  const gmail = { ...accounts[1], state: "reconnect" as const };
  let ready = false;
  let writes = 0;
  let probes = 0;
  await page.route("**/api/accounts", async (route) => {
    expect(route.request().method()).toBe("GET");
    return route.fulfill({ contentType: "application/json", body: JSON.stringify([accounts[0], { ...gmail, state: ready ? "ready" : "reconnect" }]) });
  });
  await page.route("**/api/accounts/personal", async (route) => {
    expect(route.request().method()).toBe("PUT");
    expect(route.request().headers()["x-csrf-token"]).toBe("fixture-csrf");
    const payload = route.request().postDataJSON();
    expect(payload.email).toBe(gmail.email);
    expect(payload.imap_host).toBe("imap.gmail.com");
    expect(payload.smtp_host).toBe("smtp.gmail.com");
    expect(payload.password).toBe(writes === 0 ? "first-fixture-only" : "replacement-fixture-only");
    writes += 1;
    return route.fulfill({ contentType: "application/json", body: JSON.stringify(gmail) });
  });
  await page.route("**/api/accounts/personal/probe", async (route) => {
    expect(route.request().method()).toBe("POST");
    expect(route.request().headers()["x-csrf-token"]).toBe("fixture-csrf");
    probes += 1;
    ready = probes === 3;
    return route.fulfill({ status: ready ? 200 : 401, contentType: "application/json", body: JSON.stringify(ready ? { ...gmail, state: "ready" } : { code: "imap_authentication_rejected", message: "imap authentication rejected" }) });
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Reconnect", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByLabel("Email", { exact: true })).toHaveValue(gmail.email);
  await dialog.getByLabel("App password", { exact: true }).fill("first-fixture-only");
  await dialog.getByRole("button", { name: "Connect account", exact: true }).click();
  await expect(dialog.getByText("imap authentication rejected", { exact: true })).toBeVisible();
  await expect(dialog.getByLabel("App password", { exact: true })).toHaveValue("");
  await expect(dialog.getByLabel("App password", { exact: true })).toBeDisabled();
  await dialog.getByRole("button", { name: "Retry connection", exact: true }).click();
  await expect.poll(() => probes).toBe(2);
  await dialog.getByRole("button", { name: "Edit connection", exact: true }).click();
  await dialog.getByLabel("App password", { exact: true }).fill("replacement-fixture-only");
  await dialog.getByRole("button", { name: "Connect account", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  expect(writes).toBe(2);
  expect(probes).toBe(3);
  expect(await page.evaluate(() => JSON.stringify({ local: localStorage, session: sessionStorage }))).not.toContain("fixture-only");
});
test("notification deep link selects the owned account and message", async ({
  page,
}) => {
  await fixture(page);
  await page.goto("/?account=work&message=m0");
  await expect(
    page.getByRole("heading", { name: "A few thoughts on the launch" }),
  ).toBeVisible();
  await expect(
    page.getByRole("combobox", { name: "Account filter" }),
  ).toHaveValue("work");
});
test("inbox tabs retain the reader and separate scroll positions", async ({
  page,
}) => {
  // Keep sufficient overflow for the 160px scroll in the compact list too.
  await page.setViewportSize({ width: 1480, height: 800 });
  await fixture(page);
  await page.goto("/");
  await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
  const list = page.getByLabel("Messages", { exact: true });
  await list.evaluate((el) => {
    el.scrollTop = 160;
  });
  await page.getByRole("tab", { name: "All", exact: true }).click();
  await expect(
    page.getByRole("heading", { name: "A few thoughts on the launch" }),
  ).toBeVisible();
  await expect.poll(() => list.evaluate((el) => el.scrollTop)).toBe(0);
  await list.evaluate((el) => { el.scrollTop = 80; });
  await page.getByRole("tab", { name: "Personal", exact: true }).click();
  await expect.poll(() => list.evaluate((el) => el.scrollTop)).toBe(160);
  await page.getByRole("tab", { name: "All", exact: true }).click();
  await expect.poll(() => list.evaluate((el) => el.scrollTop)).toBe(80);
});

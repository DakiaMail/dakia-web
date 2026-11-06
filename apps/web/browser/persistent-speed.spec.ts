import { test, expect } from "@playwright/test";
import { accounts, content, messages, page as mail } from "./fixtures";
test("measures reopening, saved-folder switching and saved body opening with fixed network latency", async ({ page }, info) => {
  const inbox = { ...messages[0], subject: "Reopen speed inbox", unread: false };
  const sent = { ...inbox, id: "speed-sent", subject: "Reopen speed sent", folder_ids: ["Sent"] };
  let delay = 0;
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url()), p = url.pathname;
    const respond = (value: unknown) => route.fulfill({ json: value });
    if (p === "/api/session") return respond({ user_id: "speed-fixture", csrf_token: "not-persisted" });
    if (p === "/api/accounts") return respond([accounts[0]]);
    if (p.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (p === "/api/search" || p.endsWith("/messages")) {
      if (delay) await new Promise(r => setTimeout(r, delay));
      return respond({ ...mail, rows: url.searchParams.get("folder") === "Sent" ? [sent] : [inbox] });
    }
    if (p.endsWith("/conversation")) return respond({ rows: [inbox], continuation: null, complete: true });
    if (p.endsWith("/unsubscribe")) return respond(null);
    if (p.includes("/messages/")) {
      if (delay) await new Promise(r => setTimeout(r, delay));
      return respond({ ...content, message: p.includes(sent.id) ? sent : inbox, html: "<p>Reopen speed body</p>", text: "Reopen speed body", attachments: [] });
    }
    return respond([]);
  });
  await page.goto("/");
  await page.locator(".message-open").first().focus(); await page.keyboard.press("Enter");
  await expect(page.frameLocator(".conversation iframe").locator("body")).toContainText("Reopen speed body");
  await page.locator(".reader").focus(); await page.keyboard.press("Escape");
  await expect(page.locator(".conversation")).toHaveCount(0);
  await expect(page.getByRole("region", { name: "Messages", exact: true })).toBeFocused();
  await page.getByRole("button", { name: "Sent", exact: true }).focus(); await page.keyboard.press("Enter");
  await expect(page.getByText(sent.subject, { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Unified inbox", exact: true }).focus(); await page.keyboard.press("Enter");
  await expect(page.getByText(inbox.subject, { exact: true })).toBeVisible();
  await page.waitForTimeout(500);
  delay = 600;
  const start = performance.now(); await page.reload();
  await expect(page.getByText(inbox.subject, { exact: true })).toBeVisible();
  const reopenMs = performance.now() - start;
  const folderStart = performance.now();
  await page.getByRole("button", { name: "Sent", exact: true }).focus(); await page.keyboard.press("Enter");
  await expect(page.getByText(sent.subject, { exact: true })).toBeVisible();
  const switchFolderMs = performance.now() - folderStart;
  await page.getByRole("button", { name: "Unified inbox", exact: true }).focus(); await page.keyboard.press("Enter");
  const bodyStart = performance.now();
  await page.locator(".message-open").first().focus(); await page.keyboard.press("Enter");
  await expect(page.frameLocator(".conversation iframe").locator("body")).toContainText("Reopen speed body");
  const result = { reopenMs, switchFolderMs, bodyMs: performance.now() - bodyStart, listAndBodyDelayMs: delay };
  console.log(JSON.stringify(result));
  await info.attach("persistent-speed.json", { body: JSON.stringify(result), contentType: "application/json" });
});

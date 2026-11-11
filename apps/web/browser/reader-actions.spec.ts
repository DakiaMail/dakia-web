import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { accounts, content, page as mail } from "./fixtures";

const rows = mail.rows.slice(0, 2).map((m, i) => ({ ...m, id: `toolbar-${i}`, subject: `Toolbar message ${i}`, unread: false }));
const actions = ["Archive", "Mark unread", "Snooze", "Move", "Move to trash", "Mark as spam"];
async function fixture(page: Page, empty = false) {
  const writes: unknown[] = [];
  await page.route("**/api/**", route => {
    const path = new URL(route.request().url()).pathname;
    const respond = (value: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "toolbar-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows: empty ? [] : rows });
    if (path.endsWith("/operations")) {
      const op = route.request().postDataJSON(); writes.push(op);
      return respond({ ...op, status: "applied", revision: 2 });
    }
    const message = rows.find(m => path.includes(`/messages/${m.id}`)) || rows[0];
    if (path.endsWith("/conversation")) return respond({ rows: [message], continuation: null, complete: true });
    if (path.includes("/messages/")) return respond({ ...content, message });
    return respond([]);
  });
  await page.goto("/");
  await expect(page.getByRole("tab", { name: "List", exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "List", exact: true }).click();
  return writes;
}
async function screenshot(page: Page, name: string) {
  const dir = resolve("artifacts/reader-actions"); mkdirSync(dir, { recursive: true });
  await page.evaluate(() => document.fonts.ready);
  await page.screenshot({ path: `${dir}/${name}.png`, fullPage: true, animations: "disabled" });
}
for (const width of [1480, 390]) for (const theme of ["light", "dark"]) {
  test(`contextual reader actions ${width} ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    await page.addInitScript(theme => localStorage.setItem("dakia-theme", theme), theme);
    const writes = await fixture(page);
    const reader = page.getByRole("region", { name: "Reading pane" });
    await expect(page.locator(".reader-toolbar")).toHaveCount(0);
    for (const name of actions) await expect(page.getByRole("button", { name, exact: true })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Compose", exact: true }).filter({ visible: true }).first()).toBeVisible();
    await screenshot(page, `${width}-${theme}-empty`);

    // Checkbox-only selection must retain bulk controls even with no preview.
    await page.getByRole("checkbox", { name: "Select Toolbar message 0", exact: true }).check();
    await page.getByRole("checkbox", { name: "Select Toolbar message 1", exact: true }).check();
    await expect(page.locator(".reader-toolbar")).toHaveCount(1);
    await expect(page.locator(".conversation-message")).toHaveCount(0);
    // Checkbox-only controls now live in the visible list pane on all sizes.
    await page.getByRole("button", { name: "More actions", exact: true }).click();
    for (const name of actions) await expect(page.getByRole(["Move", "Mark as spam"].includes(name) ? "menuitem" : "button", { name, exact: true })).toBeEnabled();
    await page.keyboard.press("Escape");
    await screenshot(page, `${width}-${theme}-multi`);
    await page.getByRole("button", { name: width === 390 ? "Clear selection (2 selected)" : "2 selected · Clear", exact: true }).click();
    await expect(page.locator(".reader-toolbar")).toHaveCount(0);

    await page.locator(".message-open").first().click();
    await expect(page.locator(".conversation-message")).toHaveCount(1);
    const toolbar = reader.locator(".reader-toolbar");
    await expect(toolbar.getByRole("button")).toHaveCount(5);
    await expect(page.getByRole("menuitem", { name: "Move", exact: true })).toHaveCount(0);
    for (const button of await page.getByRole("button").all()) {
      await expect(button).toHaveAttribute("title", /.+/);
    }
    for (const label of await toolbar.locator(".message-action-label").all()) {
      if ((await reader.boundingBox())!.width >= 560) await expect(label).toBeVisible();
      else await expect(label).toBeHidden();
    }
    await screenshot(page, `${width}-${theme}-toolbar`);
    await reader.getByRole("button", { name: "More actions" }).click();
    for (const name of actions) await expect(page.getByRole(["Move", "Mark as spam"].includes(name) ? "menuitem" : "button", { name, exact: true })).toBeEnabled();
    await expect(page.getByRole("menuitem", { name: "Category", exact: true })).toBeVisible();
    const more = page.getByRole("menu", { name: "More message actions" });
    await expect(more.getByRole("button", { name: /reply/i })).toHaveCount(0);
    await expect(more.getByRole("menuitem", { name: /^(Star|Unstar) selected messages$/ })).toHaveCount(1);
    await expect(more.getByRole("menuitem", { name: "Move", exact: true }).locator(".lucide-folder-input")).toHaveCount(1);
    await screenshot(page, `${width}-${theme}-selected`);
    await page.keyboard.press("Escape");
    await reader.getByRole("button", { name: "Archive", exact: true }).focus();
    await page.keyboard.press("Tab");
    await expect(reader.getByRole("button", { name: "Mark unread", exact: true })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect.poll(() => writes.length).toBe(1);
    expect(writes[0]).toMatchObject({ target_id: "toolbar-0", change: { kind: "read", value: false } });
    if (width === 390) await reader.getByRole("button", { name: "Back to inbox" }).click();
    else await page.getByRole("button", { name: "Starred", exact: true }).click();
    await expect(page.locator(".reader-toolbar")).toHaveCount(0);
    await expect(page.locator(".conversation-message")).toHaveCount(0);
  });
}
test("empty mailbox has no message controls but Compose remains usable", async ({ page }) => {
  await fixture(page, true);
  await expect(page.locator(".reader-toolbar")).toHaveCount(0);
  await page.getByRole("region", { name: "Reading pane" }).getByRole("button", { name: "Compose", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "New message" })).toBeVisible();
});

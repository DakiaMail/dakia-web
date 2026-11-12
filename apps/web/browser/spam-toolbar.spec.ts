import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { accounts, content, page as mail } from "./fixtures";
import type { Operation } from "@dakia/api-types";

async function fixture(page: Page, reject = false) {
  const rows = accounts.map(account => ({ ...mail.rows[0], account_id: account.id, id: "same-id", subject: `Spam fixture ${account.id}`, unread: false, revision: 1 }));
  const operations = new Map<string, Operation>();
  let reads = 0;
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    const respond = (value: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "spam-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/operations")) {
      if (reject) return route.fulfill({ status: 503, contentType: "application/json", body: JSON.stringify({ code: "unavailable", message: "Disposable spam provider unavailable" }) });
      const op: Operation = { ...route.request().postDataJSON(), status: "pending", revision: 2 };
      expect(path).toBe(`/api/accounts/${op.account_id}/operations`);
      operations.set(op.id, op); return respond(op);
    }
    if (path.includes("/operations/")) return respond(operations.get(path.split("/").at(-1)!));
    if (path === "/api/search" || path.endsWith("/messages")) {
      reads++;
      return respond({ ...mail, rows: rows.filter(row => ![...operations.values()].some(op => op.target_id === row.id && op.account_id === row.account_id && op.status === "applied")), continuation: null });
    }
    const message = rows.find(row => path.includes(`/accounts/${row.account_id}/`)) || rows[0];
    if (path.endsWith("/conversation")) return respond({ rows: [message], complete: true, continuation: null });
    if (path.includes("/messages/")) return respond({ ...content, message });
    return respond([]);
  });
  await page.goto("/");
  await page.getByRole("tab", { name: "All", exact: true }).click();
  return { operations, reads: () => reads, settle(status: "applied" | "failed") { for (const [id, op] of operations) operations.set(id, { ...op, status, revision: 3 }); } };
}
async function capture(page: Page, name: string) {
  mkdirSync("artifacts/spam-toolbar", { recursive: true });
  await page.screenshot({ path: `artifacts/spam-toolbar/${name}.png`, fullPage: true });
}
const toolbar = (page: Page) => page.locator(".reader-toolbar");
async function spam(page: Page) {
  await toolbar(page).getByRole("button", { name: "More actions", exact: true }).click();
  await page.getByRole("menuitem", { name: "Mark as spam", exact: true }).click();
}
for (const width of [320, 390, 768, 901, 1024, 1480]) for (const theme of ["light", "dark"]) {
  test(`toolbar layout keyboard and scoped spam ${width} ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    await page.addInitScript(theme => localStorage.setItem("dakia-theme", theme), theme);
    const state = await fixture(page);
    await page.locator(".message-open").filter({ hasText: "Spam fixture personal" }).click();
    for (const action of ["Archive", "Mark unread", "Move to trash"]) await expect(toolbar(page).getByRole("button", { name: action, exact: true })).toBeVisible();
    await expect(toolbar(page).getByRole("button").filter({ visible: true })).toHaveCount(5);
    const toolbarBounds = await toolbar(page).boundingBox();
    for (const button of await toolbar(page).getByRole("button").filter({ visible: true }).all()) {
      const bounds = await button.boundingBox();
      expect(bounds!.x).toBeGreaterThanOrEqual(toolbarBounds!.x);
      expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(toolbarBounds!.x + toolbarBounds!.width);
    }
    await capture(page, `${width}-${theme}-toolbar`);
    {
      const trigger = toolbar(page).getByRole("button", { name: "More actions" });
      await trigger.focus(); await page.keyboard.press("Enter");
      await expect(trigger).toHaveAttribute("aria-expanded", "true");
      await expect(page.getByRole("menuitem", { name: "Move", exact: true })).toBeFocused();
      const preserved = await page.getByRole("menuitem", { name: "Move", exact: true }).evaluate(button =>
        ["ctrlKey", "metaKey", "altKey", "shiftKey", "isComposing"].map(modifier => button.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true, [modifier]: true }))));
      expect(preserved).toEqual([true, true, true, true, true]);
      await page.keyboard.press("ArrowDown");
      await expect(page.getByRole("menuitem", { name: "Mark as spam", exact: true })).toBeFocused();
      await page.keyboard.press("ArrowDown");
      await expect(page.getByRole("menuitem", { name: "Personal or Automated", exact: true })).toBeFocused();
      await page.keyboard.press("ArrowRight");
      await expect(page.getByRole("menuitemradio").first()).toBeFocused();
      await page.keyboard.press("ArrowLeft");
      await page.keyboard.press("Escape");
      await expect(trigger).toBeFocused();
      await expect(trigger).toHaveAttribute("aria-expanded", "false");
      await expect(page.locator(".conversation-message")).toHaveCount(1);
      await trigger.click(); await capture(page, `${width}-${theme}-more`);
      const bounds = await page.getByRole("menu", { name: "More message actions" }).boundingBox();
      expect(bounds!.x).toBeGreaterThanOrEqual(8);
      expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(width - 8);
      await page.locator(".reader .pane-header > span").click();
      await expect(trigger).toHaveAttribute("aria-expanded", "false");
      // Tab exits normally, no focus trap or document-wide key listener.
      await trigger.click(); await page.keyboard.press("End");
      await expect(page.getByRole("menuitem", { name: "Block sender", exact: true })).toBeFocused();
      await page.keyboard.press("Tab");
      await expect(trigger).toHaveAttribute("aria-expanded", "false");
    }
    await spam(page);
    await expect(page.locator(".app")).not.toHaveClass(/reading/);
    await expect.poll(() => state.operations.size).toBe(1);
    expect([...state.operations.values()][0]).toMatchObject({ account_id: "personal", target_id: "same-id", change: { kind: "spam" }, status: "pending" });
    await expect(page.getByText(/Applying/)).toHaveCount(0);
    state.settle("applied");
    await expect.poll(state.reads).toBeGreaterThan(1);
    await expect(page.locator(".message-open").filter({ hasText: "Spam fixture personal" })).toHaveCount(0);
    await expect(page.locator(".message-open").filter({ hasText: "Spam fixture work" })).toBeVisible();
  });
}
for (const width of [390, 1480]) {
  test(`bulk spam pending and partial failure ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    const state = await fixture(page);
    for (const account of ["work", "personal"]) await page.getByRole("checkbox", { name: `Select Spam fixture ${account}` }).check();
    // Checkbox-only controls are visible in the list, including mobile.
    await expect(toolbar(page)).toBeVisible();
    await spam(page);
    await expect.poll(() => state.operations.size).toBe(2);
    expect([...state.operations.values()].map(op => op.account_id).sort()).toEqual(["personal", "work"]);
    await expect(toolbar(page)).toHaveCount(0);
    await expect(page.getByText(/Applying/)).toHaveCount(0);
    for (const [id, op] of state.operations) state.operations.set(id, { ...op, status: op.account_id === "personal" ? "applied" : "failed", revision: 3 });
    await expect(page.getByRole("alert")).toHaveText("Mark as spam could not be verified for “Spam fixture work”. Refresh and check Junk before trying again.");
    await expect(page.locator(".message-open").filter({ hasText: "Spam fixture work" })).toBeVisible();
    await expect(page.locator(".message-open").filter({ hasText: "Spam fixture personal" })).toHaveCount(0);
    expect(state.operations.size).toBe(2);
    await capture(page, `${width}-partial-failure`);
  });
  test(`spam immediate rejection restores row ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    await fixture(page, true);
    await page.locator(".message-open").first().click();
    await spam(page);
    await expect(page.getByRole("alert")).toContainText("Disposable spam provider unavailable");
    await expect(page.locator(".message-open")).toHaveCount(2);
  });
}

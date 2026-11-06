import { test, expect, type Page } from "@playwright/test";
import type { Operation } from "@dakia/api-types";
import { accounts, content, page as mail } from "./fixtures";

async function fixture(page: Page, failed = false) {
  const rows = accounts.map(a => ({ ...mail.rows[0], account_id: a.id, id: "same-id", subject: `Quick ${a.id}`, unread: false }));
  const operations: Operation[] = [];
  let release!: () => void;
  const gate = new Promise<void>(resolve => release = resolve);
  let bodyReads = 0;
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    const respond = (value: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "quick-actions-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/operations")) {
      const op: Operation = { ...route.request().postDataJSON(), status: "pending", revision: 2 };
      operations.push(op);
      await gate;
      op.status = failed ? "failed" : "applied";
      return respond(op);
    }
    if (path.includes("/operations/")) return respond(operations.find(op => path.endsWith(op.id)));
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows: rows.filter(row => !operations.some(op => op.account_id === row.account_id && op.status === "applied")), continuation: null });
    if (path.endsWith("/conversation")) return respond({ rows: [rows[0]], complete: true, continuation: null });
    if (path.endsWith("/unsubscribe")) return respond(null);
    if (path.includes("/messages/")) { bodyReads++; return respond({ ...content, message: rows[0] }); }
    return respond([]);
  });
  await page.goto("/");
  await page.getByRole("tab", { name: "List", exact: true }).click();
  await expect(page.locator(".message-row")).toHaveCount(2);
  return { operations, release, bodyReads: () => bodyReads };
}

for (const [kind, label] of [["archive", "Archive message"], ["trash", "Move message to trash"], ["spam", "Move message to spam"]] as const) {
  for (const failed of [false, true]) test(`${kind} acts only on hovered mail, optimistic ${failed ? "rollback" : "success"}`, async ({ page }) => {
    const f = await fixture(page, failed);
    const target = page.locator(".message-row").filter({ hasText: "Quick work" });
    const neighbor = page.locator(".message-row").filter({ hasText: "Quick personal" });
    await neighbor.getByRole("checkbox").check();
    await target.hover();
    await target.getByRole("button", { name: label, exact: true }).click();
    await expect(target).toHaveCount(0);
    await expect(neighbor.getByRole("checkbox")).toBeChecked();
    await expect.poll(() => f.operations.length).toBe(1);
    expect(f.operations[0]).toMatchObject({ account_id: "work", target_id: "same-id", change: { kind } });
    expect(f.bodyReads()).toBe(0);
    f.release();
    if (failed) {
      await expect(target).toBeVisible();
      await expect(target.getByRole("button", { name: label, exact: true })).toBeEnabled();
      await expect(page.getByRole("alert")).toBeVisible();
    } else {
      await expect.poll(() => page.evaluate(() => JSON.parse(sessionStorage.getItem("dakia-pending-operations:quick-actions-fixture") || "[]").length)).toBe(0);
      await expect(target).toHaveCount(0);
    }
  });
}

for (const width of [1024, 1480, 2560]) test(`hover and focus preserve column positions at ${width}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 980 });
  await fixture(page);
  await page.mouse.move(0, 0);
  const rows = page.locator(".message-row");
  const first = rows.first();
  const actions = first.locator(".row-quick-actions");
  await expect(actions).toHaveCSS("opacity", "0");
  const original = await first.locator(".account-label").boundingBox();
  const before = await first.locator(".subject").boundingBox();
  await first.hover();
  await expect(actions).toHaveCSS("opacity", "1");
  await expect(first.locator(".account-label")).toHaveCSS("opacity", "0");
  const box = (await actions.boundingBox())!;
  expect(Math.abs(box.x + box.width - original!.x - original!.width)).toBeLessThan(2);
  expect(await first.locator(".subject").boundingBox()).toEqual(before);
  await rows.nth(1).hover();
  await expect(actions).toHaveCSS("opacity", "0");
  const secondBox = (await rows.nth(1).locator(".row-quick-actions").boundingBox())!;
  expect(secondBox.x).toBe(box.x);
  expect(secondBox.width).toBe(box.width);
  await page.mouse.move(0, 0);
  await first.locator(".message-open").focus();
  await page.keyboard.press("Tab");
  await expect(first.getByRole("button", { name: "Archive message", exact: true })).toBeFocused();
  await expect(actions).toHaveCSS("opacity", "1");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(actions).toHaveCSS("transition-duration", "0s");
});

test("mobile keeps the account label and excludes hover controls", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await fixture(page);
  await expect(page.locator(".row-quick-actions").first()).toBeHidden();
  await page.locator(".message-open").first().focus();
  await expect(page.locator(".account-label").first()).toHaveCSS("opacity", "1");
  await page.keyboard.press("Tab");
  await expect(page.locator(".star").first()).toBeFocused();
});

for (const account of ["work", "personal"]) test(`archiving ${account} only closes its own reader`, async ({ page }) => {
  const f = await fixture(page);
  await page.locator(".message-open").filter({ hasText: "Quick work" }).click();
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  const target = page.locator(".message-row").filter({ hasText: `Quick ${account}` });
  await target.hover();
  await target.getByRole("button", { name: "Archive message", exact: true }).click();
  await expect(target).toHaveCount(0);
  await expect(page.locator(".conversation-message")).toHaveCount(account === "work" ? 0 : 1);
  f.release();
});

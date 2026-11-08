// Real Rust HTTP/auth/storage, with only a disposable provider boundary.
import { test, expect, type Page, type Locator } from "@playwright/test";
async function reach(page: Page, target: Locator) {
  for (let n = 0; n < 100; n++) {
    if (await target.evaluate(el => el === document.activeElement)) return;
    await page.keyboard.press("Tab");
  }
  throw new Error("Control could not be reached using Tab");
}
function senderMore(page: Page) { return page.getByRole("button", { name: "More actions", exact: true }).first(); }
async function senderMenu(page: Page) { await reach(page, senderMore(page)); await page.keyboard.press("Enter"); await page.keyboard.press("End"); }
test.beforeEach(async ({ context }) => {
  await context.addCookies([{ name: "__Host-dakia", value: "release-browser-fixture", url: "https://127.0.0.1:4327/", secure: true, sameSite: "Lax" }]);
});
for (const theme of ["light", "dark"]) for (const size of [{ width: 1480, height: 900 }, { width: 390, height: 568 }]) {
  test(`blocking persists and unblocking is keyboard accessible ${theme} ${size.width}`, async ({ page }, info) => {
    await page.setViewportSize(size);
    await page.addInitScript(theme => localStorage.setItem("dakia-theme", theme), theme);
    await page.goto("/");
    await expect(page.locator(".message-open").first()).toBeVisible();
    const row = page.locator(".message-open").filter({ hasText: "Synthetic 7" });
    await reach(page, row); await page.keyboard.press("Enter");
    await senderMenu(page);
    const trigger = page.getByRole("button", { name: "Block sender", exact: true }).first();
    await reach(page, trigger); await page.keyboard.press("Enter");
    let dialog = page.getByRole("dialog");
    await expect(dialog.getByRole("button", { name: "Close", exact: true })).toBeFocused();
    await dialog.screenshot({ animations: "disabled", path: info.outputPath("block-confirm.png") });
    // Mail shortcuts stay inside the dialog boundary.
    await page.keyboard.press("e"); await expect(dialog).toBeVisible();
    await reach(page, dialog.getByRole("button", { name: "Block sender", exact: true })); await page.keyboard.press("Enter");
    await expect(dialog.getByRole("heading", { name: "Sender blocked", exact: true })).toBeVisible();
    const review = dialog.getByRole("button", { name: "Review older emails", exact: true });
    await expect(review).toBeFocused(); await page.keyboard.press("Enter");
    await expect(dialog.getByText("Checking older emails…")).toHaveCount(0);
    await expect(dialog.getByRole("button", { name: /Move \d+ emails? to Trash/ })).toBeVisible();
    await dialog.screenshot({ animations: "disabled", path: info.outputPath("older-mail-review.png") });
    await page.keyboard.press("Escape"); await expect(dialog).toHaveCount(0);
    await expect(senderMore(page)).toBeFocused();
    await page.reload();
    await senderMenu(page);
    await expect(page.getByRole("button", { name: "Unblock sender", exact: true }).first()).toBeVisible();
    await page.keyboard.press("Escape");
    await page.keyboard.press("Escape");
    if (size.width < 600) { await reach(page, page.getByRole("button", { name: "Open navigation", exact: true })); await page.keyboard.press("Enter"); }
    await reach(page, page.getByRole("button", { name: "Manage workspace", exact: true })); await page.keyboard.press("Enter");
    const section = page.getByRole("region", { name: "Blocked senders", exact: true });
    await expect(section.getByText("fixture@example.invalid", { exact: true })).toBeVisible();
    await section.scrollIntoViewIfNeeded();
    if (size.width < 600) await expect.poll(() => page.locator(".sidebar").evaluate(node => node.getBoundingClientRect().right)).toBeLessThanOrEqual(0);
    await section.screenshot({ animations: "disabled", path: info.outputPath("blocked-settings.png") });
    const unblock = section.getByRole("button", { name: "Unblock fixture@example.invalid", exact: true });
    await reach(page, unblock); await page.keyboard.press("Enter");
    dialog = page.getByRole("dialog");
    await dialog.screenshot({ animations: "disabled", path: info.outputPath("unblock-confirm.png") });
    await reach(page, dialog.getByRole("button", { name: "Unblock sender", exact: true })); await page.keyboard.press("Enter");
    await expect(dialog).toHaveCount(0); await expect(page.getByRole("heading", { name: "Workspace", exact: true })).toBeFocused();
    await expect(section.getByText(/No blocked senders/)).toBeVisible(); await page.reload();
    await expect(page.locator(".message-open").first()).toBeVisible();
    if (size.width < 600) { await reach(page, page.getByRole("button", { name: "Open navigation", exact: true })); await page.keyboard.press("Enter"); }
    await reach(page, page.getByRole("button", { name: "Manage workspace", exact: true })); await page.keyboard.press("Enter");
    await expect(section.getByText(/No blocked senders/)).toBeVisible();
  });
}

test("sender block changes reach other open tabs", async ({ page, context }) => {
  const peer = await context.newPage();
  for (const tab of [page, peer]) {
    await tab.goto("/"); await tab.locator(".message-open").filter({ hasText: "Synthetic 7" }).click();
    await senderMenu(tab);
    await expect(tab.getByRole("button", { name: "Block sender", exact: true })).toBeVisible();
  }
  await page.getByRole("button", { name: "Block sender", exact: true }).click();
  await page.getByRole("dialog").getByRole("button", { name: "Block sender", exact: true }).click();
  await expect(peer.getByRole("button", { name: "Unblock sender", exact: true })).toBeVisible();
  await peer.getByRole("button", { name: "Unblock sender", exact: true }).click();
  await peer.getByRole("dialog").getByRole("button", { name: "Unblock sender", exact: true }).click();
  await page.keyboard.press("Escape");
  await senderMenu(page);
  await expect(page.getByRole("button", { name: "Block sender", exact: true })).toBeVisible(); await peer.close();
});

test("keyboard cleanup, restoration and unblocking persist through the real backend", async ({ page }, info) => {
  await page.goto("/");
  const row = page.locator(".message-open").filter({ hasText: "Synthetic 7" });
  await expect(row).toBeVisible(); await reach(page, row); await page.keyboard.press("Enter");
  await senderMenu(page);
  await reach(page, page.getByRole("button", { name: "Block sender", exact: true })); await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog");
  await reach(page, dialog.getByRole("button", { name: "Block sender", exact: true })); await page.keyboard.press("Enter");
  await expect(dialog.getByRole("button", { name: "Review older emails", exact: true })).toBeFocused(); await page.keyboard.press("Enter");
  const move = dialog.getByRole("button", { name: "Move 8 emails to Trash", exact: true });
  await expect(move).toBeEnabled(); await reach(page, move);
  const accepted = page.waitForResponse(r => r.url().endsWith("/sender-cleanups") && r.request().method() === "POST");
  await page.keyboard.press("Enter"); const job = await (await accepted).json(); expect(job.pending).toBe(8);
  await page.reload();
  await reach(page, page.getByRole("button", { name: "Trash", exact: true })); await page.keyboard.press("Enter");
  await expect(page.locator(".message-open").first()).toBeVisible();
  await expect.poll(async () => {
    const jobs = await (await page.request.get(`/api/accounts/${job.account_id}/sender-cleanups`)).json();
    return jobs.find((row: { request: { id: string } }) => row.request.id === job.request.id)?.applied;
  }).toBe(8);
  await page.reload(); await reach(page, page.getByRole("button", { name: "Trash", exact: true })); await page.keyboard.press("Enter");
  const restoredRow = page.locator(".message-open").filter({ hasText: "Synthetic 7" });
  await expect(restoredRow).toBeVisible(); await reach(page, restoredRow); await page.keyboard.press("Enter");
  await reach(page, page.getByRole("button", { name: "More actions", exact: true })); await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: "Move", exact: true })).toBeFocused(); await page.keyboard.press("Enter");
  await expect(dialog.getByRole("heading", { name: "Move messages", exact: true })).toBeVisible();
  await reach(page, dialog.getByRole("combobox", { name: "Folder for Fixture", exact: true }));
  await page.keyboard.press("ArrowDown"); await page.keyboard.press("i"); await page.keyboard.press("Tab");
  await expect(dialog.getByRole("combobox", { name: "Folder for Fixture", exact: true })).toHaveValue("INBOX");
  await expect(dialog.getByRole("button", { name: "Move", exact: true })).toBeEnabled();
  const operation = page.waitForResponse(r => r.url().endsWith("/operations") && r.request().method() === "POST");
  await reach(page, dialog.getByRole("button", { name: "Move", exact: true })); await page.keyboard.press("Enter");
  const op = await (await operation).json(); expect(op.change).toEqual({ kind: "move", folder_id: "INBOX" });
  await expect.poll(async () => (await (await page.request.get(`/api/accounts/${op.account_id}/operations/${op.id}`)).json()).status).toBe("applied");
  await page.reload();
  await expect(page.locator(".message-open").filter({ hasText: "Synthetic 7" })).toBeVisible();
  await reach(page, page.getByRole("button", { name: "Refresh mail", exact: true })); await page.keyboard.press("Enter");
  await expect(page.locator(".message-open").filter({ hasText: "Synthetic 7" })).toBeVisible();
  await page.screenshot({ animations: "disabled", path: info.outputPath("restored-inbox.png") });
  await reach(page, page.locator(".message-open").filter({ hasText: "Synthetic 7" })); await page.keyboard.press("Enter");
  await senderMenu(page);
  await reach(page, page.getByRole("button", { name: "Unblock sender", exact: true })); await page.keyboard.press("Enter");
  await reach(page, dialog.getByRole("button", { name: "Unblock sender", exact: true })); await page.keyboard.press("Enter");
  await expect(dialog.getByRole("heading", { name: "Sender unblocked", exact: true })).toBeVisible();
  await page.keyboard.press("Escape"); await page.reload();
  await expect(page.locator(".message-open").filter({ hasText: "Synthetic 7" })).toBeVisible();
});

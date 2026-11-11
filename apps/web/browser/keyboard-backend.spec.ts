// Real Rust HTTP/auth/storage and mutation queue. Only the provider is synthetic.
import { test, expect, type Page } from "@playwright/test";

test.beforeEach(async ({ context }) => {
  await context.addCookies([{ name: "__Host-dakia", value: "release-browser-fixture", url: "https://127.0.0.1:4327/", secure: true, sameSite: "Lax" }]);
});

async function action(page: Page, key: string, kind: string) {
  const accepted = page.waitForResponse(r => r.url().endsWith("/operations") && r.request().method() === "POST");
  await page.keyboard.press(key);
  const op = await (await accepted).json();
  expect(op.change.kind).toBe(kind);
  await expect.poll(async () => (await (await page.request.get(`/api/accounts/${op.account_id}/operations/${op.id}`)).json()).status).toBe("applied");
  await expect(page.locator(".message-row .star:disabled")).toHaveCount(0);
  return op;
}

test("compose field expansion stays keyboard-operable", async ({ page }, testInfo) => {
  await page.goto("/"); await expect(page.locator(".message-open").first()).toBeVisible(); await page.keyboard.press("c");
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("textbox", { name: "To", exact: true })).toBeFocused();
  await page.keyboard.type("reader@example.invalid"); await page.keyboard.press("Tab");
  await expect(dialog.getByRole("button", { name: "Cc / Bcc", exact: true })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(dialog.getByRole("textbox", { name: "Cc", exact: true })).toBeFocused();
  await page.keyboard.type("copy@example.invalid"); await page.keyboard.press("Tab");
  await expect(dialog.getByRole("textbox", { name: "Bcc", exact: true })).toBeFocused();
  await page.keyboard.press("Tab"); await expect(dialog.getByRole("textbox", { name: "Subject", exact: true })).toBeFocused();
  await page.keyboard.type("Keyboard field journey"); await page.keyboard.press("Tab");
  await expect(dialog.getByRole("button", { name: "Bold", exact: true })).toBeFocused();
  for (let i = 0; i < 6; i++) await page.keyboard.press("Tab");
  await expect(dialog.getByRole("textbox", { name: "Message", exact: true })).toBeFocused();
  await page.keyboard.type("All fields reached without a pointer. j k e stay in this draft.");
  await page.screenshot({ animations: "disabled", path: testInfo.outputPath("compose-field-journey.png") });
  await page.keyboard.press("Escape"); await expect(dialog).toHaveCount(0);
});

test("keyboard journey persists drafts and mail changes through the real backend", async ({ page }, testInfo) => {
  await page.goto("/");
  await page.getByRole("tab", { name: "List", exact: true }).focus(); await page.keyboard.press("Enter");
  const row = (n: number) => page.locator(".message-open").filter({ has: page.locator(".subject", { hasText: new RegExp(`^Synthetic ${n}$`) }) });
  await row(0).focus(); await page.keyboard.press("Enter");
  await page.keyboard.press("r");
  const editor = page.getByRole("dialog").getByRole("textbox", { name: "Message", exact: true });
  await expect(editor).toBeFocused();
  await page.keyboard.type("Keyboard journey draft: j k e u r stay in this message.");
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(row(0)).toBeFocused();
  const accounts = await (await page.request.get("/api/accounts")).json();
  const drafts = await (await page.request.get(`/api/accounts/${accounts[0].id}/drafts`)).json();
  expect(JSON.stringify(drafts)).toContain("Keyboard journey draft");
  await page.keyboard.press("Shift+r");
  await expect(editor).toBeFocused(); await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.keyboard.press("f");
  await expect(page.getByRole("dialog").getByRole("textbox", { name: "To", exact: true })).toBeFocused();
  await page.keyboard.press("Escape"); await expect(page.getByRole("dialog")).toHaveCount(0);
  await page.keyboard.press("/"); await expect(page.getByRole("textbox", { name: "Search all mail" })).toBeFocused();
  await page.keyboard.press("Escape"); await expect(page.getByRole("region", { name: "Messages", exact: true })).toBeFocused();
  const order = await page.locator(".message-row .subject").allTextContents();
  const index = order.indexOf("Synthetic 0");
  // Fixture IDs break equal-date ties and can place Synthetic 0 last.
  const direction = index < order.length - 1 ? "j" : "k";
  await page.keyboard.press(direction);
  await expect(page.locator(".conversation-title h2")).toHaveText(order[index + (direction === "j" ? 1 : -1)]);
  await expect(page.getByRole("region", { name: "Reading pane" })).toBeFocused();
  await row(1).focus(); await page.keyboard.press("Enter");
  await expect(page.locator(".conversation-title h2")).toHaveText("Synthetic 1");
  await action(page, "u", "read"); await page.keyboard.press("Escape"); await page.reload();
  await expect(row(1).locator("..")).toHaveClass(/unread/);
  await row(1).focus(); await action(page, "Shift+u", "read");
  await row(1).focus(); await action(page, "s", "star");
  await page.reload(); await expect(row(1).locator("..")).not.toHaveClass(/unread/);
  await expect(row(1).locator("..").getByRole("button", { name: "Unstar message", exact: true })).toBeVisible();
  await row(2).focus(); await action(page, "e", "archive"); await page.reload(); await expect(row(2)).toHaveCount(0);
  await row(3).focus(); await action(page, "Delete", "trash");
  await expect(page.locator(".message-open:focus")).toHaveCount(1);
  await page.reload(); await expect(row(3)).toHaveCount(0);
  await row(4).focus(); await action(page, "!", "spam"); await page.reload(); await expect(row(4)).toHaveCount(0);
  await row(5).focus(); await page.keyboard.press("m");
  await expect(page.getByRole("dialog", { name: "Move messages" })).toBeVisible();
  await expect(page.getByRole("combobox", { name: "Folder for Fixture", exact: true })).toBeEnabled();
  await page.screenshot({ animations: "disabled", path: testInfo.outputPath("move-dialog.png") });
  await page.keyboard.press("Escape"); await expect(row(5)).toBeFocused();
  await page.keyboard.press("b"); await expect(page.getByRole("dialog", { name: "Snooze until" })).toBeVisible();
  await expect(page.getByRole("dialog").getByRole("button", { name: /Later today|This evening|Tomorrow/ }).first()).toBeFocused();
  await page.screenshot({ animations: "disabled", path: testInfo.outputPath("snooze-dialog.png") });
  await page.keyboard.press("Escape"); await expect(row(5)).toBeFocused();
  await row(5).focus(); await page.keyboard.press("x"); await row(7).focus(); await page.keyboard.press("x");
  await expect(page.locator(".message-select input:checked")).toHaveCount(2);
  const requests: Promise<unknown>[] = [];
  page.on("response", r => { if (r.url().endsWith("/operations") && r.request().method() === "POST") requests.push(r.json()); });
  await page.keyboard.press("e");
  await expect(row(5)).toHaveCount(0); await expect(row(7)).toHaveCount(0);
  await expect.poll(() => requests.length).toBe(2);
  for (const op of await Promise.all(requests) as { account_id: string; id: string }[]) {
    await expect.poll(async () => (await (await page.request.get(`/api/accounts/${op.account_id}/operations/${op.id}`)).json()).status).toBe("applied");
  }
  await page.reload(); await expect(row(5)).toHaveCount(0); await expect(row(7)).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.screenshot({ animations: "disabled", path: testInfo.outputPath("real-keyboard-journey.png") });
});

import { test, expect, type Locator, type Page } from "@playwright/test";

async function tabTo(page: Page, name: string) {
  for (let i = 0; i < 350; i++) {
    if (await page.evaluate(name => {
      const el = document.activeElement;
      return el?.getAttribute("aria-label") === name || el?.getAttribute("title") === name || el?.textContent?.trim() === name;
    }, name)) return;
    await page.keyboard.press("Tab");
  }
  throw new Error(`Could not reach ${name}`);
}

async function tabToElement(page: Page, locator: Locator) {
  for (let i = 0; i < 350; i++) {
    if (await locator.evaluate(el => el === document.activeElement)) return;
    await page.keyboard.press("Tab");
  }
  throw new Error("Could not reach message");
}

async function waitForAppliedActions(page: Page) {
  // The picker closes optimistically. Reload only after the action is acknowledged.
  await expect.poll(() => page.evaluate(() => Object.entries(sessionStorage)
    .filter(([key]) => key.startsWith("dakia-pending-operations:"))
    .flatMap(([, value]) => JSON.parse(value)))).toEqual([]);
  await expect(page.getByRole("alert")).toHaveCount(0);
}

test("snooze keyboard presets and custom bulk selection persist after reload", async ({ page }) => {
  await page.goto("/");
  const row = page.locator(".message-open").first();
  await tabToElement(page, row);
  const firstSubject = await row.locator(".subject").innerText();
  await page.keyboard.press("b");
  let dialog = page.getByRole("dialog", { name: "Snooze until" });
  await expect(dialog.getByRole("button", { name: /Later today|This evening|Tomorrow/ }).first()).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(row).toBeFocused();
  await page.keyboard.press("b");
  await tabTo(page, "Tomorrow"); await page.keyboard.press("Enter");
  await expect(dialog).toHaveCount(0);
  await waitForAppliedActions(page);
  await page.reload();
  await expect(page.locator(".message-open").filter({ hasText: firstSubject })).toHaveCount(0);
  await tabTo(page, "Snoozed"); await page.keyboard.press("Enter");
  await expect(page.locator(".message-open").filter({ hasText: firstSubject })).toBeVisible();
  await tabToElement(page, page.getByRole("navigation", { name: "Mailboxes" }).getByRole("button", { name: /^Unified inbox/ })); await page.keyboard.press("Enter");
  const subjects: string[] = [];
  for (let i = 0; i < 2; i++) {
    const next = page.locator(".message-open").nth(i);
    subjects.push(await next.locator(".subject").innerText());
    await tabToElement(page, next);
    await page.keyboard.press("x");
  }
  await tabTo(page, "Snooze"); await page.keyboard.press("Enter");
  await tabTo(page, "Pick date & time"); await page.keyboard.press("Enter");
  dialog = page.getByRole("dialog", { name: "Pick date & time" });
  await expect(dialog.getByLabel("Date", { exact: true })).toBeFocused();
  await tabTo(page, "Back to time options"); await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: "Pick date & time" })).toBeFocused();
  await page.keyboard.press("Enter");
  // Native date/time segments vary by OS. Enter the year using its keyboard segment.
  await page.keyboard.press("ArrowRight"); await page.keyboard.press("ArrowRight");
  await page.keyboard.type("2099");
  await tabTo(page, "Snooze"); await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await waitForAppliedActions(page);
  await page.reload();
  await tabTo(page, "Snoozed"); await page.keyboard.press("Enter");
  for (const subject of subjects) await expect(page.locator(".message-open").filter({ hasText: subject })).toBeVisible();
});

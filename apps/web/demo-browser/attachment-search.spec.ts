import { test, expect, type Page } from "@playwright/test";

async function tabToFilter(page: Page) {
  const filter = page.getByRole("button", { name: "Has attachment", exact: true });
  for (let i = 0; i < 30; i++) {
    if (await filter.evaluate(el => el === document.activeElement)) return filter;
    await page.keyboard.press("Tab");
  }
  throw new Error("Attachment filter is not reachable by keyboard");
}

for (const width of [320, 1480]) test(`attachment search keyboard journey ${width}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 480 });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Inbox", exact: true })).toBeVisible();
  await page.keyboard.press("/");
  const search = page.getByRole("textbox", { name: "Search all mail" });
  await expect(search).toBeFocused();
  const filter = await tabToFilter(page);
  await page.keyboard.press("Enter");
  await expect(search).toHaveValue("has:attachment");
  await expect(filter).toHaveAttribute("aria-pressed", "true");
  await expect(filter).toBeFocused();
  await expect(page.getByRole("checkbox", { name: "Select A few thoughts on the launch", exact: true })).toBeVisible();
  await expect(page.getByRole("checkbox", { name: "Select Lunch on Thursday?", exact: true })).toHaveCount(0);
  await page.keyboard.press("Space");
  await expect(search).toHaveValue("");
  if (width <= 600) {
    await expect(page.getByRole("button", { name: "Open navigation" })).toBeFocused();
    await page.keyboard.press("/");
  } else {
    await expect(filter).toHaveAttribute("aria-pressed", "false");
    await expect(filter).toBeFocused();
    await page.keyboard.press("Shift+Tab");
  }
  await expect(search).toBeFocused();
  await page.keyboard.type('subject:"launch"');
  await tabToFilter(page);
  await page.keyboard.press("Enter");
  await expect(search).toHaveValue('subject:"launch" has:attachment');
  await expect(filter).toBeFocused();
  await expect(page.locator(".message-open")).toHaveCount(1);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.keyboard.press("Space");
  await expect(search).toHaveValue('subject:"launch"');
  await expect(filter).toHaveAttribute("aria-pressed", "false");
  await expect(filter).toBeFocused();
  await page.reload();
  await expect(page.getByRole("heading", { name: "Inbox", exact: true })).toBeVisible();
  if (width <= 600) {
    await page.keyboard.press("Tab");
    await expect(page.getByRole("button", { name: "Open navigation" })).toBeFocused();
    await page.keyboard.press("Enter");
    await page.keyboard.press("/");
    await expect(search).toBeFocused();
  }
  await tabToFilter(page);
  await page.keyboard.press("Enter");
  await expect(filter).toHaveAttribute("aria-pressed", "true");
});

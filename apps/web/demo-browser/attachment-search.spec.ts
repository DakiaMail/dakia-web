import { test, expect, type Page } from "@playwright/test";

async function tabToButton(page: Page, name: string) {
  const button = page.getByRole("button", { name, exact: true });
  for (let i = 0; i < 35; i++) {
    if (await button.evaluate(el => el === document.activeElement)) return;
    await page.keyboard.press("Tab");
  }
  throw new Error(`${name} is not reachable by keyboard`);
}
async function addAttachment(page: Page) {
  await tabToButton(page, "Search filters");
  await page.keyboard.press("Enter");
  const picker = page.getByRole("textbox", { name: "Find a filter" });
  await expect(picker).toBeFocused();
  await page.keyboard.type("attachment");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog", { name: "Search filters" })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Remove Has attachment filter" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Search filters", exact: true })).toBeFocused();
}
for (const width of [320, 1480]) test(`attachment search keyboard journey ${width}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 480 });
  await page.goto("/");
  await expect(page.locator(".message-open").first()).toBeVisible();
  const search = page.getByRole("combobox", { name: "Search all mail" });
  await page.keyboard.press("/");
  await expect(search).toBeFocused();
  await expect(page.locator('.sidebar [role="search"]')).toHaveCount(0);
  await addAttachment(page);
  await expect(page.getByRole("checkbox", { name: "Select A few thoughts on the launch", exact: true })).toBeVisible();
  await expect(page.getByRole("checkbox", { name: "Select Lunch on Thursday?", exact: true })).toHaveCount(0);
  await tabToButton(page, "Remove Has attachment filter");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: "Remove Has attachment filter" })).toHaveCount(0);
  await page.keyboard.press("/");
  await page.keyboard.type('subject:"launch"');
  await addAttachment(page);
  await expect(page.getByRole("button", { name: "Edit Subject launch filter" })).toBeVisible();
  await expect(page.locator(".message-open")).toHaveCount(1);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await tabToButton(page, "Remove Has attachment filter");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: "Edit Subject launch filter" })).toBeVisible();
  await page.reload();
  await expect(page.locator(".message-open").first()).toBeVisible();
  await page.keyboard.press("/");
  await expect(search).toBeFocused();
  await addAttachment(page);
});

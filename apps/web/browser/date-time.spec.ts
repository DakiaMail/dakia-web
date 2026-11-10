import { test, expect } from "@playwright/test";

test("library date-time selector works standalone with focus return and theme tokens", async ({ page }) => {
  await page.clock.install({ time: new Date("2026-11-10T08:15:00Z") });
  await page.goto("/ui-gallery.html");
  const opener = page.getByRole("button", { name: "Snooze", exact: true });
  await opener.focus(); await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Snooze until" });
  await expect(dialog.getByRole("button", { name: "Later today" })).toBeFocused();
  await page.evaluate(() => document.documentElement.style.setProperty("--color-accent", "rgb(22, 100, 80)"));
  expect(await dialog.getByRole("button", { name: "Later today" }).evaluate(el => getComputedStyle(el).outlineColor)).toBe("rgb(22, 100, 80)");
  await page.keyboard.press("Shift+Escape"); await expect(dialog).toBeVisible();
  await page.keyboard.press("Tab"); await page.keyboard.press("Tab"); await page.keyboard.press("Tab");
  await expect(dialog.getByRole("button", { name: "Pick date & time" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByLabel("Date", { exact: true })).toBeFocused();
  await page.getByLabel("Date", { exact: true }).fill("2026-11-10");
  await page.getByLabel("Time", { exact: true }).fill("00:01");
  await page.getByRole("button", { name: "Snooze", exact: true }).last().click();
  await expect(page.getByRole("alert")).toContainText("future date and time");
  await page.keyboard.press("Escape"); await expect(opener).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("dialog", { name: "Snooze until" })).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: "Send later", exact: true }).click();
  await page.getByRole("button", { name: /^Tomorrow/ }).click();
  await expect(page.getByRole("status").filter({ hasText: "Selected:" })).toHaveText("Selected: 2026-11-11T09:00");
  await expect(page.getByRole("button", { name: "Send later", exact: true })).toBeFocused();
});

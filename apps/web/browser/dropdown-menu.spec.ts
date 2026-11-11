import { test, expect } from "@playwright/test";

for (const dark of [false, true]) for (const viewport of [{ width: 1480, height: 980 }, { width: 360, height: 320 }]) {
  test(`shared menu keyboard navigation ${dark ? "dark" : "light"} ${viewport.width}`, async ({ page }, info) => {
    await page.setViewportSize(viewport);
    await page.addInitScript(dark => localStorage.setItem("dakia-theme", dark ? "dark" : "light"), dark);
    await page.goto("/ui-gallery.html");
    const trigger = page.getByRole("button", { name: "Actions", exact: true });
    await trigger.focus(); await page.keyboard.press("ArrowDown");
    const menu = page.getByRole("menu", { name: "Example actions", exact: true });
    const archive = menu.getByRole("menuitem", { name: "Archive", exact: true });
    const category = menu.getByRole("menuitem", { name: "Category", exact: true });
    const settings = menu.getByRole("menuitem", { name: "Settings", exact: true });
    await expect(archive).toBeFocused();
    await expect(trigger).toHaveAttribute("aria-haspopup", "menu");
    await expect(menu.getByRole("menuitem", { name: "Unavailable action" })).toBeDisabled();
    const preserved = await archive.evaluate(el => ["ctrlKey", "metaKey", "altKey", "shiftKey", "isComposing"].map(modifier =>
      el.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true, [modifier]: true }))));
    expect(preserved).toEqual([true,true,true,true,true]);
    await expect(archive).toBeFocused();
    const selectedTextPreserved = await archive.evaluate(el => {
      const range = document.createRange(); range.selectNodeContents(document.querySelector("h1")!);
      window.getSelection()!.removeAllRanges(); window.getSelection()!.addRange(range);
      const preserved = el.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true, cancelable: true }));
      window.getSelection()!.removeAllRanges(); return preserved;
    });
    expect(selectedTextPreserved).toBe(true); await expect(archive).toBeFocused();
    await page.keyboard.press("ArrowDown"); await expect(category).toBeFocused();
    await page.keyboard.press("ArrowDown"); await expect(settings).toBeFocused();
    await page.keyboard.press("ArrowUp"); await expect(category).toBeFocused();
    await page.keyboard.press("End"); await expect(settings).toBeFocused();
    await page.keyboard.press("ArrowDown"); await expect(archive).toBeFocused();
    await page.keyboard.press("s"); await expect(settings).toBeFocused();
    await page.keyboard.press("Home"); await expect(archive).toBeFocused();
    await page.keyboard.press("Shift+Escape"); await expect(menu).toBeVisible();
    await page.keyboard.press("ArrowDown"); await page.keyboard.press("ArrowRight");
    const submenu = page.getByRole("menu", { name: "Category", exact: true });
    await expect(submenu.getByRole("menuitemradio", { name: "People", exact: true })).toBeFocused();
    await expect(submenu.getByRole("menuitemradio", { name: "People", exact: true })).toHaveAttribute("aria-checked", "true");
    await page.screenshot({ path: info.outputPath("category-submenu.png") });
    for (const panel of [menu, submenu]) {
      const box = (await panel.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(7); expect(box.y).toBeGreaterThanOrEqual(7);
      expect(box.x+box.width).toBeLessThanOrEqual(viewport.width-7);
      expect(box.y+box.height).toBeLessThanOrEqual(viewport.height-7);
    }
    await page.keyboard.press("ArrowLeft"); await expect(category).toBeFocused(); await expect(submenu).toHaveCount(0);
    await page.keyboard.press("ArrowRight"); await expect(submenu.getByRole("menuitemradio").first()).toBeFocused(); await page.keyboard.press("Escape"); await expect(category).toBeFocused();
    await page.keyboard.press("ArrowRight"); await expect(submenu.getByRole("menuitemradio", { name: "People", exact: true })).toBeFocused(); await page.keyboard.press("ArrowDown"); await page.keyboard.press("Space");
    await expect(trigger).toBeFocused(); await expect(menu).toHaveCount(0);
    await expect(page.getByRole("status").filter({ hasText: "Notifications" })).toBeVisible();
    await page.keyboard.press("ArrowUp"); await expect(settings).toBeFocused();
    await page.keyboard.press("Tab"); await expect(menu).toHaveCount(0); await expect(trigger).not.toBeFocused();
    await trigger.focus(); await page.keyboard.press("Enter"); await expect(archive).toBeFocused();
    await page.keyboard.press("Shift+Tab"); await expect(menu).toHaveCount(0); await expect(trigger).not.toBeFocused();
    await trigger.focus(); await page.keyboard.press("Space"); await expect(archive).toBeFocused();
    await page.keyboard.press("Escape"); await expect(trigger).toBeFocused();
    await page.keyboard.press("Enter"); await page.screenshot({ path: info.outputPath("menu.png") });
    await page.getByRole("heading", { name: "Dropdown menus", exact: true }).click();
    await expect(menu).toHaveCount(0);
  });
}

test("hovered submenu accepts pointer selection without outside dismissal", async ({ page }) => {
  await page.goto("/ui-gallery.html");
  const trigger = page.getByRole("button", { name: "Actions", exact: true });
  await trigger.click();
  await page.getByRole("menuitem", { name: "Category", exact: true }).hover();
  const choice = page.getByRole("menuitemradio", { name: "Notifications", exact: true });
  await expect(choice).toBeVisible(); await choice.click();
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect(trigger).toBeFocused();
  await expect(page.getByRole("status").filter({ hasText: "Notifications" })).toBeVisible();
});

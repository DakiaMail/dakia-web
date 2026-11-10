import { test, expect, type Page } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";

async function fixture(page: Page) {
  let mode = "host";
  let fail = false;
  const writes: string[] = [];
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    const respond = (value: unknown) => route.fulfill({ json: value });
    if (path === "/api/session") return respond({ user_id: "ui-buttons-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path === "/api/translation/settings") {
      if (route.request().method() === "PUT") {
        writes.push(route.request().postDataJSON().mode);
        await new Promise(resolve => setTimeout(resolve, 250));
        if (fail) return route.fulfill({ status: 500, json: { error: "Synthetic save failure" } });
        mode = route.request().postDataJSON().mode;
      }
      return respond({ mode, can_manage: true });
    }
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond(mail);
    return respond([]);
  });
  return { writes, failNext: () => { fail = true; }, succeedNext: () => { fail = false; } };
}

async function tabTo(page: Page, selector: string) {
  for (let index = 0; index < 100; index++) {
    if (await page.locator(selector).evaluate(element => element === document.activeElement)) return;
    await page.keyboard.press("Tab");
  }
  throw new Error(`Could not reach ${selector} using Tab`);
}

for (const dark of [false, true]) for (const viewport of [{ width: 1480, height: 980 }, { width: 390, height: 500 }]) {
  test(`translation save keyboard journey ${dark ? "dark" : "light"} ${viewport.width}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.addInitScript(dark => localStorage.setItem("dakia-theme", dark ? "dark" : "light"), dark);
    const state = await fixture(page);
    await page.goto("/");
    if (viewport.width < 600) {
      await tabTo(page, '[aria-label="Open navigation"]');
      await page.keyboard.press("Enter");
    }
    await tabTo(page, '[aria-label="Manage workspace"]');
    await page.keyboard.press("Enter");
    const section = page.locator(".workspace-translation");
    const button = section.getByRole("button", { name: "Save translation settings" });
    await expect(button).toHaveAttribute("aria-disabled", "true");
    await tabTo(page, 'input[name="translation-mode"]:checked');
    // Native radio arrow keys select the next option and retain focus.
    await page.keyboard.press("ArrowUp");
    await page.keyboard.press("Tab");
    await expect(button).toBeFocused();
    const before = await button.boundingBox();
    await page.keyboard.press("Enter");
    await expect(section.getByRole("button", { name: "Saving…" })).toBeFocused();
    await page.keyboard.press("Enter");
    const saved = section.getByRole("button", { name: "Saved", exact: true });
    await expect(saved).toBeFocused();
    expect(state.writes).toEqual(["automatic"]);
    const after = await saved.boundingBox();
    expect(after?.width).toBe(before?.width);
    expect(after?.height).toBe(before?.height);
    await expect(saved.locator('[data-label="success"]')).toHaveCSS("opacity", "1");
    await page.screenshot({ path: `artifacts/ui-buttons/translation-${dark ? "dark" : "light"}-${viewport.width}.png`, fullPage: false });
    await expect(button).toBeFocused({ timeout: 4000 });
    await expect(button).toHaveAttribute("aria-disabled", "true");
    await page.reload();
    // Reopen settings after reload and read the persisted policy.
    if (viewport.width < 600) {
      await tabTo(page, '[aria-label="Open navigation"]'); await page.keyboard.press("Enter");
    }
    await tabTo(page, '[aria-label="Manage workspace"]'); await page.keyboard.press("Enter");
    await expect(section.locator('input[value="automatic"]')).toBeChecked();
    await expect(button).toHaveAttribute("aria-disabled", "true");
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await expect(page.locator("html")).toHaveAttribute("data-theme", dark ? "dark" : "light");
    await expect(page.locator("html")).toHaveAttribute("data-mantine-color-scheme", dark ? "dark" : "light");
  });
}

test("failed save stays actionable and can be retried with Space", async ({ page }) => {
  const state = await fixture(page);
  state.failNext();
  await page.goto("/");
  await tabTo(page, '[aria-label="Manage workspace"]'); await page.keyboard.press("Enter");
  await tabTo(page, 'input[name="translation-mode"]:checked');
  await page.keyboard.press("ArrowUp"); await page.keyboard.press("Tab"); await page.keyboard.press("Space");
  await expect(page.getByRole("alert")).toContainText("Could not update translation settings");
  const button = page.getByRole("button", { name: "Save translation settings" });
  await expect(button).toBeFocused();
  await expect(button).not.toHaveAttribute("aria-disabled", "true");
  await expect(page.getByRole("button", { name: "Saved", exact: true })).toHaveCount(0);
  state.succeedNext(); await page.keyboard.press("Space");
  await expect(page.getByRole("button", { name: "Saved", exact: true })).toBeFocused();
});

test("gallery keyboard controls, theme persistence, central tokens and reduced motion", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/ui-gallery.html");
  await page.keyboard.press("Tab");
  const toggle = page.getByRole("button", { name: "Dark theme" });
  await expect(toggle).toBeFocused(); await page.keyboard.press("Space");
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await tabTo(page, '.dakia-button[aria-label="Save translation settings"]');
  await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: "Saved", exact: true })).toBeFocused();
  await expect(page.locator('[data-label="success"]')).toHaveCSS("transition-duration", "0s");
  await expect(page.locator('[data-label="success"]')).toHaveCSS("transform", "none");
  // Tokens recolor every primary control together, without component overrides.
  await page.evaluate(() => document.documentElement.style.setProperty("--color-accent", "rgb(100, 60, 150)"));
  await expect(page.getByRole("button", { name: "Primary", exact: true })).toHaveCSS("background-color", "rgb(100, 60, 150)");
});

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
  await page.evaluate(() => document.documentElement.style.setProperty("--ui-button-primary-bg", "rgb(100, 60, 150)"));
  await expect(page.getByRole("button", { name: "Primary", exact: true })).toHaveCSS("background-color", "rgb(100, 60, 150)");
});

for (const dark of [false, true]) test(`shared button interaction states and contrast ${dark ? "dark" : "light"}`, async ({ page }, info) => {
  await page.addInitScript(dark => localStorage.setItem("dakia-theme", dark ? "dark" : "light"), dark);
  await page.goto("/ui-gallery.html");
  const primary = page.getByRole("button", { name: "Primary", exact: true });
  const secondary = page.getByRole("button", { name: "Secondary", exact: true });
  const tertiary = page.getByRole("button", { name: "Tertiary", exact: true });
  async function checkContrast(button: ReturnType<Page["getByRole"]>) {
    const result = await button.evaluate(el => {
      const parse = (s: string) => s.match(/[\d.]+/g)!.map(Number);
      const luminance = (rgb: number[]) => {
        const c = rgb.slice(0,3).map(v => { v /= 255; return v <= .04045 ? v / 12.92 : ((v + .055) / 1.055) ** 2.4; });
        return c[0] * .2126 + c[1] * .7152 + c[2] * .0722;
      };
      const ratio = (a: number[], b: number[]) => { const x=luminance(a), y=luminance(b); return (Math.max(x,y)+.05)/(Math.min(x,y)+.05); };
      let parent = el.parentElement;
      let surface = [255,255,255];
      while (parent) {
        const bg = parse(getComputedStyle(parent).backgroundColor);
        if (bg.length < 4 || bg[3] === 1) { surface = bg; break; }
        parent = parent.parentElement;
      }
      const s = getComputedStyle(el), bg=parse(s.backgroundColor);
      const background = bg.length < 4 || bg[3] === 1 ? bg : surface;
      return { text: ratio(parse(s.color), background), focus: ratio(parse(s.outlineColor), surface) };
    });
    expect(result.text).toBeGreaterThanOrEqual(4.5);
    return result;
  }
  for (const button of [primary, secondary, tertiary]) {
    await page.mouse.move(0,0); await page.keyboard.press("Tab"); await button.focus();
    await expect(button).toHaveCSS("outline-style", "solid");
    await expect(button).toHaveCSS("font-weight", "500");
    expect((await checkContrast(button)).focus).toBeGreaterThanOrEqual(3);
    const idle = await button.evaluate(el => getComputedStyle(el).backgroundColor);
    await button.hover(); await page.waitForTimeout(150);
    expect(await button.evaluate(el => getComputedStyle(el).backgroundColor)).not.toBe(idle);
    await checkContrast(button);
    await page.screenshot({ path: info.outputPath(`${await button.getAttribute("data-variant")}-hover.png`) });
    await page.mouse.down(); await page.waitForTimeout(150);
    await expect(button).toHaveCSS("transform", "none");
    await checkContrast(button);
    await page.screenshot({ path: info.outputPath(`${await button.getAttribute("data-variant")}-pressed.png`) });
    await page.mouse.up();
  }
  for (const variant of ["primary", "secondary", "tertiary"]) {
    await expect(page.getByRole("button", { name: `${variant} disabled`, exact: true })).toBeDisabled();
    await expect(page.getByRole("button", { name: `${variant} loading`, exact: true })).toHaveAttribute("aria-busy", "true");
    await expect(page.getByRole("button", { name: `${variant} loading`, exact: true })).toHaveCSS("opacity", "1");
    const icon = page.getByRole("button", { name: `${variant} icon action`, exact: true });
    await icon.focus(); await expect(icon).toHaveCSS("outline-style", "solid");
    await expect(page.getByRole("button", { name: `${variant} icon disabled`, exact: true })).toBeDisabled();
    await expect(page.getByRole("button", { name: `${variant} icon loading`, exact: true })).toHaveAttribute("aria-busy", "true");
  }
  await expect(page.getByRole("button", { name: "Delete", exact: true })).toHaveCSS("background-color", dark ? "rgb(255, 180, 171)" : "rgb(180, 35, 24)");
  const save = page.getByRole("button", { name: "Save changes", exact: true });
  await save.focus(); const before = await save.boundingBox();
  await page.keyboard.press("Enter");
  await expect(save).toHaveAttribute("aria-busy", "true");
  await expect(save).toBeFocused();
  const during = await save.boundingBox(); expect(during).toEqual(before);
  await page.screenshot({ path: info.outputPath("button-states.png") });
  await expect(save).not.toHaveAttribute("aria-busy", "true");
  await page.setViewportSize({ width: 360, height: 420 });
  await expect(primary).toHaveCSS("min-height", "44px");
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: info.outputPath("button-states-narrow.png"), fullPage: true });
});

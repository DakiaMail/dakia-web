import { test, expect, type Page } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { accounts, content, page as mail } from "./fixtures";

async function fixture(page: Page, theme = "light") {
  const searches: string[] = [], suggestions: string[] = [], writes: string[] = [];
  await page.addInitScript(theme => !localStorage.getItem("dakia-theme") && localStorage.setItem("dakia-theme", theme), theme);
  const rows = mail.rows.slice(0, 5).map(row => ({ ...row, unread: false }));
  await page.route("**/api/**", route => {
    const request = route.request(), url = new URL(request.url()), path = url.pathname;
    if (request.method() !== "GET") writes.push(path);
    const respond = (json: unknown) => route.fulfill({ json });
    if (path === "/api/session") return respond({ user_id: "search-toolbar-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/conversation")) return respond({ rows: [rows[0]], continuation: null, complete: true });
    if (path.includes("/messages/") && !path.endsWith("/messages")) return respond({ ...content, message: rows[0] });
    if (path === "/api/search" || path.endsWith("/messages")) {
      if (url.searchParams.get("suggest")) suggestions.push(url.searchParams.get("q") || "");
      else searches.push(url.searchParams.get("q") || "");
      return respond({ ...mail, rows, continuation: null, pending: false });
    }
    return respond([]);
  });
  await page.goto("/");
  await expect(page.locator(".message-open").first()).toBeVisible();
  return { searches, suggestions, writes };
}

for (const theme of ["light", "dark"]) for (const width of [1480, 900, 390, 320]) {
  test(`toolbar keyboard and filters ${theme} ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: width === 900 ? 600 : 980 });
    const requests = await fixture(page, theme);
    const input = page.getByRole("combobox", { name: "Search all mail" });
    await expect(input).toHaveCount(1);
    await expect(page.locator(".sidebar [role=search]")).toHaveCount(0);
    await page.locator(".mailbox").focus(); await page.keyboard.press("/");
    await expect(input).toBeFocused();
    const before = requests.searches.length;
    await input.fill("launch");
    await expect(page.getByRole("option", { name: /A few thoughts on the launch/ })).toBeVisible();
    expect(requests.suggestions).toContain("launch");
    expect(requests.searches.slice(before).filter(query => query === "launch")).toEqual([]);
    await input.press("ArrowDown");
    await expect(input).toBeFocused();
    await expect(input).toHaveAttribute("aria-activedescendant", /.+/);
    await input.press("Escape");
    await expect(page.getByRole("listbox", { name: "Search suggestions" })).toHaveCount(0);
    await expect(input).toHaveValue("launch"); await expect(input).toBeFocused();
    await input.press("Enter");
    await expect.poll(() => requests.searches.includes("launch")).toBe(true);
    await expect(page.getByRole("button", { name: "Back to inbox", exact: true })).toBeVisible();
    await expect(input).toBeFocused();
    await page.getByRole("button", { name: "Search filters", exact: true }).focus(); await page.keyboard.press("Enter");
    const dialog = page.getByRole("dialog", { name: "Search filters", exact: true });
    await expect(dialog).toHaveCSS("opacity", "1");
    await expect(dialog.getByRole("textbox", { name: "Find a filter" })).toBeFocused();
    await page.screenshot({ animations: "disabled", path: `artifacts/search-toolbar/filters-${theme}-${width}.png` });
    await page.keyboard.press("Escape"); await expect(dialog).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Search filters", exact: true })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(dialog).toHaveCSS("opacity", "1");
    await expect(dialog.getByRole("textbox", { name: "Find a filter" })).toBeFocused();
    await dialog.getByRole("textbox", { name: "Find a filter" }).fill("status");
    await page.keyboard.press("Tab"); await page.keyboard.press("Enter");
    await dialog.getByRole("combobox", { name: "Status" }).focus();
    await dialog.getByRole("combobox", { name: "Status" }).selectOption("unread");
    await expect(dialog.getByRole("combobox", { name: "Status" })).toBeFocused();
    await page.keyboard.press("Tab"); await expect(dialog.getByRole("button", { name: "Apply filter" })).toBeFocused(); await page.keyboard.press("Enter");
    await expect(dialog).toHaveCount(0);
    await expect.poll(() => requests.searches.some(query => query === "launch is:unread")).toBe(true);
    await expect(input).toHaveValue("launch");
    await expect(page.getByRole("button", { name: "Remove Unread filter" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Search filters", exact: true })).toBeFocused();
    await page.screenshot({ animations: "disabled", path: `artifacts/search-toolbar/results-${theme}-${width}.png` });
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.getByRole("button", { name: "Remove Unread filter" }).focus(); await page.keyboard.press("Enter");
    await expect(page.getByRole("button", { name: "Remove Unread filter" })).toHaveCount(0);
    await expect(input).toHaveValue("launch");
    await page.getByRole("combobox", { name: "Account filter" }).selectOption("work");
    await expect(input).toHaveValue("launch");
    await expect(page.locator('nav[aria-label="Accounts"] button.selected')).toHaveText(/Work/);
    await page.getByRole("button", { name: "Back to inbox", exact: true }).focus(); await page.keyboard.press("Enter");
    await expect(input).toHaveValue("");
    await input.fill("launch");
    await expect(page.getByRole("option", { name: /A few thoughts on the launch/ })).toBeVisible();
    await page.screenshot({ animations: "disabled", path: `artifacts/search-toolbar/autocomplete-${theme}-${width}.png` });
    expect(requests.writes).toEqual([]);
  });
}

test.beforeEach(() => mkdirSync("artifacts/search-toolbar", { recursive: true }));

test("native keys, explicit message choice, and theme persistence", async ({ page }) => {
  const requests = await fixture(page);
  const input = page.getByRole("combobox", { name: "Search all mail" });
  await input.fill("launch"); await expect(page.getByRole("option", { name: /A few thoughts on the launch/ })).toBeVisible();
  const untouched = await input.evaluate(element => {
    for (const modifiers of [{ metaKey: true }, { ctrlKey: true }, { altKey: true }, { isComposing: true }]) {
      for (const key of ["Enter", "Escape", "ArrowDown"]) {
        if (!element.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...modifiers }))) return false;
      }
    }
    return true;
  });
  expect(untouched).toBe(true);
  await input.press("ArrowUp"); await input.press("Enter");
  await expect(page.locator(".conversation-message")).toBeVisible();
  expect(requests.writes).toEqual([]);
  await page.getByRole("button", { name: "Dark theme", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.reload(); await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
});

for (const theme of ["light", "dark"]) test(`filter edits and short viewport ${theme}`, async ({ page }) => {
  await page.setViewportSize({ width: 720, height: 400 });
  const requests = await fixture(page, theme);
  await page.getByRole("button", { name: "Search filters", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Search filters", exact: true });
  await expect(dialog.getByRole("textbox", { name: "Find a filter" })).toBeFocused();
  const bounds = (await dialog.boundingBox())!;
  expect(bounds.y).toBeGreaterThanOrEqual(0); expect(bounds.y + bounds.height).toBeLessThanOrEqual(400);
  await dialog.getByRole("textbox", { name: "From", exact: true }).fill("sender@example.org");
  await dialog.getByRole("button", { name: "Apply filter" }).click();
  await expect.poll(() => requests.searches.includes("from:sender@example.org")).toBe(true);
  const edit = page.getByRole("button", { name: "Edit From sender@example.org filter" });
  await edit.focus(); await page.keyboard.press("Enter");
  await expect(dialog.getByRole("textbox", { name: "From", exact: true })).toBeFocused();
  await expect(dialog.getByRole("textbox", { name: "From", exact: true })).toHaveValue("sender@example.org");
  await page.keyboard.press("Escape"); await expect(edit).toBeFocused();
  await page.getByRole("combobox", { name: "Search all mail" }).fill("launch");
  await page.getByRole("button", { name: "Clear search text" }).click();
  await expect(edit).toBeVisible();
  await page.getByRole("button", { name: "Clear filters", exact: true }).click();
  await expect(edit).toHaveCount(0);
  const search = page.getByRole("combobox", { name: "Search all mail" });
  await search.fill('subject:"First phrase"'); await search.press("Enter");
  await search.fill('subject:"Second phrase"'); await search.press("Enter");
  await expect.poll(() => requests.searches.includes('subject:"Second phrase"')).toBe(true);
  await expect(page.getByRole("button", { name: "Edit Subject Second phrase filter" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Edit Subject First phrase filter" })).toHaveCount(0);
  expect(requests.writes).toEqual([]);
});

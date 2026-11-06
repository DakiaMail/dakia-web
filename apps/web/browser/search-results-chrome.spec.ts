import { test, expect } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { accounts, page as mail } from "./fixtures";

for (const width of [320, 390, 1480]) test(`search results chrome ${width}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 980 });
  let failed = false;
  let suggestions = 0;
  const searches: string[] = [];
  const external: string[] = [], writes: string[] = [];
  const rows = mail.rows.slice(0, 8).map((row, i) => ({ ...row, unread: false, subject: `Project planning ${i + 1}`, category: "people" }));
  await page.route("**/*", route => {
    const url = new URL(route.request().url()), path = url.pathname;
    if (url.origin !== (process.env.DAKIA_FIXTURE_ORIGIN || "http://127.0.0.1:4173")) { external.push(url.origin); return route.abort(); }
    if (!path.startsWith("/api/")) return route.continue();
    if (route.request().method() !== "GET") writes.push(path);
    const respond = (json: unknown) => route.fulfill({ json });
    if (path === "/api/session") return respond({ user_id: "search-chrome-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path === "/api/search" && url.searchParams.get("q")?.startsWith("planning")) {
      expect(url.searchParams.get("category") || "").toBe("");
      if (url.searchParams.has("suggest")) suggestions++;
      else searches.push(url.searchParams.get("q")!);
      return respond({ ...mail, rows, index_page: true, cache_preview: true, continuation: null, pending: !failed, counts_exact: false,
        coverage: [{ account_id: accounts[0].id, folder_ids: [], state: failed ? "error" : "partial", error: failed ? "provider_unavailable" : null }] });
    }
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows });
    return respond([]);
  });
  await page.goto("/");
  if (width <= 600) await page.getByRole("button", { name: "Open navigation" }).click();
  const input = page.getByRole("textbox", { name: "Search all mail" });
  await input.fill("planning");
  await expect(page.locator(".search-suggestions").getByText("Project planning 1", { exact: true })).toBeVisible();
  expect(suggestions).toBeGreaterThan(0);
  // A debounced search must not move a still-focused mobile drawer input
  // behind the scrim or lose desktop typing focus.
  await expect.poll(() => searches.includes("planning")).toBe(true);
  await expect(input).toBeVisible();
  await expect(input).toBeFocused();
  await input.press("Enter");
  await expect(page.locator(".message-open").first()).toContainText("Project planning");
  await expect(page.getByRole("combobox", { name: "Search category" })).toHaveCount(0);
  await expect(page.locator(".search-filters,.inbox-sync-status")).toHaveCount(0);
  await expect(page.locator(".search-suggestions")).toHaveCount(0);
  await expect(input).toBeVisible();
  await expect(input).toHaveValue("planning");
  await expect(page.getByRole("textbox", { name: "Search all mail" })).toHaveCount(1);
  if (width <= 600) {
    await expect(page.locator(".app")).not.toHaveClass(/nav-open/);
    await expect.poll(async () => {
      const bounds = (await page.locator(".sidebar").boundingBox())!;
      return bounds.x + bounds.width;
    }).toBeLessThanOrEqual(0);
  }
  mkdirSync("artifacts/search-results", { recursive: true });
  await page.screenshot({ path: `artifacts/search-results/search-${width}.png` });
  // Edit directly in results, retaining typeahead and account scope. Long
  // queries scroll inside the input rather than widening the whole page.
  const longQuery = "planning a long project query with Unicode café and 東京 ".repeat(3).trim();
  await input.fill(longQuery);
  await expect(page.locator(".search-suggestions").getByText("Project planning 1", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Search", exact: true }).click();
  await expect.poll(() => searches.includes(longQuery)).toBe(true);
  await expect(input).toHaveValue(longQuery);
  await expect(input).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.screenshot({ path: `artifacts/search-results/search-long-${width}.png` });
  await page.goBack();
  await expect(page.locator(".results-search")).toHaveCount(0);
  await page.goForward();
  await expect(page.locator(".results-search input")).toHaveValue(longQuery);
  await expect(page.locator(".results-search input")).toBeVisible();
  await page.getByRole("combobox", { name: "Account filter" }).selectOption(accounts[0].id);
  const scopedRequest = page.waitForRequest(request => {
    const url = new URL(request.url());
    return url.pathname === "/api/search" && url.searchParams.get("q") === "planning scoped" && url.searchParams.get("accounts") === accounts[0].id;
  });
  await input.fill("planning scoped"); await input.press("Enter");
  await scopedRequest;
  await expect(page.getByRole("combobox", { name: "Account filter" })).toHaveValue(accounts[0].id);
  await input.fill("planning after Enter");
  await expect(page.locator(".search-suggestions").getByText("Project planning 1", { exact: true })).toBeVisible();
  await input.press("Enter");
  // Real provider errors remain actionable, unlike routine partial coverage.
  failed = true;
  await input.fill("planning again"); await input.press("Enter");
  await expect(page.getByRole("alert")).toContainText("Couldn’t finish checking the provider");
  await expect(page.locator(".message-open").first()).toBeVisible();
  expect(external).toEqual([]); expect(writes).toEqual([]);
});

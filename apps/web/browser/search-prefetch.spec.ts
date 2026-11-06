import { test, expect } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";

test("indexed search prefetches one page without appending or refetching it", async ({ page }) => {
  const calls: string[] = [];
  const row = (id: string) => ({ ...mail.rows[0], id, subject: `Indexed ${id}`, unread: false });
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url());
    if (url.pathname === "/api/session") return route.fulfill({ json: { user_id: "prefetch-fixture", csrf_token: "synthetic" } });
    if (url.pathname === "/api/accounts") return route.fulfill({ json: accounts });
    if (url.pathname.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (url.pathname === "/api/search" && url.searchParams.get("q") === "needle" && !url.searchParams.has("suggest")) {
      const cursor = url.searchParams.get("continuation") || "initial"; calls.push(cursor);
      const rows = cursor === "initial" ? [row(cursor), ...Array.from({ length: 49 }, (_, i) => row(`initial-${i}`))] : [row(cursor)];
      return route.fulfill({ json: { ...mail, rows, index_page: cursor === "initial", cache_preview: true, continuation: cursor === "initial" ? "second" : null, pending: true, counts_exact: false, coverage: [{ account_id: accounts[0].id, folder_ids: [], state: "partial", error: null }] } });
    }
    if (url.pathname === "/api/search" || url.pathname.endsWith("/messages")) return route.fulfill({ json: { ...mail, rows: [], continuation: null, pending: false } });
    return route.fulfill({ json: [] });
  });
  await page.goto("/");
  const input = page.getByRole("textbox", { name: "Search all mail" });
  await input.fill("needle"); await input.press("Enter");
  await expect(page.getByText("Indexed initial", { exact: true })).toBeVisible();
  await expect.poll(() => calls).toEqual(["initial", "second"]);
  await expect(page.getByText("Indexed second", { exact: true })).toHaveCount(0);
  const more = page.getByRole("button", { name: /load more|check for more/i }).first();
  if (await more.count()) await more.click();
  else await page.locator(".message-list").evaluate(element => element.scrollTop = element.scrollHeight);
  await expect(page.getByText("Indexed second", { exact: true })).toBeVisible();
  expect(calls).toEqual(["initial", "second"]);
});

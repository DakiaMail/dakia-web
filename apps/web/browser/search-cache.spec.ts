import { test, expect } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";

for (const offline of [false, true]) {
  test(`saved search matches render before provider settles, offline=${offline}`, async ({ page }) => {
    const calls: string[] = [], writes: string[] = [];
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const saved = { ...mail.rows[0], id: "saved", subject: "Saved match", unread: false };
    const fresh = { ...saved, subject: "Fresh provider match", revision: saved.revision + 1 };
    await page.route("**/api/**", async route => {
      const url = new URL(route.request().url());
      if (route.request().method() !== "GET") writes.push(url.pathname);
      if (url.pathname === "/api/session") return route.fulfill({ json: { user_id: "cache-fixture", csrf_token: "synthetic" } });
      if (url.pathname === "/api/accounts") return route.fulfill({ json: accounts });
      if (url.pathname.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
      if (url.pathname === "/api/search" && url.searchParams.get("q") === "subject:match") {
        expect(url.searchParams.get("cache_first")).toBe("true");
        const cursor = url.searchParams.get("continuation") || "initial";
        calls.push(cursor);
        if (cursor === "initial") return route.fulfill({ json: { ...mail, rows: [saved], cache_preview: true, continuation: "provider", pending: true, counts_exact: false, coverage: [{ account_id: saved.account_id, folder_ids: [], state: "pending", error: null }] } });
        await gate;
        if (offline) return route.fulfill({ status: 503, json: { error: "provider_unavailable" } });
        return route.fulfill({ json: { ...mail, rows: [fresh], continuation: null, pending: false, coverage: [{ account_id: saved.account_id, folder_ids: ["INBOX"], state: "complete", error: null }] } });
      }
      if (url.pathname === "/api/search" || url.pathname.endsWith("/messages")) return route.fulfill({ json: { ...mail, rows: [], continuation: null, pending: false } });
      return route.fulfill({ json: [] });
    });
    await page.goto("/");
    const input = page.getByRole("textbox", { name: "Search all mail" });
    await input.fill("subject:match");
    await input.press("Enter");
    await expect(page.getByText("Saved match", { exact: true })).toBeVisible();
    await expect.poll(() => calls).toEqual(["initial", "provider"]);
    await expect(page.locator(".search-filters")).toHaveCount(0);
    await expect(page.getByText("Showing saved matches", { exact: false })).toHaveCount(0);
    release();
    if (offline) {
      await expect(page.getByRole("button", { name: "Retry loading mail" })).toBeVisible();
      await expect(page.getByText("Saved match", { exact: true })).toBeVisible();

      await page.waitForTimeout(3300);
      expect(calls).toEqual(["initial", "provider"]); // no automatic failed-search polling
    } else {
      await expect(page.getByText("Fresh provider match", { exact: true })).toBeVisible();
      await expect(page.getByText("Saved match", { exact: true })).toHaveCount(0);
      await expect(page.getByRole("status").filter({ hasText: "Showing saved matches" })).toHaveCount(0);
    }
    expect(writes).toEqual([]);
  });
}

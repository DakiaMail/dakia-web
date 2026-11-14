import { test, expect } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";

test("single-account folders show cached mail before reconciliation and reuse visited filters", async ({ page }) => {
  const calls: string[] = [];
  let direct = 0;
  const reconcile: (() => void)[] = [];
  await page.route("**/api/**", async route => {
    const url = new URL(route.request().url());
    if (url.pathname === "/api/session") return route.fulfill({ json: { user_id: "fast-fixture", csrf_token: "fixture" } });
    if (url.pathname === "/api/accounts") return route.fulfill({ json: [accounts[0]] });
    if (url.pathname === "/api/search") {
      if (url.searchParams.get("attention")) return route.fulfill({ json: { ...mail, rows: [] } });
      const scope = [url.searchParams.get("q"), url.searchParams.get("inbox_kind"), url.searchParams.get("email_type"),url.searchParams.get("transactions")].join("/");
      calls.push(scope);
      const kind = url.searchParams.get("inbox_kind") || "automated";
      const type = url.searchParams.get("email_type");
      const rows = [{ ...mail.rows[0], account_id: accounts[0].id, subject: `Cached ${scope}`, classification: { kind, status: "ready", types: type ? [type] : url.searchParams.get("transactions") ? ["receipt","invoice"] : [], details: [], attention: [], attention_override: null, source: "automatic", policy_revision: "personal-first-1" } }];
      if (url.searchParams.get("q") !== "folder:INBOX") {
        if (url.searchParams.get("continuation")) {
          await new Promise<void>(resolve => reconcile.push(resolve));
        } else return route.fulfill({ json: { ...mail, rows, coverage: mail.coverage.map(c => ({ ...c, state: "pending" })), cache_preview: true, continuation: "provider-reconcile", pending: true } });
      }
      return route.fulfill({ json: { ...mail, rows } });
    }
    if (url.pathname.endsWith("/messages")) { direct++; return route.fulfill({ json: { ...mail, rows: [] } }); }
    return route.fulfill({ json: [] });
  });
  await page.goto("/");
  await expect(page.locator(".message-row")).toHaveCount(1);
  await page.getByRole("tab", { name: "Automated", exact: true }).click();
  await expect(page.getByText("Cached folder:INBOX/automated//", { exact: true })).toBeVisible();
  for (const [label, type] of [["Transactions", "transactions"], ["Orders", "order"]]) {
    await page.getByRole("button", { name: label, exact: true }).click();
    await expect(page.getByText(`Cached folder:INBOX/automated/${type === "transactions" ? "/true" : type+"/"}`, { exact: true })).toBeVisible();
  }
  const before = calls.length;
  await page.getByRole("button", { name: "Transactions", exact: true }).click();
  await expect(page.getByText("Cached folder:INBOX/automated//true", { exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "Personal", exact: true }).click();
  await expect(page.getByText("Cached folder:INBOX/personal//", { exact: true })).toBeVisible();
  expect(calls.length).toBe(before);
  for (const nav of ["Sent", "Trash"]) {
    await page.getByRole("button", { name: nav, exact: true }).click();
    await expect(page.getByText(`Cached folder:${nav}///`, { exact: true })).toBeVisible();
    await expect.poll(() => reconcile.length).toBe(1);
    reconcile.shift()!();
  }
  expect(direct).toBe(0);
});

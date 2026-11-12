// Actual browser -> Rust HTTP/CSRF -> SQLite outbox -> controlled provider.
// No API interception and no network-accessible mail or unsubscribe provider.
import { test, expect } from "@playwright/test";
import type { CleanupJob } from "../src/cleanupJobs";

test("48-message cleanup is optimistic, silent across reload and eventually applied by the real worker", async ({ page, context }) => {
  test.setTimeout(60000);
  await context.addCookies([{ name: "__Host-dakia", value: "release-browser-fixture", url: "https://127.0.0.1:4327/", secure: true, sameSite: "Lax" }]);
  await page.setViewportSize({ width: 390, height: 900 });
  await page.addInitScript(() => localStorage.setItem("dakia-theme", "dark"));
  const writes: string[] = [];
  page.on("request", r => { if (r.method() === "POST") writes.push(new URL(r.url()).pathname); });
  await page.goto("/");
  await page.getByRole("tab", { name: "All", exact: true }).click();

  // Virtualized rows can be below the initial viewport. The exact source is
  // discovered using the real read-only local catalogue, not guessed IDs.
  const accounts = await (await page.request.get("/api/accounts")).json();
  const account = accounts[0].id;
  const catalogue = await (await page.request.get(`/api/accounts/${account}/messages?folder=INBOX`)).json();
  const source = catalogue.rows.find((m: { subject: string }) => m.subject === "Synthetic 7");
  expect(source).toBeTruthy();
  await page.goto(`/?account=${account}&message=${source.id}`);
  await page.getByRole("button", { name: "Unsubscribe", exact: true }).click();
  const dialog = page.getByRole("dialog");
  const move = dialog.getByRole("button", { name: /^Move 48 (found )?emails to Trash$/ });
  await expect(move).toBeEnabled();
  const accepted = page.waitForResponse(r => r.url().endsWith("/sender-cleanups") && r.request().method() === "POST");
  await move.click();
  await expect(dialog).toHaveCount(0);
  const job: CleanupJob = await (await accepted).json();
  expect(job.pending).toBe(48); expect(job.applied).toBe(0);
  await expect(page.locator(".message-open")).toHaveCount(0);
  await expect(page.getByRole("complementary", { name: "Background sender cleanup" })).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await page.screenshot({ path: "artifacts/silent-cleanup/real-backend-inbox-390.png" });
  await page.reload();
  await expect(page.locator(".message-open")).toHaveCount(0);
  await page.getByRole("button", { name: "Open navigation" }).click();
  await page.getByRole("navigation", { name: "Mailboxes" }).getByRole("button", { name: "Trash", exact: true }).click();
  await expect(page.locator(".message-open").first()).toBeVisible();
  await expect.poll(() => page.locator(".sidebar").evaluate(node => node.getBoundingClientRect().right)).toBeLessThanOrEqual(0);
  await expect(page.getByText("invalid provider metadata", { exact: true })).toHaveCount(0);
  await page.screenshot({ path: "artifacts/silent-cleanup/real-backend-trash-390.png" });
  await page.setViewportSize({ width: 1480, height: 980 });
  await expect(page.locator(".message-open").first()).toBeVisible();
  await page.screenshot({ path: "artifacts/silent-cleanup/real-backend-trash-1480.png" });
  // Test-side readback only; application itself has no cleanup status timer.
  await expect.poll(async () => {
    const jobs: CleanupJob[] = await (await page.request.get(`/api/accounts/${account}/sender-cleanups`)).json();
    return jobs.find(row => row.request.id === job.request.id)?.applied;
  }, { timeout: 30000 }).toBe(48);
  await page.reload();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByText(/pending in background|not verified|moved to Trash\./)).toHaveCount(0);
  expect(writes.filter(path => path.endsWith("/sender-cleanups"))).toHaveLength(1);
  expect(writes.filter(path => path.endsWith("/unsubscribe"))).toHaveLength(0);
});

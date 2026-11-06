// This suite uses actual Rust HTTP handlers, SQLite, MIME parsing and workers.
// Only the mail-provider boundary is synthetic. No page.route API mocks.
import { test, expect } from "@playwright/test";
import { readFile } from "node:fs/promises";

test.beforeEach(async ({ context }) => {
  // A real browser cookie also authenticates the download subsystem. Injecting
  // a Cookie request header alone does not reproduce attachment navigation.
  await context.addCookies([{ name: "__Host-dakia", value: "release-browser-fixture", url: "https://127.0.0.1:4327/", secure: true, sameSite: "Lax" }]);
});

test("received attachment downloads exact bytes with its original filename", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("tab", { name: "List", exact: true }).click();
  await page.locator(".message-open").filter({ hasText: "Synthetic 0" }).click();
  const downloadPromise = page.waitForEvent("download");
  await page.getByRole("link", { name: /example.txt/ }).click();
  const download = await downloadPromise;
  expect(await download.failure()).toBeNull();
  expect(await readFile((await download.path())!)).toEqual(Buffer.from("attachment bytes"));
  expect(download.suggestedFilename()).toBe("example.txt");
});

test("general search uses saved matches through real Rust HTTP and SQLite", async ({ page }) => {
  const writes: string[] = [];
  page.on("request", r => { if (r.method() !== "GET") writes.push(r.url()); });
  await page.goto("/");
  const previewResponse = page.waitForResponse(r => {
    const u = new URL(r.url());
    return u.pathname === "/api/search" && u.searchParams.get("q") === "subject:Synthetic" && !u.searchParams.has("continuation");
  });
  const providerResponse = page.waitForResponse(r => {
    const u = new URL(r.url());
    return u.pathname === "/api/search" && u.searchParams.get("q") === "subject:Synthetic" && u.searchParams.has("continuation");
  });
  const input = page.getByRole("textbox", { name: "Search all mail" });
  await input.fill("subject:Synthetic"); await input.press("Enter");
  const preview = await (await previewResponse).json();
  expect(preview.cache_preview).toBe(true);
  expect(preview.rows.length).toBeGreaterThan(0);
  expect(preview.pending).toBe(true);
  expect(preview.counts_exact).toBe(false);
  expect(preview.continuation).toBeTruthy();
  const provider = await (await providerResponse).json();
  expect(provider.cache_preview).toBeUndefined();
  expect(provider.rows.length).toBeGreaterThan(0);
  await expect(page.locator(".message-open").filter({ hasText: "Synthetic 0" })).toBeVisible();
  expect(writes).toEqual([]);
});

for (const [index, label, destination] of [[1, "Archive", "Archive"], [2, "Move to trash", "Trash"], [3, "Mark as spam", "Junk"]] as const) {
  test(`${label} completes through the real queue and survives reload`, async ({ page }) => {
    await page.goto("/");
    await page.getByRole("tab", { name: "List", exact: true }).click();
    const row = page.locator(".message-open").filter({ hasText: `Synthetic ${index}` });
    await row.click();
    const accepted = page.waitForResponse(r => r.url().endsWith("/operations") && r.request().method() === "POST");
    await page.getByRole("region", { name: "Reading pane" }).getByRole("button", { name: label, exact: true }).click();
    const op = await (await accepted).json();
    expect(op.status).toBe("pending");
    await expect.poll(async () => {
      const result = await page.request.get(`/api/accounts/${op.account_id}/operations/${op.id}`);
      return (await result.json()).status;
    }).toBe("applied");
    const result = await page.request.get(`/api/accounts/${op.account_id}/messages/${op.target_id}`);
    expect((await result.json()).message.folder_ids).toContain(destination);
    await page.reload();
    await expect(row).toHaveCount(0);
    await expect(page.getByRole("alert")).toHaveCount(0);
  });
}

test("mark unread persists through the real provider boundary", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("tab", { name: "List", exact: true }).click();
  await page.locator(".message-open").filter({ hasText: "Synthetic 4" }).click();
  const accepted = page.waitForResponse(r => r.url().endsWith("/operations") && r.request().method() === "POST");
  await page.getByRole("region", { name: "Reading pane" }).getByRole("button", { name: "Mark unread", exact: true }).click();
  const op = await (await accepted).json();
  await expect.poll(async () => (await (await page.request.get(`/api/accounts/${op.account_id}/operations/${op.id}`)).json()).status).toBe("applied");
  const result = await page.request.get(`/api/accounts/${op.account_id}/messages/${op.target_id}`);
  expect((await result.json()).message.unread).toBe(true);
});

test("sender count, read-only review and explicit Trash use real backend and durable queue", async ({ page }) => {
  const writes: { path: string; targets?: { id: string }[] }[] = [];
  page.on("request", r => { if (r.method() === "POST") writes.push({ path: new URL(r.url()).pathname, ...r.postDataJSON() }); });
  await page.goto("/");
  await page.getByRole("tab", { name: "List", exact: true }).click();
  await page.locator(".message-open").filter({ hasText: "Synthetic 7" }).click();
  await page.getByRole("button", { name: "Unsubscribe", exact: true }).click();
  const dialog = page.getByRole("dialog");
  await expect(dialog.getByRole("heading", { name: "Unsubscribe request sent" })).toBeVisible();
  const move = dialog.getByRole("button", { name: /^Move \d+ (found )?emails? to Trash$/ });
  await expect(move).toBeEnabled();
  await expect(dialog.getByText("Checking for more…", { exact: true })).toHaveCount(0);
  const count = Number((await move.innerText()).match(/\d+/)![0]);
  expect(count).toBeGreaterThan(0);
  const before = writes.length;
  await dialog.getByRole("button", { name: "Review emails", exact: true }).click();
  await expect(dialog.getByRole("listitem")).toHaveCount(count);
  await dialog.getByRole("listitem").filter({ hasText: "Synthetic 7" }).getByRole("button").click();
  await expect(dialog.locator("pre")).toContainText("Read-only sender cleanup preview.");
  await dialog.getByRole("button", { name: "Back to emails" }).click();
  expect(writes.length).toBe(before);
  await move.click();
  await expect(dialog).toHaveCount(0);
  await page.reload();
  const progress = page.getByRole("complementary", { name: "Background sender cleanup" });
  await expect(progress).toHaveCount(0);
  const batches = writes.filter(r => r.path.endsWith("/sender-cleanups"));
  expect(batches).toHaveLength(1);
  expect(batches[0].targets).toHaveLength(count);
  expect(new Set(batches[0].targets!.map(r => r.id)).size).toBe(count);
  expect(writes.some(r => r.path.endsWith("/unsubscribe"))).toBe(false);
  await expect.poll(async () => {
    const response = await page.request.get(batches[0].path);
    const jobs = await response.json();
    return jobs.find((job: { request: { id: string } }) => job.request.id === (batches[0] as { id?: string }).id)?.applied;
  }).toBe(count);
  await page.screenshot({ path: "artifacts/sender-real-backend-finished.png" });
});

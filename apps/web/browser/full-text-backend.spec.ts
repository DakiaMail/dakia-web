import { test, expect } from "@playwright/test";

test.beforeEach(async ({ context }) => {
  await context.addCookies([{ name: "__Host-dakia", value: "release-browser-fixture", url: "https://127.0.0.1:4327/", secure: true, sameSite: "Lax" }]);
});
for (const width of [390, 1480]) {
  test(`real backend finds historical body-only Unicode and suggestions without mail writes at ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    const writes: string[] = [];
    page.on("request", r => { if (r.method() !== "GET") writes.push(new URL(r.url()).pathname); });
    await page.goto("/");
    if (width < 800) await page.getByRole("button", { name: "Open navigation" }).click();
    const input = page.getByRole("textbox", { name: "Search all mail" });
    const suggested = page.waitForResponse(r => new URL(r.url()).searchParams.get("suggest") === "true");
    const start = performance.now();
    await input.fill("cafe\u0301");
    const result = await (await suggested).json();
    expect(result.rows).toHaveLength(5);
    expect(result.continuation).toBeNull();
    expect(result.pending).toBe(false);
    await expect(page.getByLabel("Indexed search suggestions")).toBeVisible();
    const suggestionsMs = performance.now() - start;
    const initial = page.waitForResponse(r => { const u = new URL(r.url()); return u.pathname === "/api/search" && u.searchParams.get("q") === "body-only-historical" && !u.searchParams.has("continuation") && !u.searchParams.has("suggest"); });
    const searchStart = performance.now();
    await input.fill("body-only-historical"); await input.press("Enter");
    const search = await (await initial).json();
    expect(search.rows).toHaveLength(8);
    expect(search.cache_preview).toBeUndefined();
    expect(search.coverage.every((c: { state: string }) => c.state === "complete")).toBe(true);
    await expect(page.locator(".message-open").filter({ hasText: "Synthetic 0" })).toBeVisible();
    const renderMs = performance.now() - searchStart;
    expect(writes).toEqual([]);
    console.log(JSON.stringify({ width, suggestions_submit_to_visible_ms: suggestionsMs, body_submit_to_render_ms: renderMs, initial_matches: search.rows.length }));
    await page.waitForTimeout(350); // Allow the mobile navigation close transition to finish before capture.
    await page.screenshot({ path: `../../verification/full-text-${width}.png`, fullPage: true });
  });
}

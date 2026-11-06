import { test, expect, type Page } from "@playwright/test";
import { createHash } from "node:crypto";
import { accounts, content, page as mail } from "./fixtures";
import type { UnsubscribeAction } from "../src/Unsubscribe";
async function fixture(page: Page, action: UnsubscribeAction | null, fail = false) {
  const mutations: { path: string; body: unknown }[] = [];
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    const method = route.request().method();
    const respond = (value: unknown, status = 200) => route.fulfill({ status, contentType: "application/json", body: JSON.stringify(value) });
    if (method !== "GET") mutations.push({ path, body: route.request().postDataJSON() });
    if (path === "/api/session") return respond({ user_id: "fixture", csrf_token: "fixture-csrf" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ":connected\n\n" });
    if (path.endsWith("/sender-mail")) return respond({ rows: [], count: 0, cutoff: new Date().toISOString(), after: null });
    if (path === "/api/search") return respond(mail);
    if (path.endsWith("/unsubscribe")) {
      if (method === "GET") return respond(action);
      expect(route.request().headers()["x-csrf-token"]).toBe("fixture-csrf");
      if (fail) return respond({ code: "unsubscribe_result_unknown", message: "not confirmed" }, 502);
      return route.fulfill({ status: 204 });
    }
    if (path.includes("/messages/")) return respond({ ...content, message: mail.rows.find(m => path.endsWith(m.id)) || content.message });
    if (path.endsWith("/operations")) return respond({ ...route.request().postDataJSON(), status: "applied", revision: 2 });
    if (path.includes("/drafts/") && method === "PUT") return respond({ ...route.request().postDataJSON(), revision: 1 });
    if (path.endsWith("/recipients")) return respond([]);
    return respond([]);
  });
  await page.goto("/");
  return mutations;
}
async function open(page: Page) { await page.getByRole("button", { name: /Maya Chen/ }).click(); }
for (const mobile of [false, true]) for (const dark of [false, true]) {
  const label = `${mobile ? "mobile" : "desktop"}-${dark ? "dark" : "light"}`;
  test(`one-click unsubscribe and canonical branding ${label}`, async ({ page }) => {
    await page.setViewportSize(mobile ? { width: 390, height: 844 } : { width: 1480, height: 980 });
    await page.addInitScript(dark => localStorage.setItem("dakia-theme", dark ? "dark" : "light"), dark);
    const action = { kind: "one_click" as const, url: "https://newsletter.example.org/unsubscribe?token=synthetic" };
    const mutations = await fixture(page, action);
    if (mobile) await page.getByRole("button", { name: "Open navigation" }).click();
    const image = page.locator(".brand img");
    await expect(image).toBeVisible();
    expect(await image.evaluate((i: HTMLImageElement) => i.complete && i.naturalWidth > 0)).toBe(true);
    const logo = await page.request.get("/dakia-icon-512.png");
    // Exact original PNG used by apps/site/src/site.tsx, not the approximate SVG.
    expect(createHash("sha256").update(await logo.body()).digest("hex")).toBe("1b003fd4d98cffeb70f3b45606e974227a8028c29f8398a9136c6da083c5c8c2");
    await page.screenshot({ path: `artifacts/branding-${label}.png`, animations: "disabled" });
    if (mobile) await page.getByRole("complementary").getByRole("button", { name: "Close navigation" }).click();
    await open(page);
    const trigger = page.getByRole("button", { name: "Unsubscribe", exact: true });
    await trigger.click();
    await expect(page.getByRole("dialog").getByRole("heading", { name: "Unsubscribe request sent" })).toBeVisible();
    expect(mutations.filter(m => m.path.endsWith("/unsubscribe"))).toEqual([{ path: "/api/accounts/work/messages/m0/unsubscribe", body: { expected_action: action } }]);
    await page.screenshot({ path: `artifacts/unsubscribe-${label}-sent.png`, animations: "disabled" });
    await page.getByRole("button", { name: "Done", exact: true }).click();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  });
}
test("unsupported webpages never navigate; mailto sends directly without a composer", async ({ page }) => {
  const mutations = await fixture(page, { kind: "web", url: "https://newsletter.example.org/leave?token=fixture" });
  await open(page);
  await page.getByRole("button", { name: "Unsubscribe", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText("doesn’t support direct unsubscribe");
  await expect(page.getByRole("link", { name: "Open unsubscribe page" })).toHaveCount(0);
  expect(mutations.filter(m => m.path.endsWith("/unsubscribe"))).toHaveLength(0);
  await page.route("**/api/**/unsubscribe", route => {
    if (route.request().method() === "GET") return route.fulfill({ contentType: "application/json", body: JSON.stringify({ kind: "mailto", to: "listunsubscribe-synthetic@email.example.org", subject: "Unsubscribe", body: "Please remove me.\n<not markup>" }) });
    mutations.push({ path: "/unsubscribe-mailto", body: route.request().postDataJSON() });
    return route.fulfill({ status: 204 });
  });
  await page.reload(); await expect(page.getByRole("region", { name: "Reading pane" })).toBeVisible();
  await page.getByRole("button", { name: "Unsubscribe", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Unsubscribe request sent" })).toBeVisible();
  await expect(page.getByLabel("To", { exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Review email", exact: true })).toHaveCount(0);
  expect(mutations.filter(m => m.path === "/unsubscribe-mailto")).toHaveLength(1);
});
test("unsupported, hostile and failed actions never silently send or retry", async ({ page }) => {
  const mutations = await fixture(page, null);
  await open(page);
  await expect(page.getByText("Hi Alex,", { exact: true })).toHaveCount(0); // rendered in sandbox, not app DOM
  await expect(page.getByRole("button", { name: "Unsubscribe", exact: true })).toHaveCount(0);
  await page.route("**/api/**/unsubscribe", route => route.fulfill({ contentType: "application/json", body: JSON.stringify({ kind: "web", url: "javascript:alert(1)" }) }));
  await page.reload(); await expect(page.getByRole("region", { name: "Reading pane" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Unsubscribe", exact: true })).toHaveCount(0);
  await page.unroute("**/api/**/unsubscribe");
  await page.route("**/api/**/unsubscribe", route => {
    if (route.request().method() === "GET") return route.fulfill({ contentType: "application/json", body: JSON.stringify({ kind: "one_click", url: "https://newsletter.example.org/u" }) });
    mutations.push({ path: "/unsubscribe", body: {} });
    return route.fulfill({ status: 502, contentType: "application/json", body: JSON.stringify({ code: "unknown", message: "private provider error" }) });
  });
  await page.reload(); await expect(page.getByRole("region", { name: "Reading pane" })).toBeVisible();
  await page.getByRole("button", { name: "Unsubscribe", exact: true }).click();

  await expect(page.getByRole("alert")).toContainText("No further request will be sent");
  await expect(page.getByRole("button", { name: "Confirm unsubscribe" })).toHaveCount(0);
  await page.screenshot({ path: "artifacts/unsubscribe-error-desktop-light.png", animations: "disabled" });
  expect(mutations.filter(m => m.path === "/unsubscribe")).toHaveLength(1);
});
test("favicon, touch and install icons are real canonical assets; login is branded", async ({ page }) => {
  await page.route("**/api/**", route => route.fulfill({ status: 401, contentType: "application/json", body: JSON.stringify({ code: "login_required", message: "Login required" }) }));
  await page.goto("/");
  await expect(page.getByRole("img", { name: "Dakia" })).toBeVisible();
  await expect(page.locator('link[rel="icon"][type="image/x-icon"]')).toHaveAttribute("href", "/favicon.ico");
  const manifest = await (await page.request.get("/manifest.webmanifest")).json();
  expect(manifest.icons).toEqual(expect.arrayContaining([
    expect.objectContaining({ sizes: "192x192", purpose: "any" }),
    expect.objectContaining({ sizes: "512x512", purpose: "any" }),
    expect.objectContaining({ sizes: "512x512", purpose: "maskable" }),
  ]));
  for (const path of ["/favicon-32.png", "/apple-touch-icon.png", ...manifest.icons.map((i: { src: string }) => i.src)]) {
    const response = await page.request.get(path);
    expect(response.ok()).toBe(true);
    expect(response.headers()["content-type"]).toContain("image/png");
    expect((await response.body()).subarray(0, 8).toString("hex")).toBe("89504e470d0a1a0a");
  }
  await page.screenshot({ path: "artifacts/branding-login-light.png" });
});

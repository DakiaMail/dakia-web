import { test, expect } from "@playwright/test";
import { accounts, content, page as mail } from "./fixtures";

const png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAFAAAAAgCAIAAAAKUWg7AAAAQklEQVR4nO3PAQkAIBDAQCMawKxmM4awP1iA3Tr7jmp9PwAGBgYGBgYeE3A94HrA9YDrAdcDrgdcD7gecD3gesD1HgWJJNNntUVKAAAAAElFTkSuQmCC";
for (const width of [390, 1480]) test(`honest independent image failures ${width}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 980 });
  let recover = false;
  const calls: string[] = [];
  const external: string[] = [];
  const errors = ["remote_image_unavailable", "remote_image_redirect_blocked", "remote_image_unsupported_content", "remote_image_too_large"];
  const html = "<h1>Image response regression</h1>" + errors.map((_, i) => `<img src="https://assets.example.org/${i}" width="80" height="32">`).join("");
  await page.route("**/*", async route => {
    const url = new URL(route.request().url());
    if (url.origin !== "http://127.0.0.1:4173") { external.push(url.href); return route.abort(); }
    const path = url.pathname;
    if (!path.startsWith("/api/")) return route.continue();
    const respond = (json: unknown) => route.fulfill({ json });
    if (path === "/api/session") return respond({ user_id: "fixture", csrf_token: "fixture" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": connected\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond(mail);
    if (path.endsWith("/conversation")) return respond({ rows: [content.message], continuation: null, complete: true });
    if (path.endsWith("/messages/m0")) return respond({ ...content, detected_language: "en", html });
    if (path.endsWith("/image")) {
      expect(route.request().headers()["x-csrf-token"]).toBe("fixture");
      const source = new URL(route.request().postDataJSON().url);
      calls.push(source.pathname);
      const index = Number(source.pathname.slice(1));
      if (recover && index === 0) return respond(png);
      return route.fulfill({ status: index === 0 ? 502 : 400, json: { code: errors[index], message: "upstream private URL must not appear" } });
    }
    if (path.endsWith("/operations")) { const op = route.request().postDataJSON(); return respond({ ...op, status: "applied", revision: op.expected_revision + 1 }); }
    return respond(null);
  });
  await page.goto("/");
  await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
  const frame = page.frameLocator('iframe[title="Message body"]');
  await expect(page.locator("summary")).toHaveText("4 images could not be loaded.");
  await expect(frame.locator("img")).toHaveCount(0);
  await page.locator("summary").click();
  for (const text of ["Image host unavailable or returned an error", "unsafe, invalid or too many redirects", "not a supported PNG", "5 MiB size limit"]) await expect(page.getByText(text, { exact: false })).toBeVisible();
  await expect(page.getByText("upstream private URL", { exact: false })).toHaveCount(0);
  recover = true;
  await page.getByRole("button", { name: "Retry loading images" }).click();
  await expect(page.locator(".image-status")).toHaveCount(0);
  await expect.poll(() => frame.locator("img").evaluateAll(nodes => nodes.length === 1 && (nodes[0] as HTMLImageElement).naturalWidth === 80)).toBe(true);
  expect(calls).toEqual(["/0", "/1", "/2", "/3", "/0", "/1", "/2", "/3"]);
  await expect(page.getByRole("button", { name: "Retry loading images" })).toHaveCount(0);
  expect(external).toEqual([]);
  await page.screenshot({ path: `artifacts/image-errors-${width}.png`, fullPage: true });
});

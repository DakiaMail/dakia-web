import { test, expect } from "@playwright/test";
import { accounts, content, page as mail } from "./fixtures";

// Controlled assets only, including the screenshot's 13 visible + hidden pixel shape.
const png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAFAAAAAgCAIAAAAKUWg7AAAAQklEQVR4nO3PAQkAIBDAQCMawKxmM4awP1iA3Tr7jmp9PwAGBgYGBgYeE3A94HrA9YDrAdcDrgdcD7gecD3gesD1HgWJJNNntUVKAAAAAElFTkSuQmCC";
for (const width of [390, 1480]) for (const theme of ["light", "dark"]) {
  test(`uncapped progressive images ${width} ${theme}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    const calls: number[] = [];
    const external: string[] = [];
    let reject = true;
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const html = '<h1>Event registration</h1><p>Tuesday 17 November</p>' +
      Array.from({ length: 13 }, (_, i) => `<img src="https://images.example.org/${i}.png" alt="Asset ${i}">`).join("") +
      '<img src="https://tracker.example.org/open" style="display:none;width:1px;height:1px">';
    await page.route("**/*", async route => {
      const url = new URL(route.request().url());
      if (url.origin !== (process.env.DAKIA_FIXTURE_ORIGIN || "http://127.0.0.1:4173")) { external.push(url.href); return route.abort(); }
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
        expect(["images.example.org", "tracker.example.org"]).toContain(source.hostname);
        const index = source.hostname === "tracker.example.org" ? 13 : Number(source.pathname.slice(1).split(".")[0]); calls.push(index);
        if (index === 1 && reject) return route.fulfill({ status: 400, json: { code: "remote_image_rejected", message: "remote image rejected" } });
        if (index === 12) await gate;
        return respond(png);
      }
      if (path.endsWith("/operations")) {
        const op = route.request().postDataJSON(); return respond({ ...op, status: "applied", revision: op.expected_revision + 1 });
      }
      return respond(null);
    });
    await page.addInitScript(theme => localStorage.setItem("dakia-theme", theme), theme);
    await page.goto("/");
    await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
    const readerHistory = await page.evaluate(() => history.length);
    const frame = page.frameLocator('iframe[title="Message body"]');
    await expect.poll(() => calls.length).toBe(13);
    await expect(frame.locator("img[src]")).toHaveCount(11);
    release();
    await expect.poll(() => calls.length).toBe(14);
    await expect(frame.locator("img[src]")).toHaveCount(13);
    await expect.poll(() => frame.locator("img").evaluateAll(nodes => nodes.every(node => (node as HTMLImageElement).naturalWidth === 80))).toBe(true);
    await expect(page.locator(".image-status")).toHaveCount(0);
    await page.screenshot({ path: `artifacts/images-partial-${width}-${theme}.png`, fullPage: true });
    reject = false;
    await expect(frame.locator("img[src]")).toHaveCount(13);
    expect(calls).toEqual(Array.from({ length: 14 }, (_, i) => i));
    expect(await page.evaluate(() => history.length)).toBe(readerHistory);
    await expect(page.getByRole("button", { name: "Retry loading images", exact: true })).toHaveCount(0);
    expect(external).toEqual([]);
    await expect(frame.locator('meta[http-equiv="Content-Security-Policy"]')).toHaveAttribute("content", /img-src data:/);
    await expect(page.locator('iframe[title="Message body"]')).toHaveAttribute("sandbox", "allow-scripts allow-popups allow-popups-to-escape-sandbox");
    await page.screenshot({ path: `artifacts/images-recovered-${width}-${theme}.png`, fullPage: true });
    if (width === 390) {
      await page.getByRole("button", { name: "Back to inbox" }).click();
      await page.getByRole("button", { name: "Open navigation" }).click();
    }
    await page.getByRole("button", { name: "Manage workspace" }).click();
    const block = page.getByRole("checkbox", { name: "Block external images by default" });
    const hidden = page.getByRole("checkbox", { name: "Block hidden images and tracking pixels" });
    await expect(block).not.toBeChecked();
    await expect(hidden).not.toBeChecked();
    await hidden.check();
    await page.screenshot({ path: `artifacts/image-settings-${width}-${theme}.png`, fullPage: true });
    await page.reload();
    calls.length = 0;
    await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
    await expect(frame.locator("img[src]")).toHaveCount(13);
    expect(calls).toEqual(Array.from({ length: 13 }, (_, i) => i));
    if (width === 390) {
      await page.getByRole("button", { name: "Back to inbox" }).click();
      await page.getByRole("button", { name: "Open navigation" }).click();
    }
    await page.getByRole("button", { name: "Manage workspace" }).click();
    await expect(hidden).toBeChecked();
    await block.check();
    await page.reload();
    calls.length = 0;
    await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
    await expect(page.getByRole("button", { name: "Load images", exact: true })).toBeVisible();
    expect(calls).toEqual([]);
    await page.getByRole("button", { name: "Load images", exact: true }).click();
    await expect(frame.locator("img[src]")).toHaveCount(13);
    expect(calls).toEqual(Array.from({ length: 13 }, (_, i) => i));
    expect(external).toEqual([]);
  });
}

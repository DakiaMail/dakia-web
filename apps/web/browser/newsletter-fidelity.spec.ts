import { test, expect } from "@playwright/test";
import { readFileSync, mkdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { accounts, content, page as mail } from "./fixtures";

const png = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aWQAAAABJRU5ErkJggg==";
const templates: Record<string, string> = {
  preview: readFileSync(new URL("./fixtures/hidden-preview.html", import.meta.url), "utf8"),
  offer: readFileSync(new URL("./fixtures/offer-social.html", import.meta.url), "utf8"),
};
// Optional owner-approved local-only bodies. Never commit these files or their
// personalized URLs; all image APIs and every external request are intercepted.
const privateDir = process.env.DAKIA_PRIVATE_NEWSLETTERS;
if (privateDir) for (const [name, file] of Object.entries({ substack: "private-substack-0.html", aakriti: "private-aakriti.html", aquaphor: "private-aquaphor-0.html", swedbank: "private-swedbank.html" })) {
  templates[name] = readFileSync(resolve(privateDir, file), "utf8");
}
const privateImages: Record<string, string> = privateDir ? JSON.parse(readFileSync(resolve(privateDir, "private-aquaphor-images.json"), "utf8")) : {};
for (const [name, html] of Object.entries(templates)) for (const width of [320, 390, 1480]) for (const dark of [false, true]) test(`newsletter fidelity ${name} ${width} ${dark ? "dark" : "light"}`, async ({ page }) => {
  await page.setViewportSize({ width, height: 1050 });
  const root = { ...content.message, unread: false, subject: `Newsletter review: ${name}`, sender: { name: "Newsletter fixture", email: "newsletter@example.invalid" } };
  const imageCalls: string[] = [], external: string[] = [], writes: string[] = [];
  const imagesEnabled = name === "offer" || name === "aquaphor";
  await page.addInitScript(({ dark, imagesEnabled }) => {
    localStorage.setItem("dakia-theme", dark ? "dark" : "light");
    localStorage.setItem("dakia:newsletter-fixture:block-hidden-images", "true");
    localStorage.setItem("dakia:newsletter-fixture:always-load-images", String(imagesEnabled));
  }, { dark, imagesEnabled });
  await page.route("**/*", route => {
    const url = new URL(route.request().url()), path = url.pathname;
    if (!url.hostname.match(/^(127\.0\.0\.1|localhost)$/)) { external.push(url.origin); return route.abort(); }
    if (!path.startsWith("/api/")) return route.continue();
    const respond = (json: unknown, status = 200) => route.fulfill({ json, status });
    if (path.endsWith("/image")) {
      const source = route.request().postDataJSON().url as string; imageCalls.push(source);
      if (name === "offer") return respond(png);
      return privateImages[source] ? respond(privateImages[source]) : respond({ code: "remote_image_unavailable" }, 502);
    }
    if (route.request().method() !== "GET") writes.push(path);
    if (path === "/api/session") return respond({ user_id: "newsletter-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows: [root] });
    if (path.endsWith("/conversation")) return respond({ rows: [root], complete: false, continuation: null });
    if (path.endsWith("/unsubscribe")) return respond(null);
    if (path.includes("/messages/")) return respond({ ...content, message: root, html, detected_language: "en", attachments: [] });
    return respond([]);
  });
  await page.goto("/");
  await page.locator(".message-open").click();
  const frame = page.frameLocator('iframe[title="Message body"]');
  await expect(frame.locator("body")).not.toBeEmpty();
  // Nonempty DOM does not prove the asynchronous sandbox has been resized or
  // painted. Wait for actual article content and its parent frame extent.
  if (name === "aakriti" || name === "substack") {
    await expect(frame.locator("h1").first()).toBeVisible();
    await expect(frame.locator("h1").first()).toContainText(name === "aakriti" ? "How Vision Actually Learned to Scale" : "Poland");
    await expect.poll(async () => (await page.locator('iframe[title="Message body"]').boundingBox())!.height).toBeGreaterThan(1000);
  }
  if (imagesEnabled) {
    const expected = name === "offer" ? (process.env.DAKIA_BASELINE ? 0 : 3) : (process.env.DAKIA_BASELINE ? 3 : 6);
    await expect.poll(() => imageCalls.length).toBe(expected);
    await expect(page.getByRole("button", { name: "Loading images…", exact: true })).toHaveCount(0);
  }
  // Capture the initial viewport as well as the offer/social section that the
  // owner reported. Screenshots are actual Chromium, not generated mockups.
  const out = process.env.DAKIA_BASELINE ? "artifacts/newsletter-baseline" : "artifacts/newsletter-fidelity";
  mkdirSync(out, { recursive: true });
  await page.screenshot({ path: `${out}/${name}-${width}-${dark ? "dark" : "light"}.png` });
  if (name === "aakriti" || name === "substack") {
    const paragraph = frame.locator("p").filter({ hasText: /\S.{120}/ }).last();
    await paragraph.scrollIntoViewIfNeeded();
    await expect(paragraph).toBeVisible();
    const bounds = (await paragraph.boundingBox())!;
    const toolbar = await page.locator(".reader-toolbar").filter({ visible: true }).last().boundingBox();
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(width <= 600 ? toolbar!.y : 1050);
    await page.screenshot({ path: `${out}/${name}-article-end-${width}-${dark ? "dark" : "light"}.png` });
    await frame.locator("h1").first().scrollIntoViewIfNeeded();
  }
  const preheaders = await frame.locator("body").evaluate(body => {
    return { textLength: body.textContent?.length, visiblePreviews: [...body.querySelectorAll("div")].filter(el => el.style.fontSize === "1px" && el.getBoundingClientRect().height > 0 && getComputedStyle(el).visibility !== "hidden").length, overflow: body.scrollWidth - innerWidth,
      headingTop: body.querySelector("h1")?.getBoundingClientRect().top,
      compactTables: [...body.querySelectorAll('table[width="110"]')].map(el => el.getBoundingClientRect().width),
      socialImages: body.querySelectorAll('img[width="30"]').length };
  });
  const button = frame.getByRole("link", { name: name === "aquaphor" ? "VAATA PAKKUMISI" : "VIEW OFFERS", exact: true });
  if (imagesEnabled) {
    await button.evaluate(el => el.scrollIntoView({ block: "center" }));
    await expect(button).toBeInViewport({ ratio: 1 });
    await page.screenshot({ path: `${out}/${name}-offer-${width}-${dark ? "dark" : "light"}.png` });
    if (name === "aquaphor") {
      const social = frame.locator('img[width="30"]').last();
      await social.evaluate(el => el.scrollIntoView({ block: "center" }));
      await expect(social).toBeInViewport({ ratio: 1 });
      await page.screenshot({ path: `${out}/${name}-social-${width}-${dark ? "dark" : "light"}.png` });
      await expect(page.getByText("Other images are still shown.", { exact: false })).toHaveCount(0);
      await expect(page.locator(".image-status")).toHaveCount(0);
      await expect(page.getByText("Image options", { exact: true })).toHaveCount(0);
    }
  }
  writeFileSync(`${out}/${name}-${width}-${dark ? "dark" : "light"}.json`, JSON.stringify({ ...preheaders, imagesRequested: imageCalls.length }, null, 2));
  if (!process.env.DAKIA_BASELINE) {
    expect(preheaders.visiblePreviews).toBe(0);
    expect(preheaders.overflow).toBeLessThanOrEqual(1);
    await expect(page.getByText("Showing available messages.", { exact: true })).toHaveCount(0);
    if (imagesEnabled) {
      expect(await button.evaluate(el => getComputedStyle(el.closest("td")!).textAlign)).toBe("center");
      if (name === "offer") await expect(frame.locator("img")).toHaveCount(3);
      if (name === "aquaphor") await expect(frame.locator('img[width="30"]')).toHaveCount(3);
      for (const image of await frame.locator('img[width="30"]').all()) expect(await image.evaluate(el => (el as HTMLImageElement).naturalWidth)).toBeGreaterThan(0);
      expect(imageCalls.every(url => url.startsWith("https:"))).toBe(true);
      expect(imageCalls.some(url => /\/pixel|\/image\/ui\//.test(url))).toBe(false);
    }
    await expect(frame.locator("script:not([nonce]),form,input,iframe,svg")).toHaveCount(0);
    expect(await page.locator('iframe[title="Message body"]').getAttribute("sandbox")).not.toContain("allow-same-origin");
  }
  expect(external).toEqual([]); expect(writes).toEqual([]);
});

test("legacy social image failure stays isolated without routine footer controls", async ({ page }) => {
  const root = { ...content.message, unread: false };
  const calls: string[] = []; let fail = true;
  await page.addInitScript(() => localStorage.setItem("dakia:image-fixture:block-hidden-images", "true"));
  await page.route("**/api/**", route => {
    const path = new URL(route.request().url()).pathname;
    const respond = (json: unknown, status = 200) => route.fulfill({ json, status });
    if (path.endsWith("/image")) {
      const source = route.request().postDataJSON().url as string; calls.push(source);
      if (source.endsWith("social-two.png") && fail) return respond({ code: "remote_image_redirect_blocked" }, 400);
      return respond(png);
    }
    if (path === "/api/session") return respond({ user_id: "image-fixture", csrf_token: "fixture" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows: [root] });
    if (path.endsWith("/conversation")) return respond({ rows: [root], complete: true, continuation: null });
    if (path.endsWith("/unsubscribe")) return respond(null);
    if (path.includes("/messages/")) return respond({ ...content, message: root, html: templates.offer, detected_language: "en", attachments: [] });
    return respond([]);
  });
  await page.goto("/"); await page.locator(".message-open").click();
  const frame = page.frameLocator('iframe[title="Message body"]');
  await expect(frame.locator("img")).toHaveCount(2);
  expect(calls).toHaveLength(3);
  await expect(page.locator(".image-status")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Retry loading images", exact: true })).toHaveCount(0);
  expect(calls.filter(url => url.endsWith("social-one.png"))).toHaveLength(1);
  expect(calls.every(url => url.startsWith("https:"))).toBe(true);
});

import { test, expect } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { accounts, content, page as mail } from "./fixtures";

// Fictional mail reproducing legacy table buttons and authored gutters only.
const html = `<table width="100%" cellpadding="0" cellspacing="0"><tr><td colspan="3"><br></td></tr><tr><td width="10"></td><td><table cellpadding="0" cellspacing="0"><tr><td style="background-color:#ffecff;padding:12px"><h1>Customer rewards</h1><p>Thanks for being a customer.</p><table cellpadding="0" cellspacing="0" style="border-radius:24px;>background:#ff5f00;background-image:linear-gradient(#ff5f00,#ff5f00)"><tr><td bgcolor="#ff5f00" style="padding:8px 16px;border-radius:24px"><a href="https://example.org/entries?fixture=1&amp;source=mail" style="font-size:18px;line-height:24px;font-weight:700;text-decoration:none;color:white;display:inline-block">View reward entries</a></td></tr></table></td></tr></table><p><a href="https://example.org/terms" style="color:#257886;text-decoration:underline">Campaign terms</a></p><p><a href="mailto:help@example.org">Contact</a></p><a href="javascript:alert(1)">Unsafe script</a><a href="data:text/html,bad">Unsafe data</a><a href="//example.org">Unsafe relative</a><a href="https://example.org/css" style="background:#257886;color:white;display:block;border-radius:8px;padding:12px;text-decoration:none;position:relative;background-image:url(https://tracker.example.org/pixel)">CSS button</a></td><td width="10"></td></tr></table><script>parent.document.body.textContent='bad'</script><form><input></form>`;

for (const width of [320, 390, 600, 1480, 1920]) for (const theme of ["light", "dark"]) test(`email button visuals, gutters and safe navigation ${width} ${theme}`, async ({ page, context }) => {
  await page.setViewportSize({ width, height: 980 });
  await page.addInitScript(theme => localStorage.setItem("dakia-theme", theme), theme);
  const navigations: string[] = [];
  await context.route("**/*", route => {
    const url = new URL(route.request().url());
    if (url.origin !== (process.env.DAKIA_FIXTURE_ORIGIN || "http://127.0.0.1:4173")) { navigations.push(url.href); return route.fulfill({ contentType: "text/html", body: "Intercepted fixture destination" }); }
    if (!url.pathname.startsWith("/api/")) return route.continue();
    const path = url.pathname;
    const respond = (json: unknown) => route.fulfill({ json });
    if (path === "/api/session") return respond({ user_id: "fixture", csrf_token: "fixture" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": connected\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond(mail);
    if (path.endsWith("/conversation")) return respond({ rows: [content.message], continuation: null, complete: true });
    if (path.endsWith("/messages/m0")) return respond({ ...content, html, detected_language: "en" });
    if (path.endsWith("/operations")) { const op = route.request().postDataJSON(); return respond({ ...op, status: "applied", revision: op.expected_revision + 1 }); }
    return respond(null);
  });
  await page.goto("/");
  await page.getByRole("button", { name: /Maya Chen.*A few thoughts/ }).click();
  const frame = page.frameLocator('iframe[title="Message body"]');
  const cta = frame.getByRole("link", { name: "View reward entries" });
  await expect(cta).toHaveCSS("color", "rgb(255, 255, 255)");
  await expect(cta).toHaveCSS("text-decoration-line", "none");
  await expect(cta).toHaveCSS("display", "inline-block");
  await expect(cta.locator("..")).toHaveCSS("background-color", "rgb(255, 95, 0)");
  await expect(cta.locator("..")).toHaveCSS("border-radius", "24px");
  await expect(frame.getByRole("link", { name: "CSS button" })).toHaveCSS("background-color", "rgb(37, 120, 134)");
  await expect(frame.getByRole("link", { name: "CSS button" })).toHaveCSS("background-image", "none");
  await expect(frame.getByRole("link", { name: "CSS button" })).toHaveCSS("position", "relative");
  const terms = frame.getByRole("link", { name: "Campaign terms" });
  await expect(terms).toHaveCSS("color", "rgb(37, 120, 134)");
  await expect(terms).toHaveCSS("text-decoration-line", "underline");
  await expect(frame.getByRole("link", { name: "Contact" })).toHaveAttribute("href", "mailto:help@example.org");
  await expect(frame.locator("a:not([href])")).toHaveCount(3);
  await expect(frame.locator("script:not([nonce]),form,input")).toHaveCount(0);
  await expect(page.locator("iframe")).toHaveAttribute("sandbox", "allow-scripts allow-popups allow-popups-to-escape-sandbox");
  expect(navigations).toEqual([]);
  if (width <= 600) {
    const iframe = await page.locator("iframe").boundingBox();
    expect(iframe!.x).toBe(0);
    expect(iframe!.width).toBe(width);
    const panel = await frame.locator("h1").locator("..").boundingBox();
    expect(panel!.x).toBe(22);
    expect(panel!.width).toBeLessThanOrEqual(width - 44);
    expect(panel!.width).toBeGreaterThanOrEqual((await cta.locator("..").boundingBox())!.width + 24);
    expect((await cta.locator("..").boundingBox())!.height).toBeGreaterThanOrEqual(40);
    const toolbar = page.locator(".reader > .reader-toolbar");
    expect((await toolbar.boundingBox())!.y).toBeGreaterThan(880);
    expect((await toolbar.getByRole("button", { name: "More actions", exact: true }).boundingBox())!.width).toBeGreaterThanOrEqual(44);
  }
  expect(await frame.locator("body").evaluate(node => node.scrollWidth - node.clientWidth)).toBeLessThanOrEqual(1);
  mkdirSync("artifacts/email-buttons", { recursive: true });
  await cta.scrollIntoViewIfNeeded();
  await page.screenshot({ path: `artifacts/email-buttons/${width}-${theme}.png` });
  for (const [link, destination, keyboard] of [[cta, "https://example.org/entries?fixture=1&source=mail", false], [terms, "https://example.org/terms", true]] as const) {
    await expect(link).toHaveAttribute("target", "_blank");
    await expect(link).toHaveAttribute("rel", "noopener noreferrer");
    const popupPromise = context.waitForEvent("page");
    if (keyboard) { await link.focus(); await page.keyboard.press("Enter"); } else await link.click();
    const popup = await popupPromise;
    await popup.waitForLoadState();
    expect(popup.url()).toBe(destination);
    expect(await popup.evaluate(() => window.opener === null)).toBe(true);
    await popup.close();
  }
  expect(navigations).toEqual(["https://example.org/entries?fixture=1&source=mail", "https://example.org/terms"]);
});

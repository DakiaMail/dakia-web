import { test, expect } from "@playwright/test";
import { mkdirSync, writeFileSync } from "node:fs";
import type { Message } from "@dakia/api-types";
import bodies from "./fixtures/translation-bodies.json" with { type: "json" };

for (const width of [390, 1480]) for (const dark of [false, true]) test(`actual body detection, navigation and explicit translation ${width} ${dark ? "dark" : "light"}`, async ({ page, context, request }) => {
  await page.setViewportSize({ width, height: 980 });
  await context.addCookies([{ name: "__Host-dakia", value: "translation-browser-fixture", url: "https://127.0.0.1:4336/", secure: true, sameSite: "Lax" }]);
  const rows: Message[] = await (await request.get("/__translation/messages")).json();
  const account = rows[0].account_id;
  const external: string[] = [];
  const models: string[] = [];
  let translations = 0;
  await page.addInitScript(({ dark }) => {
    localStorage.setItem("dakia-theme", dark ? "dark" : "light");
    const observations: { value: string; time: number }[] = [];
    Object.assign(window, { translationObservations: observations });
    document.addEventListener("click", event => {
      if (event.target instanceof Element && event.target.closest(".message-open")) Object.assign(window, { translationNavigationStart: performance.now() });
    }, true);
    new MutationObserver(() => {
      const source = document.querySelector<HTMLSelectElement>('[aria-label="Translate from"]');
      const value = source?.value || "";
      if (observations.at(-1)?.value !== value) observations.push({ value, time: performance.now() });
    }).observe(document, { subtree: true, childList: true, attributes: true });
  }, { dark });
  await page.route("**/*", async route => {
    const url = new URL(route.request().url());
    const path = url.pathname;
    if (url.origin !== "http://127.0.0.1:4336") { external.push(url.origin); return route.abort(); }
    const respond = (json: unknown) => route.fulfill({ json });
    if (!path.startsWith("/api/")) return route.continue();
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": fixture\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ rows: rows.filter(m => m.account_id === account), continuation: null, coverage: [], classification: { classified: 0, total: 0 }, pending: false, counts_exact: true });
    if (path.endsWith("/conversation")) return respond({ rows: rows.filter(m => path.includes(`/accounts/${m.account_id}/messages/${m.id}/`)), continuation: null, complete: true });
    if (path.endsWith("/unsubscribe")) return respond(null);
    if (path.endsWith("/image")) return respond("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=");
    if (path.endsWith("/translation") || path.includes("/jobs/")) {
      models.push(`${route.request().method()} ${path}`);
      const m = rows.find(m => path.includes(`/messages/${m.id}/`)) || rows.find(m => m.id === "estonian-english-footer")!;
      if (route.request().method() === "POST") {
        expect(url.searchParams.get("language")).toBe("et");
        translations++;
      }
      if (path.endsWith("/translation") && route.request().method() === "GET") return respond({ message: m, html: "<p>Your meeting documents are ready. We will discuss the next steps tomorrow.</p>", text: "Your meeting documents are ready.", attachments: [] });
      return respond({ id: "fixture-translation", account_id: account, message_id: m.id, content_revision: m.revision, model_revision: "synthetic", kind: "translate", status: "succeeded", deadline: "2026-11-03T00:00:00Z" });
    }
    // Actual authorization, received cache and offline detection handler.
    return route.continue();
  });
  await page.goto("/");
  const timings: unknown[] = [];
  for (const body of bodies) {
    // Direct navigation is the application's real notification/deep-link path.
    await page.goto(`/?account=${account}&message=${body.name}`);
    const frame = page.frameLocator('iframe[title="Message body"]');
    await expect(frame.locator("body")).toBeVisible();
    if (body.expected && body.expected !== "en") {
      await expect(page.getByLabel("Translate from", { exact: true })).toHaveValue(body.expected);
      await expect(page.getByRole("button", { name: "Translate to English", exact: true })).toBeVisible();
    } else {
      await expect(page.getByLabel("Translate from", { exact: true })).toHaveCount(0);
    }
    expect(models).toEqual([]);
    if (body.name === "english-monthly-review" || body.name === "estonian-english-footer" || body.name === "russian") {
      await expect(frame.locator("body")).toContainText(body.name === "english-monthly-review" ? "Here is your monthly viewing summary" : body.name === "russian" ? "Пожалуйста" : "Tere hommikust");
      await expect(frame.locator("body")).not.toHaveCSS("visibility", "hidden");
      mkdirSync("artifacts/translation", { recursive: true });
      await page.screenshot({ path: `artifacts/translation/${body.name}-${width}-${dark ? "dark" : "light"}.png`, fullPage: true });
    }
    timings.push(await page.evaluate(name => {
      const resources = performance.getEntriesByType("resource") as PerformanceResourceTiming[];
      const body = resources.find(e => new URL(e.name).pathname.endsWith(`/messages/${name}`));
      const observations = (window as unknown as { translationObservations: { value: string; time: number }[] }).translationObservations;
      const control = observations.find(o => o.value);
      return { name, body_request_ms: body ? body.responseEnd - body.startTime : null, control_after_body_ms: body && control ? control.time - body.responseEnd : null };
    }, body.name));
  }
  if (width === 1480) {
    await page.goto(`/?account=${account}&message=english-monthly-review`);
    await expect(page.frameLocator('iframe[title="Message body"]').getByText(/Here is your monthly viewing summary/)).toBeVisible();
    let release!: () => void;
    let waiting = false;
    const gate = new Promise<void>(resolve => { release = resolve; });
    await page.route(`**/api/accounts/${account}/messages/estonian-english-footer`, async route => {
      waiting = true;
      await gate;
      await route.continue().catch(() => {});
    });
    await page.locator(".message-open").filter({ hasText: "estonian-english-footer" }).click();
    await expect.poll(() => waiting).toBe(true);
    await expect(page.getByLabel("Translate from", { exact: true })).toHaveCount(0);
    let russianReads = 0;
    page.on("request", r => { if (new URL(r.url()).pathname.endsWith("/messages/russian")) russianReads++; });
    await page.locator(".message-open").filter({ hasText: "russian" }).click();
    await expect(page.getByLabel("Translate from", { exact: true })).toHaveValue("ru");
    release();
    await expect(page.frameLocator('iframe[title="Message body"]').getByText(/Пожалуйста/)).toBeVisible();
    const reads = russianReads;
    await page.locator(".message-open").filter({ hasText: "english-monthly-review" }).click();
    await expect(page.getByLabel("Translate from", { exact: true })).toHaveCount(0);
    await page.locator(".message-open").filter({ hasText: "russian" }).click();
    await expect(page.getByLabel("Translate from", { exact: true })).toHaveValue("ru");
    timings.push(await page.evaluate(additional => {
      const data = window as unknown as { translationNavigationStart: number; translationObservations: { value: string; time: number }[] };
      return { name: "warm-reopen-russian", click_to_control_ms: data.translationObservations.at(-1)!.time - data.translationNavigationStart, additional_body_requests: additional };
    }, russianReads - reads));
    expect(russianReads).toBe(reads);
  }
  await page.goto(`/?account=${account}&message=estonian-english-footer`);
  await page.getByRole("button", { name: "Translate to English", exact: true }).click();
  await expect(page.getByRole("button", { name: "Show original", exact: true })).toBeVisible();
  await expect(page.frameLocator('iframe[title="Translated message"]').getByText(/Your meeting documents/)).toBeVisible();
  await page.getByRole("button", { name: "Show original", exact: true }).click();
  await expect(page.frameLocator('iframe[title="Message body"]').getByText(/Tere hommikust/)).toBeVisible();
  await page.getByRole("button", { name: "Show translation", exact: true }).click();
  expect(translations).toBe(1);
  await page.goto(`/?account=${rows.at(-1)!.account_id}&message=russian`);
  await expect(page.getByLabel("Translate from", { exact: true })).toHaveValue("ru");
  await expect(page.frameLocator('iframe[title="Message body"]').getByText(/Пожалуйста/)).toBeVisible();
  expect(external).toEqual([]);
  mkdirSync("artifacts/translation", { recursive: true });
  writeFileSync(`artifacts/translation/timings-${width}-${dark ? "dark" : "light"}.json`, JSON.stringify(timings, null, 2));
});

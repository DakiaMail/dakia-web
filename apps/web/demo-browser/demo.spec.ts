import { test, expect, type Page } from "@playwright/test";

async function tabTo(page: Page, name: string) {
  for (let i = 0; i < 350; i++) {
    if (await page.evaluate(name => {
      const element = document.activeElement;
      return element?.getAttribute("aria-label") === name || element?.getAttribute("title") === name || element?.textContent?.trim() === name;
    }, name)) return;
    await page.keyboard.press("Tab");
  }
  throw new Error(`Could not reach ${name} with Tab`);
}

test("keyboard compose, send, persistence, reset, and isolated visitors without API traffic", async ({ page, browser, baseURL }) => {
  const requests: string[] = [];
  const origin = new URL(baseURL!).origin;
  page.on("request", req => { if (/\/api\/|\/auth\//.test(req.url()) || !req.url().startsWith(origin)) requests.push(req.url()); });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Inbox", exact: true })).toBeVisible();
  await expect(page.getByText("Demo. No real mail is sent.", { exact: true })).toHaveCount(0);
  await tabTo(page, "Compose"); await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "New message" });
  await expect(dialog).toBeVisible();
  await tabTo(page, "To");
  await page.keyboard.type("friend@example.invalid");
  await tabTo(page, "Subject");
  await page.keyboard.type("A note from the keyboard");
  await tabTo(page, "Message");
  await page.keyboard.type("Everything stays in this browser.");
  await tabTo(page, "Send");
  await page.keyboard.press("Enter");
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("status").filter({ hasText: "Sent" })).toBeVisible({ timeout: 16000 });
  await expect(page.getByRole("button", { name: "Compose", exact: true })).toBeFocused();
  await tabTo(page, "Sent"); await page.keyboard.press("Enter");
  await expect(page.getByRole("button", { name: /A note from the keyboard/ })).toBeVisible();
  await page.reload();
  await page.getByRole("navigation", { name: "Mailboxes" }).getByRole("button", { name: "Sent", exact: true }).click();
  await expect(page.getByRole("button", { name: /A note from the keyboard/ })).toBeVisible();
  const other = await browser.newContext();
  const otherPage = await other.newPage();
  await otherPage.goto(page.url());
  await otherPage.getByRole("navigation", { name: "Mailboxes" }).getByRole("button", { name: "Sent", exact: true }).click();
  await expect(otherPage.getByRole("button", { name: /A note from the keyboard/ })).toHaveCount(0);
  await other.close();
  await page.goto("/?reset=1");
  await expect(page.getByRole("heading", { name: "Inbox", exact: true })).toBeVisible();
  expect(page.url()).not.toContain("reset=");
  await page.getByRole("navigation", { name: "Mailboxes" }).getByRole("button", { name: "Sent", exact: true }).click();
  await expect(page.getByRole("button", { name: /A note from the keyboard/ })).toHaveCount(0);
  expect(requests).toEqual([]);
});

test("local search, reading, download, translation, unsubscribe, and blocked external actions", async ({ page }) => {
  const requests: string[] = [];
  page.on("request", req => { if (/\/api\/|\/auth\//.test(req.url())) requests.push(req.url()); });
  await page.goto("/");
  const search = page.getByRole("textbox", { name: "Search all mail" });
  await search.fill('subject:"Les maquettes"'); await search.press("Enter");
  await page.getByRole("region", { name: "Messages", exact: true }).getByRole("button", { name: /Sofia Laurent/ }).first().click();
  await page.getByRole("button", { name: "Translate to English", exact: true }).click();
  await expect(page.getByRole("button", { name: "Show original", exact: true })).toBeVisible();
  await expect(page.frameLocator('iframe[title="Translated message"]').locator("body")).toContainText("new designs");
  await search.fill('subject:"A slower kind"'); await search.press("Enter");
  await page.getByRole("region", { name: "Messages", exact: true }).getByRole("button", { name: /The Reading Room/ }).first().click();
  await page.getByRole("button", { name: "Unsubscribe", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "Unsubscribe request sent" })).toBeVisible();
  await page.keyboard.press("Escape");
  await search.fill('subject:"A few thoughts"'); await search.press("Enter");
  await page.getByRole("region", { name: "Messages", exact: true }).getByRole("button", { name: /Maya Chen/ }).first().click();
  const download = page.waitForEvent("download");
  await page.getByRole("link", { name: /Launch review.txt/ }).click();
  expect((await download).suggestedFilename()).toBe("Launch review.txt");
  const results = await page.evaluate(async () => {
    const paths = ["/api/new-route", "/api/accounts/gmail/start", "/api/push/subscriptions", "/auth/login", "https://example.invalid/send"];
    return Promise.all(paths.map(async path => (await fetch(path, { method: "POST", body: "{}" })).status));
  });
  expect(results.every(status => status >= 400)).toBe(true);
  expect(requests).toEqual([]);
});

for (const theme of ["light", "dark"] as const) {
  for (const viewport of [{ width: 1480, height: 980 }, { width: 390, height: 640 }, { width: 980, height: 480 }]) {
    test(`shared app screens ${theme} ${viewport.width}x${viewport.height}`, async ({ page }) => {
      await page.setViewportSize(viewport); await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
      await page.addInitScript(theme => localStorage.setItem("dakia-theme", theme), theme);
      await page.goto("/");
      await expect(page.getByRole("heading", { name: "Inbox", exact: true })).toBeVisible();
      await expect(page.locator("body")).not.toContainText("Sent in demo");
      await page.screenshot({ path: `artifacts/demo/inbox-${theme}-${viewport.width}.png`, fullPage: true, animations: "disabled" });
      await page.keyboard.press("c");
      await expect(page.getByRole("dialog", { name: "New message" })).toBeVisible();
      await page.screenshot({ path: `artifacts/demo/composer-${theme}-${viewport.width}.png`, fullPage: true, animations: "disabled" });
      await page.keyboard.press("Escape");
      await expect(page.getByRole("dialog")).toHaveCount(0);
    });
  }
}

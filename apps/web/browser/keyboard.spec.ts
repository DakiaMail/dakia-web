import { test, expect, type Page } from "@playwright/test";
import { accounts, content, page as mail } from "./fixtures";

async function fixture(page: Page) {
  const writes: string[] = [];
  const rows = mail.rows.slice(0, 3).map(m => ({ ...m, unread: false }));
  await page.route("**/api/**", route => {
    const req = route.request();
    const path = new URL(req.url()).pathname;
    if (req.method() !== "GET") writes.push(path);
    const respond = (value: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "keyboard-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows });
    if (path.endsWith("/recipients")) return respond([{ name: "Example", email: "example@example.org" }]);
    const message = rows.find(m => path.includes(`/messages/${m.id}`)) || rows[0];
    if (path.endsWith("/conversation")) return respond({ rows: [message], continuation: null, complete: true });
    if (path.includes("/messages/")) return respond({ ...content, message });
    return respond([]);
  });
  await page.goto("/");
  await page.getByRole("tab", { name: "List", exact: true }).click();
  await page.locator(".message-open").first().click();
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  return writes;
}

test("browser/OS modifier matrix never composes, navigates, archives or cancels defaults", async ({ page }) => {
  const writes = await fixture(page);
  // Synthetic events verify macOS Meta, Windows/Linux Ctrl, Alt/AltGraph-like
  // combinations and IME even when the Linux browser has no native binding.
  const failures = await page.evaluate(() => {
    const targets: Element[] = [document.body, document.querySelector(".message-open")!];
    for (const html of ['<input>', '<textarea></textarea>', '<select><option>c</option></select>', '<div contenteditable><span>editable</span></div>', '<div contenteditable="plaintext-only"><span>plain</span></div>', '<div role="textbox" tabindex="0"></div>']) {
      const host = document.createElement("div"); host.innerHTML = html; document.body.append(host);
      targets.push(host.querySelector("span") || host.firstElementChild!);
    }
    const failures: string[] = [];
    for (const [i, target] of targets.entries()) {
      for (const modifiers of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }, { ctrlKey: true, altKey: true }, { ctrlKey: true, shiftKey: true }, { metaKey: true, shiftKey: true }]) {
        for (const key of ["c", "C", "v", "x", "a", "z", "y", "f", "l", "r", "t", "w", "p", "e", "j", "k", "ArrowDown", "ArrowUp", "Escape"]) {
          const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...modifiers });
          if (!target.dispatchEvent(event)) failures.push(`${i}:${key}:${JSON.stringify(modifiers)}`);
        }
      }
      for (const key of ["c", "e", "j", "k", "ArrowDown", "ArrowUp", "Escape"]) {
        const event = new KeyboardEvent("keydown", { key, isComposing: true, bubbles: true, cancelable: true });
        if (!target.dispatchEvent(event)) failures.push(`${i}:IME:${key}`);
      }
    }
    return failures;
  });
  expect(failures).toEqual([]);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  await expect(page.locator(".message-row.selected .message-open")).toContainText(mail.rows[0].subject);
  expect(writes).toEqual([]);
  await expect(page.locator(".search kbd")).toHaveCount(0);
  await expect(page.locator(".keyboard-hints")).toHaveText("Tab Navigate controls Enter Activate");
});

test("native copy preserves selected message text; editable copy/cut/paste/select-all/undo work", async ({ page, context }) => {
  const writes = await fixture(page);
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.evaluate(() => {
    const text = document.createElement("p"); text.id = "copy-fixture"; text.textContent = "Synthetic copied message text";
    document.body.append(text);
    const range = document.createRange(); range.selectNodeContents(text);
    getSelection()!.removeAllRanges(); getSelection()!.addRange(range);
  });
  await page.keyboard.press("ControlOrMeta+c");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("Synthetic copied message text");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  // No hidden app action should run while text is selected, even for bare keys.
  for (const key of ["c", "e", "j", "k", "Escape"]) await page.keyboard.press(key);
  await expect(page.locator(".conversation-message")).toHaveCount(1);
  await page.evaluate(() => {
    getSelection()!.removeAllRanges();
    const input = document.createElement("textarea"); input.id = "native-editor"; document.body.append(input); input.focus();
  });
  const editor = page.locator("#native-editor");
  await page.keyboard.type("Native editor text");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("ControlOrMeta+c");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe("Native editor text");
  await page.keyboard.press("ControlOrMeta+x");
  await expect(editor).toHaveValue("");
  await page.keyboard.press("ControlOrMeta+v");
  await expect(editor).toHaveValue("Native editor text");
  await page.keyboard.press("ControlOrMeta+z");
  await expect(editor).toHaveValue("");
  expect(writes).toEqual([]);
});

test("compose button remains keyboard accessible and dialog keys stay local", async ({ page }) => {
  await fixture(page);
  const compose = page.getByRole("button", { name: "Compose", exact: true }).filter({ visible: true }).first();
  await compose.focus(); await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "New message" });
  await expect(dialog).toBeVisible();
  for (const init of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }, { isComposing: true }, { repeat: true }]) {
    await dialog.evaluate((el, init) => el.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true, ...init })), init);
    await expect(dialog).toBeVisible();
  }
  const to = dialog.getByRole("textbox", { name: "To", exact: true });
  await to.fill("ex");
  await expect(dialog.getByRole("listbox").getByRole("option")).toBeVisible();
  const before = await to.inputValue();
  expect(await to.evaluate(el => el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, cancelable: true })))).toBe(true);
  expect(await to.inputValue()).toBe(before);
  expect(await to.evaluate(el => el.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", isComposing: true, bubbles: true, cancelable: true })))).toBe(true);
  await page.keyboard.press("Escape");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("listbox")).toHaveCount(0);
  await to.blur(); await to.focus();
  await expect(dialog.getByRole("listbox")).toBeVisible();
  await page.keyboard.press("ArrowDown"); await page.keyboard.press("Enter");
  await expect(to).toHaveValue("example@example.org, ");
  await dialog.getByRole("heading", { name: "New message" }).click();
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(page.locator(".conversation-message")).toHaveCount(1);
});

test("tab and range keys only act without OS modifiers and leave native focus navigation", async ({ page }) => {
  await fixture(page);
  const list = page.getByRole("tab", { name: "List", exact: true });
  await list.focus();
  for (const init of [{ ctrlKey: true }, { metaKey: true }, { altKey: true }, { shiftKey: true }, { isComposing: true }]) {
    expect(await list.evaluate((el, init) => el.dispatchEvent(new KeyboardEvent("keydown", { key: "Home", bubbles: true, cancelable: true, ...init })), init)).toBe(true);
    await expect(list).toHaveAttribute("aria-selected", "true");
  }
  await page.keyboard.press("Home");
  await expect(page.getByRole("tab", { name: "Smart", exact: true })).toBeFocused();
  await page.keyboard.press("End");
  await expect(list).toBeFocused();
  const checks = page.locator(".message-select input");
  await checks.first().focus(); await page.keyboard.press("Space");
  await checks.nth(2).focus(); await page.keyboard.press("Shift+Space");
  await expect(page.locator(".message-select input:checked")).toHaveCount(3);
  for (const ctrl of ["ctrlKey", "metaKey", "altKey"]) {
    expect(await checks.nth(1).evaluate((el, ctrl) => el.dispatchEvent(new KeyboardEvent("keydown", { key: " ", shiftKey: true, [ctrl]: true, bubbles: true, cancelable: true })), ctrl)).toBe(true);
  }
  await expect(page.locator(".message-select input:checked")).toHaveCount(3);
  await page.locator("h1").click(); await page.keyboard.press("Escape");
  await expect(page.locator(".message-select input:checked")).toHaveCount(0);
  await expect(page.locator(".conversation-message")).toHaveCount(0);
});

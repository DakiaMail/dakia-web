import { test, expect, type Page } from "@playwright/test";
import { accounts, content, page as mail } from "./fixtures";

async function fixture(page: Page) {
  const writes: { target_id?: string; change?: { kind: string; value?: boolean } }[] = [];
  const rows = mail.rows.slice(0, 3).map(m => ({ ...m, unread: false }));
  await page.route("**/api/**", route => {
    const req = route.request();
    const path = new URL(req.url()).pathname;

    const respond = (value: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(value) });
    if (path.endsWith("/operations") && req.method() === "POST") {
      const op = req.postDataJSON(); writes.push(op);
      return respond({ ...op, status: "applied", revision: 2 });
    }
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
  await expect(page.locator(".keyboard-hints")).toHaveText(/Tab Navigate controls.*Enter Activate/);
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


test("mail navigation, selection, read, unread, stars and removal use the intended targets", async ({ page }) => {
  const writes = await fixture(page);
  await page.locator(".message-open").first().focus();
  await page.keyboard.press("j");
  await expect(page.locator(".message-row.selected .message-open")).toContainText(mail.rows[1].subject);
  await expect(page.getByRole("region", { name: "Reading pane" })).toBeFocused();
  await page.keyboard.press("j");
  await expect(page.locator(".message-row.selected .message-open")).toContainText(mail.rows[2].subject);
  await page.keyboard.press("k");
  await expect(page.locator(".message-row.selected .message-open")).toContainText(mail.rows[1].subject);
  await page.keyboard.press("k");
  await expect(page.locator(".message-row.selected .message-open")).toContainText(mail.rows[0].subject);
  await page.keyboard.press("u");
  await expect.poll(() => writes.length).toBe(1);
  expect(writes[0]).toMatchObject({ target_id: mail.rows[0].id, change: { kind: "read", value: false } });
  await expect(page.getByRole("button", { name: "Mark unread", exact: true })).toBeEnabled();
  await page.keyboard.press("Shift+u");
  await expect.poll(() => writes.length).toBe(2);
  expect(writes[1]).toMatchObject({ change: { kind: "read", value: true } });
  await expect(page.getByRole("button", { name: "Mark unread", exact: true })).toBeEnabled();
  await page.keyboard.press("s");
  await expect.poll(() => writes.length).toBe(3);
  expect(writes[2]).toMatchObject({ change: { kind: "star", value: !mail.rows[0].starred } });
  await expect(page.getByRole("button", { name: "Mark unread", exact: true })).toBeEnabled();
  await page.keyboard.press("x");
  await expect(page.locator(".message-select input:checked")).toHaveCount(1);
  await page.keyboard.press("j");
  await expect(page.locator(".message-row.selected .message-open")).toContainText(mail.rows[1].subject);
  await page.keyboard.press("e");
  await expect.poll(() => writes.length).toBe(4);
  expect(writes[3]).toMatchObject({ target_id: mail.rows[0].id, change: { kind: "archive" } });
  await expect(page.locator(".message-select input:checked")).toHaveCount(0);
  await page.keyboard.press("Delete");
  await expect.poll(() => writes.length).toBe(5);
  expect(writes[4]).toMatchObject({ target_id: mail.rows[1].id, change: { kind: "trash" } });
});

test("reply, reply all, forward, compose and field navigation work without a pointer", async ({ page }) => {
  await fixture(page);
  await page.locator("h1").click();
  for (const key of ["r", "Shift+r", "f", "c"]) {
    await page.keyboard.press(key);
    const dialog = page.getByRole("dialog");
    await expect(dialog).toBeVisible();
    const to = dialog.getByRole("textbox", { name: "To", exact: true });
    const extraFields = dialog.getByRole("button", { name: "Cc / Bcc", exact: true });
    if (await extraFields.count()) {
      await extraFields.focus(); await page.keyboard.press("Enter");
    }
    await to.focus();
    if (await to.getAttribute("aria-expanded") === "true") await page.keyboard.press("Escape");
    await page.keyboard.press("Tab");
    await expect(dialog.getByRole("textbox", { name: "Cc", exact: true })).toBeFocused();
    await page.keyboard.press("Tab");
    await expect(dialog.getByRole("textbox", { name: "Bcc", exact: true })).toBeFocused();
    await page.keyboard.press("Tab");
    await expect(dialog.getByRole("textbox", { name: "Subject", exact: true })).toBeFocused();
    await page.keyboard.press("Shift+Tab");
    await expect(dialog.getByRole("textbox", { name: "Bcc", exact: true })).toBeFocused();
    // Mail action letters inside a composer must remain text.
    await to.fill("ceujkr@example.org");
    await expect(to).toHaveValue("ceujkr@example.org");
    await dialog.getByRole("heading").click();
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
  }
  await page.keyboard.press("/");
  await expect(page.getByRole("textbox", { name: "Search all mail" }).filter({ visible: true })).toBeFocused();
});

test("keyboard hints and shortcut opt-out persist, and the shortcut reference is accessible", async ({ page }) => {
  const writes = await fixture(page);
  await page.getByRole("button", { name: "Manage workspace" }).click();
  const hints = page.getByRole("checkbox", { name: "Show extra keyboard hints" });
  await expect(hints).not.toBeChecked();
  await hints.focus(); await page.keyboard.press("Space");
  await expect(hints).toBeChecked();
  await page.getByRole("button", { name: "Back to mail", exact: true }).click();
  await expect(page.locator(".keyboard-hints")).toContainText("Next / previous");
  await expect(page.getByRole("button", { name: "Archive", exact: true }).locator("kbd")).toHaveText("E");
  await page.locator("h1").click(); await page.keyboard.press("?");
  await expect(page.getByRole("dialog", { name: "Keyboard shortcuts" })).toContainText("Shift+R");
  await page.keyboard.press("Escape");
  await page.reload();
  await page.getByRole("button", { name: "Manage workspace" }).click();
  await expect(hints).toBeChecked();
  await page.getByRole("checkbox", { name: "Enable mail keyboard shortcuts" }).uncheck();
  await page.getByRole("button", { name: "Back to mail", exact: true }).click();
  await page.locator("h1").click();
  for (const key of ["j", "e", "Delete", "c"]) await page.keyboard.press(key);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(writes).toEqual([]);
  await expect(page.locator(".keyboard-hints")).not.toContainText("Next / previous");
  await page.reload();
  await page.getByRole("button", { name: "Manage workspace" }).click();
  await expect(page.getByRole("checkbox", { name: "Enable mail keyboard shortcuts" })).not.toBeChecked();
});


test("move and snooze open keyboard-operable dialogs, and spam uses the current message", async ({ page }) => {
  const writes = await fixture(page);
  await page.locator("h1").click();
  for (const [key, title] of [["m", "Move messages"], ["b", "Snooze until"]]) {
    await page.keyboard.press(key);
    const dialog = page.getByRole("dialog", { name: title, exact: true });
    await expect(dialog).toBeVisible();
    await page.keyboard.press("e");
    expect(writes).toEqual([]);
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
  }
  await page.keyboard.press("!");
  await expect.poll(() => writes.length).toBe(1);
  expect(writes[0]).toMatchObject({ target_id: mail.rows[0].id, change: { kind: "spam" } });
});

for (const width of [390, 1280, 1480]) test(`extra keyboard hints fit the reader at ${width}px`, async ({ page }, testInfo) => {
  await page.setViewportSize({ width, height: 980 });
  await page.addInitScript(() => localStorage.setItem("dakia:keyboard-fixture:keyboard-hints", "true"));
  await fixture(page);
  const toolbar = page.locator(".reader > .reader-toolbar");
  await expect(toolbar.locator("kbd")).toHaveCount(4);
  expect(await toolbar.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
  await page.screenshot({ path: testInfo.outputPath("keyboard-hints.png"), fullPage: true });
});

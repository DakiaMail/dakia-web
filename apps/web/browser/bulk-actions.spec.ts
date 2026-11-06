import { test, expect, type Page } from "@playwright/test";
import type { Change, Message, Operation } from "@dakia/api-types";
import { accounts, content, page as mail } from "./fixtures";

async function fixture(page: Page) {
  const rows: Message[] = accounts.map(account => ({ ...mail.rows[0], account_id: account.id, id: "same-id", subject: `Bulk ${account.id}`, unread: false, revision: 1 }));
  rows.push({ ...rows[0], id: "sibling", subject: "Unselected thread sibling" });
  const operations = new Map<string, Operation>();
  let posts = 0;
  const folderReads: string[] = [];
  await page.route("**/api/**", async route => {
    const path = new URL(route.request().url()).pathname;
    const respond = (value: unknown) => route.fulfill({ contentType: "application/json", body: JSON.stringify(value) });
    if (path === "/api/session") return respond({ user_id: "bulk-fixture", csrf_token: "synthetic" });
    if (path === "/api/accounts") return respond(accounts);
    if (path.endsWith("/events")) return route.fulfill({ contentType: "text/event-stream", body: ": ready\n\n" });
    if (path.endsWith("/operations")) {
      posts++;
      const op: Operation = { ...route.request().postDataJSON(), status: "pending", revision: 2 };
      expect(path).toBe(`/api/accounts/${op.account_id}/operations`);
      expect(op.target_id).toBe("same-id");
      operations.set(op.id, op); return respond(op);
    }
    if (path.includes("/operations/")) return respond(operations.get(path.split("/").at(-1)!));
    if (path.endsWith("/folders")) {
      const id = path.split("/")[3]; folderReads.push(id);
      return respond([`Destination ${id}`]);
    }
    if (path === "/api/search" || path.endsWith("/messages")) return respond({ ...mail, rows: rows.filter(row => row.folder_ids.includes("INBOX")), continuation: null });
    const message = rows.find(row => path.endsWith(`/messages/${row.id}`) && path.includes(`/accounts/${row.account_id}/`)) || rows.at(-1)!;
    if (path.endsWith("/conversation")) return respond({ rows: [message], complete: true, continuation: null });
    if (path.includes("/messages/")) return respond({ ...content, message });
    return respond([]);
  });
  await page.goto("/");
  await page.getByRole("tab", { name: "List", exact: true }).click();
  return {
    rows, operations, folderReads, posts: () => posts,
    settle(failAccount?: string) {
      for (const [id, op] of operations) {
        if (op.status !== "pending") continue;
        const failed = op.account_id === failAccount;
        operations.set(id, { ...op, status: failed ? "failed" : "applied", revision: 3 });
        if (failed) continue;
        const row = rows.find(row => row.id === op.target_id && row.account_id === op.account_id)!;
        const change = op.change;
        if (change.kind === "read") row.unread = !change.value;
        else if (change.kind === "star") row.starred = change.value;
        else if (change.kind === "category") row.category = change.value;
        else if (change.kind === "move") row.folder_ids = [change.folder_id];
        else if (["archive", "trash", "spam"].includes(change.kind)) row.folder_ids = [change.kind];
        row.revision = 3;
      }
    },
  };
}
async function select(page: Page) {
  for (const account of ["work", "personal"]) await page.getByRole("checkbox", { name: `Select Bulk ${account}`, exact: true }).check();
}
async function finished(page: Page) {
  await expect.poll(() => page.evaluate(() => JSON.parse(sessionStorage.getItem("dakia-pending-operations:bulk-fixture") || "[]").length)).toBe(0);
  await expect(page.getByText(/Applying|applied, .*not verified/)).toHaveCount(0);
}
async function action(page: Page, name: string) {
  const toolbar = page.locator(".reader-toolbar");
  if (!["Archive", "Mark unread", "Move to trash", "Snooze"].includes(name)) await toolbar.getByRole("button", { name: "More actions", exact: true }).click();
  await toolbar.getByRole("button", { name, exact: true }).click();
}
const cases: [string, Change][] = [
  ["Archive", { kind: "archive" }], ["Move to trash", { kind: "trash" }], ["Mark as spam", { kind: "spam" }],
  ["Mark read", { kind: "read", value: true }], ["Mark unread", { kind: "read", value: false }],
  ["Star selected messages", { kind: "star", value: true }], ["Unstar selected messages", { kind: "star", value: false }],
];
for (const width of [390, 1480]) {
  for (const [name, change] of cases) test(`bulk ${name} explicit account targets ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    const state = await fixture(page);
    await select(page);
    await action(page, name);
    await expect.poll(state.posts).toBe(2);
    for (const op of state.operations.values()) expect(op.change).toEqual(change);
    await expect(page.getByText(/Applying/)).toHaveCount(0);
    if (["archive", "trash", "spam"].includes(change.kind)) await expect(page.locator(".message-open").filter({ hasText: /Bulk / })).toHaveCount(0);
    // Refreshing the list while accepted work is pending never repeats POSTs.
    await page.locator(".header-refresh").click();
    expect(state.posts()).toBe(2);
    state.settle();
    const selected = state.rows.filter(row => row.id === "same-id");
    await finished(page);
    if (change.kind === "read") expect(selected.every(row => row.unread === !change.value)).toBeTruthy();
    if (change.kind === "star") expect(selected.every(row => row.starred === change.value)).toBeTruthy();
    expect(state.rows.at(-1)).toMatchObject({ id: "sibling", folder_ids: ["INBOX"], unread: false, starred: false, revision: 1 });
    await expect(page.getByRole("checkbox", { name: "Select Unselected thread sibling" })).not.toBeChecked();
    expect(state.posts()).toBe(2);
  });
  test(`bulk category targets checks not preview ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    const state = await fixture(page);
    await select(page);
    if (width > 900) await page.locator(".message-open").filter({ hasText: "Unselected thread sibling" }).click();
    await page.getByRole("button", { name: "More actions", exact: true }).click();
    await page.locator(".reader-toolbar").getByRole("combobox", { name: "Category" }).selectOption("notifications");
    await expect.poll(state.posts).toBe(2);
    state.settle();
    await finished(page);
    expect(state.rows.filter(row => row.id === "same-id").map(row => row.category)).toEqual(["notifications", "notifications"]);
    expect(state.rows.at(-1)!.category).toBe(mail.rows[0].category);
  });
  test(`account-specific move from checks only ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    const state = await fixture(page);
    await select(page); await action(page, "Move");
    const dialog = page.getByRole("dialog", { name: "Move messages" });
    await expect(dialog.getByRole("button", { name: "Move", exact: true })).toBeDisabled();
    await dialog.getByLabel("Folder for Work").selectOption("Destination work");
    await dialog.getByLabel("Folder for Personal").selectOption("Destination personal");
    expect([...new Set(state.folderReads)].sort()).toEqual(["personal", "work"]);
    await dialog.getByRole("button", { name: "Move", exact: true }).click();
    await expect.poll(state.posts).toBe(2);
    for (const op of state.operations.values()) expect(op.change).toEqual({ kind: "move", folder_id: `Destination ${op.account_id}` });
    state.settle();
    await finished(page);
  });
  test(`snooze snapshots checked targets ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    const state = await fixture(page);
    await select(page); await action(page, "Snooze");
    const dialog = page.getByRole("dialog", { name: "Snooze until" });
    await dialog.getByLabel("Return to inbox").fill("2099-01-01T10:00");
    await dialog.getByRole("button", { name: "Snooze", exact: true }).click();
    await expect.poll(state.posts).toBe(2);
    for (const op of state.operations.values()) expect(op.change.kind).toBe("snooze");
    state.settle();
    await finished(page);
  });
  test(`partial failure retains failed selection and restart does not replay ${width}`, async ({ page }) => {
    await page.setViewportSize({ width, height: 980 });
    const state = await fixture(page);
    await select(page); await action(page, "Move to trash");
    await expect.poll(state.posts).toBe(2);
    await page.reload();
    await expect(page.getByRole("checkbox", { name: "Select Unselected thread sibling" })).toBeVisible();
    await expect(page.getByRole("checkbox", { name: /Select Bulk / })).toHaveCount(0);
    expect(state.posts()).toBe(2);
    state.settle("work");
    await finished(page);
    await expect(page.getByRole("alert")).toContainText("Move to trash could not be verified");
    await expect(page.getByRole("checkbox", { name: "Select Bulk work", exact: true })).toBeVisible();
    expect(state.rows.find(row => row.account_id === "personal")!.folder_ids).toEqual(["trash"]);
    expect(state.rows.find(row => row.account_id === "work")!.folder_ids).toEqual(["INBOX"]);
    expect(state.posts()).toBe(2);
    expect(await page.evaluate(() => JSON.parse(sessionStorage.getItem("dakia-pending-operations:bulk-fixture") || "[]"))).toEqual([]);
  });
}

test("partial failure in same session restores only failed check", async ({ page }) => {
  const state = await fixture(page);
  await select(page); await action(page, "Archive");
  await expect.poll(state.posts).toBe(2);
  state.settle("work");
  await finished(page);
  await expect(page.getByRole("checkbox", { name: "Select Bulk work", exact: true })).toBeChecked();
  await expect(page.getByRole("button", { name: "1 selected · Clear" })).toBeVisible();
});

import { expect, test, vi } from "vitest";
import { collectMatchingMail } from "./collectMatchingMail";
import { page, messages, accounts } from "../browser/fixtures";
const first = messages[0];
const second = { ...first, id: "beyond-first-page" };
const cutoff = Date.parse("2026-11-13T12:00:00Z");
const ids = accounts.map(a => a.id);
const signal = () => new AbortController().signal;
test("collects every page and account, deduplicates and excludes later arrivals", async () => {
  const twin = { ...first, account_id: accounts[1].id };
  const fetch = vi.fn().mockResolvedValueOnce({ ...page, rows: [first, twin], continuation: "second", pending: true })
    .mockResolvedValueOnce({ ...page, rows: [first, second, { ...first, id: "new-arrival", received_at: "2026-11-13T12:01:00Z" }], continuation: null });
  const progress = vi.fn();
  expect(await collectMatchingMail(fetch, ids, signal(), progress, cutoff)).toEqual([first, twin, second]);
  expect(fetch.mock.calls.map(c => c[0])).toEqual([null, "second"]);
  expect(progress).toHaveBeenLastCalledWith(3);
});
test.each([
  { ...page, cache_preview: true },
  { ...page, pending: true },
  { ...page, coverage: [] },
  { ...page, coverage: [{ ...page.coverage[0], state: "partial" }] },
  { ...page, coverage: [{ ...page.coverage[0], state: "error" }] },
])("refuses incomplete coverage instead of returning a partial all-selection", async value => {
  await expect(collectMatchingMail(vi.fn().mockResolvedValue(value), ids, signal(), () => {}, cutoff)).rejects.toThrow();
});
test("rejects repeated cursors and unexpected accounts", async () => {
  const fetch = vi.fn().mockResolvedValue({ ...page, continuation: "loop" });
  await expect(collectMatchingMail(fetch, ids, signal(), () => {}, cutoff)).rejects.toThrow("repeated");
  expect(fetch).toHaveBeenCalledTimes(2);
  await expect(collectMatchingMail(vi.fn().mockResolvedValue({ ...page, rows: [{ ...first, account_id: "someone-else" }] }), ids, signal(), () => {}, cutoff)).rejects.toThrow("account");
});
test("cancels between pages without returning or selecting partial matches", async () => {
  const controller = new AbortController();
  const fetch = vi.fn().mockResolvedValue({ ...page, continuation: "more" });
  await expect(collectMatchingMail(fetch, ids, controller.signal, () => controller.abort(), cutoff)).rejects.toThrow();
  expect(fetch).toHaveBeenCalledTimes(1);
});

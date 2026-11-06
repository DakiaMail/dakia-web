import { afterEach, expect, it, vi } from "vitest";
import { api } from "./api";
import { findSenderMail } from "./SenderCleanup";
import type { Message, SearchPage } from "@dakia/api-types";
const message = { id: "one", account_id: "a", sender: { email: 'list+tag@example.org', name: null }, received_at: "2026-01-01T00:00:00Z" } as Message;
const page = (rows: Message[], continuation: string | null = null, state = "complete") => ({ rows, continuation, pending: !!continuation, coverage: [{ account_id: "a", state }] }) as SearchPage;
afterEach(() => vi.restoreAllMocks());
it("follows every continuation, deduplicates and rejects substring/other-account/future targets", async () => {
  const search = vi.spyOn(api, "search").mockResolvedValueOnce(page([message, { ...message, account_id: "b" }, { ...message, id: "substring", sender: { email: "xlist+tag@example.org", name: null } }], "next"))
    .mockResolvedValueOnce(page([message, { ...message, id: "two" }, { ...message, id: "future", received_at: "2999-01-01T00:00:00Z" }]));
  const abort = new AbortController();
  expect(await findSenderMail(message, abort.signal)).toEqual({ rows: [message, { ...message, id: "two" }], complete: true });
  expect(search.mock.calls).toEqual([[`from:"list+tag@example.org"`, ["a"], null, undefined, abort.signal, true], [`from:"list+tag@example.org"`, ["a"], "next", undefined, abort.signal, true]]);
});
it("retains found rows without claiming complete after later-page failure", async () => {
  vi.spyOn(api, "search").mockResolvedValueOnce(page([message], "next")).mockRejectedValueOnce(new Error("offline"));
  expect(await findSenderMail(message, new AbortController().signal)).toEqual({ rows: [message], complete: false });
});
it.each(["partial", "error", "pending"])("does not claim zero complete for %s coverage", async state => {
  vi.spyOn(api, "search").mockResolvedValue(page([], null, state));
  expect(await findSenderMail(message, new AbortController().signal)).toEqual({ rows: [], complete: false });
});
it("reports a genuinely empty complete search", async () => {
  vi.spyOn(api, "search").mockResolvedValue(page([]));
  expect(await findSenderMail(message, new AbortController().signal)).toEqual({ rows: [], complete: true });
});
it("stops repeated cursors and never repeats a request indefinitely", async () => {
  const search = vi.spyOn(api, "search").mockResolvedValue(page([message], "same"));
  expect((await findSenderMail(message, new AbortController().signal)).complete).toBe(false);
  expect(search).toHaveBeenCalledTimes(2);
});
it("escapes sender query syntax and does not widen account scope", async () => {
  const search = vi.spyOn(api, "search").mockResolvedValue(page([]));
  await findSenderMail({ ...message, sender: { email: 'a" folder:INBOX\\@example.org', name: null } }, new AbortController().signal);
  expect(search.mock.calls[0][0]).toBe('from:"a\\" folder:INBOX\\\\@example.org"');
});

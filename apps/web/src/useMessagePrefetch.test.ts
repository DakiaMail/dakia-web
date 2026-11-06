import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { MessagePrefetch } from "./useMessagePrefetch";
import { api } from "./api";
import { messageContent } from "./messageContent";
vi.mock("./api", () => ({ api: { contentById: vi.fn() } }));
let client: QueryClient;
let prefetch: MessagePrefetch;
const target = { account_id: "a", id: "one" };
beforeEach(() => {
  vi.useFakeTimers();
  Object.defineProperty(document, "visibilityState", { configurable: true, value: "visible" });
  Object.defineProperty(navigator, "onLine", { configurable: true, value: true });
  Object.defineProperty(navigator, "connection", { configurable: true, value: undefined });
  client = new QueryClient(); prefetch = new MessagePrefetch(client);
  vi.mocked(api.contentById).mockReset().mockResolvedValue({} as never);
});
afterEach(() => { prefetch.cancel(); client.clear(); vi.useRealTimers(); });
test("dwell replaces previous intent and reuses fresh account-scoped content", async () => {
  prefetch.request({ ...target, id: "discard" });
  prefetch.request(target);
  await vi.advanceTimersByTimeAsync(151);
  expect(api.contentById).toHaveBeenCalledTimes(1);
  prefetch.request(target); await vi.advanceTimersByTimeAsync(151);
  expect(api.contentById).toHaveBeenCalledTimes(1);
  prefetch.request({ ...target, account_id: "b" }); await vi.advanceTimersByTimeAsync(151);
  expect(api.contentById).toHaveBeenCalledTimes(2);
});
test.each(["offline", "hidden", "saveData", "2g"])("skips prefetch when %s", async mode => {
  if (mode === "offline") Object.defineProperty(navigator, "onLine", { value: false });
  if (mode === "hidden") Object.defineProperty(document, "visibilityState", { value: "hidden" });
  if (mode === "saveData" || mode === "2g") Object.defineProperty(navigator, "connection", { value: { saveData: mode === "saveData", effectiveType: mode === "2g" ? "2g" : "4g" } });
  prefetch.request(target); await vi.advanceTimersByTimeAsync(200);
  expect(api.contentById).not.toHaveBeenCalled();
});
test("serializes background reads and yields to foreground requests", async () => {
  vi.mocked(api.contentById).mockImplementation(() => new Promise(() => {}));
  prefetch.request(target); await vi.advanceTimersByTimeAsync(151);
  prefetch.request({ ...target, id: "two" }); await vi.advanceTimersByTimeAsync(300);
  expect(api.contentById).toHaveBeenCalledTimes(1);
  const signal = vi.mocked(api.contentById).mock.calls[0][2]!;
  prefetch.foreground({ ...target, id: "three" });
  expect(signal.aborted).toBe(true);
});
test("opening prefetched content shares the request and cancellation preserves the reader", async () => {
  vi.mocked(api.contentById).mockImplementation(() => new Promise(() => {}));
  prefetch.request(target); await vi.advanceTimersByTimeAsync(151);
  const observer = new QueryObserver(client, messageContent(target.account_id, target.id));
  const unsubscribe = observer.subscribe(() => {});
  prefetch.foreground(target); prefetch.cancel();
  expect(api.contentById).toHaveBeenCalledTimes(1);
  expect(vi.mocked(api.contentById).mock.calls[0][2]!.aborted).toBe(false);
  unsubscribe();
});
test("failed speculation never retries automatically", async () => {
  vi.mocked(api.contentById).mockRejectedValue(new Error("offline"));
  prefetch.request(target); await vi.advanceTimersByTimeAsync(151);
  prefetch.request(target); await vi.advanceTimersByTimeAsync(151);
  expect(api.contentById).toHaveBeenCalledTimes(1);
});
test("does not start while a foreground body is loading", async () => {
  vi.mocked(api.contentById).mockImplementation(() => new Promise(() => {}));
  void client.fetchQuery(messageContent("a", "foreground")).catch(() => {});
  prefetch.request(target); await vi.advanceTimersByTimeAsync(500);
  expect(api.contentById).toHaveBeenCalledTimes(1);
});

test("retains the focused body plus at most three unused speculative bodies", async () => {
  for (let i = 0; i < 8; i++) {
    prefetch.request({ ...target, id: String(i) });
    await vi.advanceTimersByTimeAsync(151);
  }
  expect(client.getQueryCache().findAll({ queryKey: ["content"] })).toHaveLength(4);
});
test("an explicit open retries a failed speculative read", async () => {
  vi.mocked(api.contentById).mockRejectedValueOnce(new Error("temporary"));
  prefetch.request(target); await vi.advanceTimersByTimeAsync(151);
  await client.fetchQuery(messageContent(target.account_id, target.id));
  expect(api.contentById).toHaveBeenCalledTimes(2);
});

test("warms every viewport body serially, then replaces the offscreen queue", async () => {
  prefetch.setVisible(Array.from({ length: 12 }, (_, i) => ({ ...target, id: String(i) })));
  await vi.advanceTimersByTimeAsync(200);
  expect(api.contentById).toHaveBeenCalledTimes(12);
  expect(client.getQueryCache().findAll({ queryKey: ["content"] })).toHaveLength(12);
  prefetch.setVisible([{ ...target, id: "new" }]); await vi.advanceTimersByTimeAsync(200);
  expect(api.contentById).toHaveBeenCalledTimes(13);
  expect(client.getQueryCache().findAll({ queryKey: ["content"] })).toHaveLength(4);
});
test("pause preserves viewport intent and resume restarts it without new geometry", async () => {
  prefetch.pause(); prefetch.setVisible([{ ...target, id: "visible-one" }, { ...target, id: "visible-two" }]);
  await vi.advanceTimersByTimeAsync(300);
  expect(api.contentById).not.toHaveBeenCalled();
  prefetch.resume(); await vi.advanceTimersByTimeAsync(200);
  expect(api.contentById).toHaveBeenCalledTimes(2);
});

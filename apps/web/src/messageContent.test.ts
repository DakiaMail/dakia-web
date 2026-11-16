import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import type { MessageContent } from "@dakia/api-types";
import { api } from "./api";
import { localMailCache } from "./localMailCache";
import { messageContent } from "./messageContent";

vi.mock("./api", () => ({ api: { contentById: vi.fn() } }));
vi.mock("./localMailCache", () => ({ localMailCache: { body: vi.fn(), verifiedBody: vi.fn() } }));
const fresh = { text: "Fresh content" } as MessageContent;
let client: QueryClient;
beforeEach(() => {
  vi.useFakeTimers(); vi.resetAllMocks();
  Object.defineProperty(navigator, "onLine", { configurable: true, value: true });
  client = new QueryClient();
  vi.mocked(localMailCache.body).mockResolvedValue(undefined);
  vi.mocked(api.contentById).mockResolvedValue(fresh);
});
afterEach(() => { client.clear(); vi.useRealTimers(); });

test("a stalled optional cache falls through to the network and ignores late content", async () => {
  let resolveCache!: (value: MessageContent) => void;
  vi.mocked(localMailCache.body).mockImplementation(() => new Promise(resolve => { resolveCache = resolve; }));
  const read = client.fetchQuery(messageContent("account", "message", client));
  await vi.advanceTimersByTimeAsync(249);
  expect(api.contentById).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1);
  await expect(read).resolves.toBe(fresh);
  expect(vi.mocked(localMailCache.body).mock.calls[0][2]!.aborted).toBe(true);
  resolveCache({ text: "Old content" } as MessageContent);
  await Promise.resolve();
  expect(client.getQueryData(["content", "account", "message"])).toBe(fresh);
  client.clear();
  expect(vi.getTimerCount()).toBe(0);
});

test("cancelling while local storage is stalled settles immediately without a network read", async () => {
  vi.mocked(localMailCache.body).mockImplementation(() => new Promise(() => {}));
  const read = client.fetchQuery(messageContent("account", "message", client));
  const rejected = expect(read).rejects.toMatchObject({ name: "Error" });
  await client.cancelQueries({ queryKey: ["content", "account", "message"] });
  await rejected;
  expect(api.contentById).not.toHaveBeenCalled();
  expect(vi.mocked(localMailCache.body).mock.calls[0][2]!.aborted).toBe(true);
});

test("an already cancelled read never starts cache or network work", async () => {
  const signal = AbortSignal.abort();
  const query = messageContent("account", "message");
  const read = query.queryFn as (context: { signal: AbortSignal }) => Promise<MessageContent>;
  await expect(read({ signal })).rejects.toMatchObject({ name: "AbortError" });
  expect(localMailCache.body).not.toHaveBeenCalled();
  expect(api.contentById).not.toHaveBeenCalled();
  expect(vi.getTimerCount()).toBe(0);
});

test("a failed optional cache does not prevent a healthy network read", async () => {
  vi.mocked(localMailCache.body).mockRejectedValue(new Error("Storage unavailable"));
  await expect(client.fetchQuery(messageContent("account", "message", client))).resolves.toBe(fresh);
  expect(api.contentById).toHaveBeenCalledTimes(1);
});

test("slow saved content remains readable offline without starting a network request", async () => {
  Object.defineProperty(navigator, "onLine", { configurable: true, value: false });
  const cached = { text: "Saved content" } as MessageContent;
  vi.mocked(localMailCache.body).mockImplementation(() => new Promise(resolve => setTimeout(() => resolve(cached), 1000)));
  const read = client.fetchQuery(messageContent("account", "message", client));
  await vi.advanceTimersByTimeAsync(250);
  expect(api.contentById).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(750);
  await expect(read).resolves.toBe(cached);
  expect(api.contentById).not.toHaveBeenCalled();
  client.clear();
  expect(vi.getTimerCount()).toBe(0);
});

test("a provider retry finishing beyond the old 30-second limit remains readable", async () => {
  vi.mocked(api.contentById).mockImplementation((_account, _id, signal) => new Promise((resolve, reject) => {
    setTimeout(() => resolve(fresh), 33000);
    signal!.addEventListener("abort", () => reject(signal!.reason), { once: true });
  }));
  const read = client.fetchQuery(messageContent("account", "message", client));
  await vi.advanceTimersByTimeAsync(30001);
  expect(vi.mocked(api.contentById).mock.calls[0][2]!.aborted).toBe(false);
  await vi.advanceTimersByTimeAsync(2999);
  await expect(read).resolves.toBe(fresh);
});

test("the complete read budget still terminates an unresponsive request and retains cached content", async () => {
  const cached = { text: "Saved content" } as MessageContent;
  vi.mocked(localMailCache.body).mockResolvedValue(cached);
  vi.mocked(api.contentById).mockImplementation(() => new Promise(() => {}));
  const read = client.fetchQuery(messageContent("account", "message", client));
  const rejected = expect(read).rejects.toThrow("This message couldn’t be loaded");
  await vi.advanceTimersByTimeAsync(65000);
  await rejected;
  expect(client.getQueryData(["content", "account", "message"])).toBe(cached);
  expect(vi.mocked(api.contentById).mock.calls[0][2]!.aborted).toBe(true);
});

import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { Change, Operation } from "@dakia/api-types";
import { api, ApiFailure, OperationQueue, request } from "./api";
import { messages } from "../browser/fixtures";

const actions: [string, Change][] = [
  ["archive", { kind: "archive" }], ["trash", { kind: "trash" }],
  ["spam", { kind: "spam" }], ["move", { kind: "move", folder_id: "Projects/2026" }],
  ["read", { kind: "read", value: true }], ["unread", { kind: "read", value: false }],
  ["star", { kind: "star", value: true }], ["unstar", { kind: "star", value: false }],
  ["category", { kind: "category", value: "people" }],
  ["snooze", { kind: "snooze", until: "2099-01-01T00:00:00Z" }],
];
beforeEach(() => { vi.useFakeTimers(); sessionStorage.clear(); });
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); vi.useRealTimers(); sessionStorage.clear(); });

it.each(actions)("%s waits for durable completion, journals pending work and sends only once", async (_, change) => {
  const queue = new OperationQueue(); queue.restore("fixture-user");
  const post = vi.spyOn(api, "operate").mockImplementation(async op => ({ ...op, status: "pending", revision: 2 }));
  const status = vi.spyOn(api, "operationStatus")
    .mockImplementationOnce(async op => ({ ...op, status: "pending", revision: 2 }))
    .mockImplementationOnce(async op => ({ ...op, status: "applied", revision: 3 }));
  const update = vi.fn(), invalidate = vi.fn();
  const result = queue.enqueue(messages[0], change, update, invalidate);
  await vi.advanceTimersByTimeAsync(0);
  expect(update).toHaveBeenCalledOnce();
  expect(post).toHaveBeenCalledOnce();
  expect(new OperationQueue().restore("fixture-user")).toHaveLength(1);
  expect(invalidate).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(2000); await result;
  expect(post).toHaveBeenCalledOnce(); expect(status).toHaveBeenCalledTimes(2);
  expect(invalidate).toHaveBeenCalledOnce();
  expect(new OperationQueue().restore("fixture-user")).toEqual([]);
});

it.each(actions)("%s retains uncertain outcome for recovery without a second write", async (_, change) => {
  const queue = new OperationQueue(); queue.restore("fixture-user");
  const post = vi.spyOn(api, "operate").mockRejectedValue(new TypeError("connection lost"));
  const invalidate = vi.fn();
  await expect(queue.enqueue(messages[0], change, vi.fn(), invalidate)).rejects.toThrow("could not be verified");
  const recovered = new OperationQueue(); const [op] = recovered.restore("fixture-user");
  expect(op.change).toEqual(change);
  vi.spyOn(api, "operationStatus").mockResolvedValue({ ...op, status: "applied", revision: 3 });
  await recovered.resume(op);
  expect(post).toHaveBeenCalledOnce();
  expect(new OperationQueue().restore("fixture-user")).toEqual([]);
  expect(invalidate).toHaveBeenCalledOnce();
});

it("same-message actions serialize through provider completion and use the resulting revision", async () => {
  const queue = new OperationQueue();
  const post = vi.spyOn(api, "operate").mockImplementation(async op => ({ ...op, status: "pending", revision: 2 }));
  vi.spyOn(api, "operationStatus").mockImplementation(async op => ({ ...op, status: "applied", revision: 10 }));
  const first = queue.enqueue(messages[0], { kind: "read", value: true }, vi.fn(), vi.fn());
  const second = queue.enqueue(messages[0], { kind: "star", value: true }, vi.fn(), vi.fn());
  await vi.advanceTimersByTimeAsync(0); expect(post).toHaveBeenCalledOnce();
  await vi.advanceTimersByTimeAsync(1000); await first;
  expect(post).toHaveBeenCalledTimes(2);
  expect(post.mock.calls[1][0].expected_revision).toBe(10);
  await vi.advanceTimersByTimeAsync(1000); await second;
});

it("a slow account does not block operations on another account with the same local message ID", async () => {
  const queue = new OperationQueue();
  let release!: (op: Operation) => void;
  let held!: Operation;
  const post = vi.spyOn(api, "operate").mockImplementation(op => op.account_id === "work"
    ? new Promise(resolve => { held = op; release = resolve; })
    : Promise.resolve({ ...op, status: "applied", revision: 2 }));
  const slow = queue.enqueue(messages[0], { kind: "archive" }, vi.fn(), vi.fn());
  await queue.enqueue({ ...messages[0], account_id: "personal" }, { kind: "spam" }, vi.fn(), vi.fn());
  expect(post).toHaveBeenCalledTimes(2);
  release({ ...held, status: "applied", revision: 2 }); await slow;
});

it.each([400, 401, 403, 404, 409, 422, 429])("a definitive HTTP %i rejection leaves no replayable operation", async code => {
  const queue = new OperationQueue(); queue.restore("fixture-user");
  vi.spyOn(api, "operate").mockRejectedValue(new ApiFailure(code, "fixture", "Rejected"));
  await expect(queue.enqueue(messages[0], { kind: "trash" }, vi.fn(), vi.fn())).rejects.toThrow("Rejected");
  expect(new OperationQueue().restore("fixture-user")).toEqual([]);
});

it("API requests use same-origin credentials, disable caching and forward cancellation", async () => {
  const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({ ok: true })));
  vi.stubGlobal("fetch", fetch);
  const controller = new AbortController();
  await request("/search", { signal: controller.signal });
  expect(fetch.mock.calls[0][1]).toMatchObject({ credentials: "same-origin", cache: "no-store", signal: controller.signal });
});
it("a non-JSON upstream error produces a typed error rather than hiding its HTTP status", async () => {
  vi.stubGlobal("fetch", vi.fn().mockResolvedValue(new Response("gateway failed", { status: 502 })));
  await expect(request("/search")).rejects.toMatchObject({ status: 502, code: "http_error" });
});
it("download links encode opaque account and attachment IDs independently", () => {
  expect(api.download("a/b", "part?x#y")).toBe("/api/accounts/a%2Fb/attachments/part%3Fx%23y");
});

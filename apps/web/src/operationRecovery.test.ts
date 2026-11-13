import { afterEach, expect, it, vi } from "vitest";
import { ApiFailure, api, OperationQueue } from "./api";
import type { Message, Operation } from "@dakia/api-types";
const message: Message = {
  id: "same", account_id: "one", thread_id: "thread", provider_id: "opaque", folder_ids: ["INBOX"],
  sender: { name: null, email: "fixture@example.invalid" }, to: [], subject: "Private subject not stored",
  preview: "Private preview not stored", received_at: "2026-10-28T00:00:00Z", unread: true, starred: false,
  category: null, category_status: "pending", revision: 4,
};
const op: Operation = { id: "pending", account_id: "one", target_id: "same", expected_revision: 4, change: { kind: "trash" }, status: "pending", revision: 0 };
afterEach(() => { vi.restoreAllMocks(); sessionStorage.clear(); });
it("retains uncertain IDs across queue restart, reads exact outcome without a second POST", async () => {
  const queue = new OperationQueue(); queue.restore("user");
  const post = vi.spyOn(api, "operate").mockRejectedValue(new TypeError("lost response"));
  await expect(queue.enqueue(message, { kind: "trash" }, () => {}, () => {})).rejects.toThrow("Refresh mail and check its current state before trying again");
  const raw = sessionStorage.getItem("dakia-pending-operations:user")!;
  expect(raw).not.toMatch(/Private subject|Private preview|fixture@example/);
  const restored = new OperationQueue(); const pending = restored.restore("user");
  expect(pending).toHaveLength(1);
  const get = vi.spyOn(api, "operationStatus").mockImplementation(async value => ({ ...value, status: "applied", revision: 6 }));
  await restored.resume(pending[0]);
  expect(post).toHaveBeenCalledTimes(1); expect(get).toHaveBeenCalledWith(pending[0]);
  expect(new OperationQueue().restore("user")).toEqual([]);
});
it("an offline outcome read retains the accepted ID and never automatically retries the mutation", async () => {
  const queue = new OperationQueue(); queue.restore("user");
  const post = vi.spyOn(api, "operate").mockImplementation(async value => ({ ...value, status: "pending", revision: 5 }));
  vi.spyOn(api, "operationStatus").mockRejectedValue(new TypeError("offline"));
  await expect(queue.enqueue(message, { kind: "trash" }, () => {}, () => {})).rejects.toThrow("Refresh mail and check its current state");
  const restored = new OperationQueue(), pending = restored.restore("user");
  expect(pending).toHaveLength(1);
  vi.spyOn(api, "operationStatus").mockResolvedValue({ ...pending[0], status: "applied", revision: 6 });
  await restored.resume(pending[0]);
  expect(post).toHaveBeenCalledTimes(1);
  expect(new OperationQueue().restore("user")).toEqual([]);
});
it("never leaks another user's pending targets", () => {
  sessionStorage.setItem("dakia-pending-operations:first", JSON.stringify([op]));
  expect(new OperationQueue().restore("second")).toEqual([]);
  expect(new OperationQueue().restore("first")).toEqual([op]);
});
it("removes a definitive rejected request rather than reporting it pending after reload", async () => {
  const queue = new OperationQueue(); queue.restore("user");
  vi.spyOn(api, "operate").mockRejectedValue(new ApiFailure(409, "conflict", "Refresh first"));
  await expect(queue.enqueue(message, { kind: "trash" }, () => {}, () => {})).rejects.toThrow("Refresh first");
  expect(new OperationQueue().restore("user")).toEqual([]);
});
it("reports interrupted durable failures and does not replay them", async () => {
  sessionStorage.setItem("dakia-pending-operations:user", JSON.stringify([op]));
  const queue = new OperationQueue(); queue.restore("user");
  const post = vi.spyOn(api, "operate");
  vi.spyOn(api, "operationStatus").mockResolvedValue({ ...op, status: "failed", revision: 6 });
  await expect(queue.resume(op)).rejects.toThrow("Move to trash could not be verified");
  expect(post).not.toHaveBeenCalled();
  expect(new OperationQueue().restore("user")).toEqual([]);
});
it("a lost pre-acceptance request is read back as missing, never submitted again", async () => {
  const queue = new OperationQueue(); queue.restore("user");
  const post = vi.spyOn(api, "operate");
  vi.spyOn(api, "operationStatus").mockRejectedValue(new ApiFailure(404, "not_found", "Operation not found"));
  await expect(queue.resume(op)).rejects.toThrow("Operation not found");
  expect(post).not.toHaveBeenCalled();
});
it("corrupt or unavailable session storage does not prevent actual operations", async () => {
  sessionStorage.setItem("dakia-pending-operations:user", "not json");
  const queue = new OperationQueue(); expect(queue.restore("user")).toEqual([]);
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw new Error("denied"); });
  vi.spyOn(api, "operate").mockImplementation(async value => ({ ...value, status: "applied", revision: 5 }));
  await queue.enqueue(message, { kind: "read", value: true }, () => {}, () => {});
});
it("journals all bulk intents while limiting concurrent mutation submissions", async () => {
  const queue = new OperationQueue(); queue.restore("bulk-user");
  let active = 0, maximum = 0;
  vi.spyOn(api, "operate").mockImplementation(async value => {
    active++; maximum = Math.max(maximum, active);
    await new Promise(resolve => setTimeout(resolve, 5));
    active--; return { ...value, status: "applied", revision: 5 };
  });
  await Promise.all(Array.from({ length: 30 }, (_, i) => queue.enqueue({ ...message, id: `bulk-${i}` }, { kind: "archive" }, () => {}, () => {})));
  expect(maximum).toBe(6);
  expect(new OperationQueue().restore("bulk-user")).toEqual([]);
});

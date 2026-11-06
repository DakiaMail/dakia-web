import { afterEach, describe, expect, it, vi } from "vitest";
import type { Message, Operation } from "@dakia/api-types";
import { api, OperationQueue } from "./api";

const message = { id: "same", account_id: "one", revision: 1, folder_ids: ["INBOX", "Important"] } as Message;
afterEach(() => { vi.restoreAllMocks(); vi.useRealTimers(); });
describe("durable archive acknowledgement", () => {
  it("keeps the optimistic archive through pending polls and serializes the next intent until applied", async () => {
    vi.useFakeTimers();
    const posted: Operation[] = [];
    vi.spyOn(api, "operate").mockImplementation(async op => {
      posted.push(op);
      return { ...op, status: "pending", revision: 2 };
    });
    let applied = false;
    vi.spyOn(api, "operationStatus").mockImplementation(async op => ({ ...op, status: applied ? "applied" : "pending", revision: applied ? 7 : 2 }));
    const invalidate = vi.fn(), update = vi.fn(), queue = new OperationQueue();
    const first = queue.enqueue(message, { kind: "archive" }, update, invalidate);
    const next = queue.enqueue({ ...message, folder_ids: ["Important"] }, { kind: "star", value: true }, update, invalidate);
    await vi.advanceTimersByTimeAsync(3000);
    expect(posted).toHaveLength(1);
    expect(invalidate).not.toHaveBeenCalled();
    expect(update.mock.calls.at(-1)?.[1].folder_ids).toEqual(["Important"]);
    applied = true;
    await vi.advanceTimersByTimeAsync(2000);
    await Promise.all([first, next]);
    expect(posted[1].expected_revision).toBe(7);
    expect(invalidate).toHaveBeenCalledTimes(1);
  });
  it("a provider failure after acceptance reports failure and permits rollback", async () => {
    vi.useFakeTimers();
    vi.spyOn(api, "operate").mockImplementation(async op => ({ ...op, status: "pending", revision: 2 }));
    vi.spyOn(api, "operationStatus").mockImplementation(async op => ({ ...op, status: "failed", revision: 3 }));
    const invalidate = vi.fn(), queue = new OperationQueue();
    const result = queue.enqueue(message, { kind: "archive" }, vi.fn(), invalidate).catch(e => e);
    await vi.advanceTimersByTimeAsync(1000);
    expect((await result).message).toBe("Archive could not be verified for “(no subject)”. Refresh mail and check its current state before trying again.");
    expect(invalidate).toHaveBeenCalledTimes(1);
  });
  it("concurrent archives with identical local IDs poll their own account and settle independently", async () => {
    vi.useFakeTimers();
    vi.spyOn(api, "operate").mockImplementation(async op => ({ ...op, status: "pending", revision: 2 }));
    const polls = vi.spyOn(api, "operationStatus").mockImplementation(async op => ({ ...op, status: "applied", revision: 3 }));
    const one = vi.fn(), two = vi.fn(), queue = new OperationQueue();
    const results = [queue.enqueue(message, { kind: "archive" }, vi.fn(), one), queue.enqueue({ ...message, account_id: "two" }, { kind: "archive" }, vi.fn(), two)];
    await vi.advanceTimersByTimeAsync(1000);
    await Promise.all(results);
    expect(polls.mock.calls.map(([op]) => op.account_id).sort()).toEqual(["one", "two"]);
    expect(one).toHaveBeenCalledTimes(1); expect(two).toHaveBeenCalledTimes(1);
  });
});

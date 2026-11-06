import { afterEach, expect, it, vi } from "vitest";
import type { Change, Message } from "@dakia/api-types";
import { actionLabel, api, OperationQueue } from "./api";

afterEach(() => vi.restoreAllMocks());
it.each<[Change, string]>([
  [{ kind: "read", value: true }, "Mark as read"],
  [{ kind: "read", value: false }, "Mark as unread"],
  [{ kind: "star", value: true }, "Star"],
  [{ kind: "star", value: false }, "Unstar"],
  [{ kind: "archive" }, "Archive"],
  [{ kind: "trash" }, "Move to trash"],
  [{ kind: "spam" }, "Mark as spam"],
  [{ kind: "move", folder_id: "private-folder" }, "Move"],
  [{ kind: "category", value: "people" }, "Change category"],
  [{ kind: "snooze", until: "2026-11-01T12:00:00Z" }, "Snooze"],
])("identifies %j without claiming the provider did not apply it", async (change, label) => {
  const post = vi.spyOn(api, "operate").mockImplementation(async op => ({ ...op, status: "failed", revision: 4 }));
  const poll = vi.spyOn(api, "operationStatus");
  const message = { id: "same", account_id: "one", subject: "Synthetic newsletter", revision: 2, folder_ids: ["INBOX"] } as Message;
  const invalidate = vi.fn();
  await expect(new OperationQueue().enqueue(message, change, vi.fn(), invalidate)).rejects.toThrow(`${label} could not be verified for “Synthetic newsletter”.`);
  expect(actionLabel(change)).toBe(label);
  expect(post).toHaveBeenCalledTimes(1);
  expect(poll).not.toHaveBeenCalled();
  expect(invalidate).toHaveBeenCalledTimes(1);
});

import { describe, expect, it } from "vitest";
import { cleanupProjection, type CleanupIntent, type CleanupJob } from "./cleanupJobs";
import { messages } from "../browser/fixtures";
import { eventAffects } from "./mailEventPolicy";
import type { Query } from "@tanstack/react-query";
const source = messages[0];
const intent: CleanupIntent = { account_id: source.account_id, sender: source.sender.email, rows: [source], request: { id: "frozen", source_id: source.id, search_complete: true, targets: [{ id: source.id, expected_revision: source.revision }] } };
const job: CleanupJob = { account_id: source.account_id, sender: source.sender.email, request: intent.request, created_at: "fixture", dismissed: false, pending: 1, applied: 0, failed: 0, failed_targets: [], projected_rows: [source] };
describe("silent cleanup projection", () => {
  it("projects Trash without changing original/provider metadata or claiming completion", () => {
    const rows = cleanupProjection([intent], []);
    expect(rows[`${source.account_id}/${source.id}`].folder_ids).toEqual(["Trash"]);
    expect(source.folder_ids).toEqual(["INBOX"]);
    expect(rows[`${source.account_id}/${source.id}`].revision).toBe(source.revision);
  });
  it("restores rejected/failed targets, retaining the internal request", () => {
    expect(cleanupProjection([{ ...intent, rejected: true }], [])).toEqual({});
    expect(cleanupProjection([intent], [{ ...job, projected_rows: [], pending: 0, failed: 1, failed_targets: [source.id] }])).toEqual({});
    expect(intent.request.targets).toHaveLength(1);
  });
  it("hydrates existing pending jobs without a local journal", () => {
    expect(Object.keys(cleanupProjection([], [job]))).toEqual([`${source.account_id}/${source.id}`]);
  });
  it("never expands a frozen account, sender or target scope", () => {
    const rows = [source, { ...source, account_id: "foreign" }, { ...source, id: "late-arrival" }, { ...source, sender: { email: "substring@example.invalid", name: null } }];
    expect(Object.keys(cleanupProjection([{ ...intent, rows }], [{ ...job, projected_rows: rows }]))).toEqual([`${source.account_id}/${source.id}`]);
  });
  it("reconciles from existing operation events, not unrelated account events", () => {
    const query = { queryKey: ["sender-cleanups", "user", source.account_id], state: { data: [job] } } as unknown as Query;
    expect(eventAffects(query, source.account_id, new Set(["operation_changed"]), new Set())).toBe(true);
    expect(eventAffects(query, "foreign", new Set(["operation_changed"]), new Set())).toBe(false);
    expect(eventAffects(query, source.account_id, new Set(["message_changed"]), new Set())).toBe(false);
  });
});

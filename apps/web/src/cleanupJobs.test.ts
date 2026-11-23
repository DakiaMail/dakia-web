import { afterEach, describe, expect, it, vi } from "vitest";
import { QueryClient, QueryClientProvider, type Query } from "@tanstack/react-query";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { cleanupProjection, loadCleanupCandidates, useSenderCleanups, type CleanupIntent, type CleanupJob } from "./cleanupJobs";
import { messages } from "../browser/fixtures";
import { eventAffects } from "./mailEventPolicy";
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
  it("leaves restored mail visible when old completed jobs still contain projected rows", () => {
    expect(cleanupProjection([], [{ ...job, pending: 0, applied: 1 }])).toEqual({});
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

afterEach(() => {
  cleanup();
  sessionStorage.clear();
  vi.restoreAllMocks();
});

it("loads exact local candidates across pages and stops a repeating cursor", async () => {
  const newRow = { ...source, id: "old-match", sender: { ...source.sender, email: source.sender.email.toUpperCase() } };
  const cutoff = source.received_at;
  const fetchPage = vi.fn().mockResolvedValueOnce({ rows: [source, { ...source, id: "foreign", account_id: "different" }], cutoff, count: 3, after: "next" })
    .mockResolvedValueOnce({ rows: [source, newRow, { ...source, id: "new", received_at: "2999-01-01T00:00:00Z" }], cutoff, count: 3, after: "next" });
  expect(await loadCleanupCandidates(source.account_id, source.sender.email, Date.parse(cutoff), fetchPage))
    .toEqual({ rows: [source, newRow], limited: true });
  expect(fetchPage.mock.calls).toEqual([[], ["next"]]);
});

it("budgets concurrent cleanup requests together and blocks duplicate targets", async () => {
  const client = new QueryClient();
  client.setQueryData(["session"], { user_id: "cleanup-budget" });
  const resolve: (() => void)[] = [];
  const writes: RequestInit[] = [];
  const fetch = vi.spyOn(globalThis, "fetch").mockImplementation(async (_, init) => {
    if (init?.method !== "POST") return Response.json([]);
    writes.push(init);
    await new Promise<void>(done => resolve.push(done));
    return Response.json({ code: "revision_conflict" }, { status: 409 });
  });
  const { result } = renderHook(() => useSenderCleanups(["work", "personal"]), {
    wrapper: ({ children }: { children: ReactNode }) => createElement(QueryClientProvider, { client, children }),
  });
  const batch = (account_id: string) => Array.from({ length: 180 }, (_, index) => ({
    ...source, account_id, id: `email-${index}`, provider_id: "provider-id".repeat(14),
  }));
  const work = batch("work"), personal = batch("personal");
  act(() => {
    const [workCleanup, personalCleanup] = result.current;
    expect(workCleanup.start(work[0], work, false)).toBeDefined();
    expect(workCleanup.start(work[0], work, false)).toBeUndefined();
    expect(personalCleanup.start(personal[0], personal, false)).toBeDefined();
  });
  expect(writes.map(write => write.keepalive)).toEqual([true, false]);
  await act(async () => { resolve.forEach(done => done()); });
  await waitFor(() => expect(result.current.every(entry => entry.submissions[0]?.rejected)).toBe(true));
  expect(fetch.mock.calls.filter(([, options]) => options?.method === "POST")).toHaveLength(2);
  client.clear();
});

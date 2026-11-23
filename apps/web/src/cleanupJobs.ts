import { useEffect, useMemo } from "react";
import { useQueries, useQuery, useQueryClient, type QueryClient } from "@tanstack/react-query";
import type { Account, Message, Session } from "@dakia/api-types";
import { accountPath, ApiFailure, optimistic, request, type SenderMailPage } from "./api";
import { senderAddress } from "./mailRules";

export type CleanupRequest = { id: string; source_id: string; search_complete: boolean; targets: { id: string; expected_revision: number; expected_location?: { provider_id: string; folder_ids: string[]; sender_email: string } }[] };
export type CleanupJob = { request: CleanupRequest; account_id: string; sender: string; created_at: string; dismissed: boolean; applied: number; failed: number; pending: number; failed_targets: string[]; projected_rows?: Message[] };
export type CleanupIntent = { request: CleanupRequest; account_id: string; sender: string; rows?: Message[]; rejected?: boolean; error?: string };
const identity = (m: Message) => `${m.account_id}/${m.id}`;
export const cleanupLimit = 10000;

export async function loadCleanupCandidates(
  account: string, sender: string, cutoff: number,
  fetchPage: (after?: string) => Promise<SenderMailPage>, first?: SenderMailPage,
) {
  const rows = new Map<string, Message>();
  const seen = new Set<string>();
  let page = first ?? await fetchPage();
  for (;;) {
    for (const row of page.rows) {
      if (row.account_id === account && senderAddress(row.sender.email) === senderAddress(sender)
        && Date.parse(row.received_at) <= cutoff) rows.set(row.id, row);
    }
    if (!page.after || rows.size >= cleanupLimit || seen.has(page.after) || seen.size >= 1000) {
      return { rows: [...rows.values()].slice(0, cleanupLimit), limited: !!page.after || rows.size > cleanupLimit };
    }
    seen.add(page.after);
    page = await fetchPage(page.after);
  }
}
const journalKey = (user: string) => ["sender-cleanup-intents", user];
let cleanupKeepaliveBytes = 0;
function save(client: QueryClient, user: string, rows: CleanupIntent[]) {
  // Local recovery ledger, not proof of provider success. No bodies/credentials.
  try { sessionStorage.setItem(`dakia-cleanup-intents:${user}`, JSON.stringify(rows)); } catch { /* acceptance is durable on the server */ }
  client.setQueryData(journalKey(user), rows);
}
function useJournal() {
  const client = useQueryClient();
  const user = client.getQueryData<Session>(["session"])?.user_id || "";
  const journal = useQuery<CleanupIntent[]>({ queryKey: journalKey(user), enabled: false, queryFn: async () => [], initialData: () => {
    try {
      const rows: CleanupIntent[] = JSON.parse(sessionStorage.getItem(`dakia-cleanup-intents:${user}`) || "[]");
      return Array.isArray(rows) ? rows.filter(row => row && typeof row.account_id === "string" && typeof row.request?.id === "string" && Array.isArray(row.request.targets)) : [];
    } catch { return []; }
  } });
  return { client, user, intents: journal.data || [] };
}
function jobOptions(user: string, account: string) {
  return { queryKey: ["sender-cleanups", user, account], queryFn: () => request<CleanupJob[]>(`${accountPath(account)}/sender-cleanups`),
    enabled: !!user, staleTime: 30000, refetchInterval: false as const, retry: false };
}
function invalidateMail(client: QueryClient) {
  return client.invalidateQueries({ predicate: query => ["mail", "search-page", "conversation-snapshot", "content", "sender-mail"].includes(String(query.queryKey[0])) }, { throwOnError: true });
}
export function cleanupProjection(intents: CleanupIntent[], jobs: CleanupJob[]) {
  const rows: Record<string, Message> = {};
  for (const job of jobs) {
    if (!job.pending) continue;
    const targets = new Set(job.request.targets.map(t => t.id));
    for (const row of job.projected_rows || []) {
      if (row.account_id === job.account_id && senderAddress(row.sender.email) === senderAddress(job.sender) && targets.has(row.id)) rows[identity(row)] = optimistic(row, { kind: "trash" });
    }
  }
  for (const intent of intents) {
    if (intent.rejected) continue;
    const job = jobs.find(job => job.account_id === intent.account_id && job.request.id === intent.request.id);
    const targets = new Set(intent.request.targets.map(t => t.id));
    for (const row of intent.rows || []) {
      if (row.account_id !== intent.account_id || senderAddress(row.sender.email) !== senderAddress(intent.sender) || !targets.has(row.id) || job?.failed_targets.includes(row.id)) continue;
      rows[identity(row)] = optimistic(row, { kind: "trash" });
    }
  }
  return rows;
}
// Mounted by the ordinary mail view. Existing account events reconcile outcomes;
// there is no cleanup timer, notification, or provider polling loop.
export function useCleanupProjection(accounts: Account[]) {
  const { client, user, intents } = useJournal();
  const queries = useQueries({ queries: accounts.map(account => jobOptions(user, account.id)) });
  const jobs = queries.flatMap(query => query.data || []);
  const signature = JSON.stringify(jobs.map(job => [job.account_id, job.request.id, job.pending, job.failed_targets]));
  useEffect(() => {
    // Keep the intent until all affected caches are stale/refreshed. A stale
    // in-flight GET must not resurrect rows when the final operation completes.
    const terminal = new Set(jobs.filter(job => !job.pending).map(job => `${job.account_id}/${job.request.id}`));
    if (!intents.some(row => terminal.has(`${row.account_id}/${row.request.id}`))) return;
    let live = true;
    void invalidateMail(client).then(() => {
      if (!live) return;
      const current = client.getQueryData<CleanupIntent[]>(journalKey(user)) || [];
      save(client, user, current.filter(row => !terminal.has(`${row.account_id}/${row.request.id}`)));
    }).catch(() => { /* keep the projection until an ordinary successful refresh */ });
    return () => { live = false; };
  }, [signature, intents, client, user]);
  return useMemo(() => cleanupProjection(intents.filter(row => accounts.some(a => a.id === row.account_id)), jobs), [intents, signature, accounts]);
}
export function useSenderCleanup(account: string) {
  return useSenderCleanups([account])[0];
}

export function useSenderCleanups(accounts: string[]) {
  const { client, user, intents } = useJournal();
  const queries = useQueries({ queries: accounts.map(account => jobOptions(user, account)) });
  return accounts.map((account, index) => {
    const jobs = queries[index];
    const submissions = intents.filter(row => row.account_id === account);
    const unavailable = new Set<string>();
    for (const entry of [...submissions.filter(intent => !intent.rejected), ...(jobs.data || []).filter(job => job.pending > 0)]) {
      for (const target of entry.request.targets) unavailable.add(target.id);
    }
    return {
      jobs, submissions, unavailable,
      start: (source: Message, rows: Message[], complete: boolean) => {
        if (!user || source.account_id !== account || !rows.length || rows.some(row => row.account_id !== account || unavailable.has(row.id) || senderAddress(row.sender.email) !== senderAddress(source.sender.email))) return;
        const current = () => client.getQueryData<CleanupIntent[]>(journalKey(user)) || [];
        const ids = new Set(rows.map(row => row.id));
        if (current().some(intent => intent.account_id === account && !intent.rejected && intent.request.targets.some(target => ids.has(target.id)))) return;
        const intent: CleanupIntent = { account_id: account, sender: source.sender.email, rows, request: {
          id: crypto.randomUUID(), source_id: source.id, search_complete: complete,
          targets: rows.map(row => ({ id: row.id, expected_revision: row.revision,
            expected_location: { provider_id: row.provider_id, folder_ids: [...row.folder_ids], sender_email: row.sender.email } })) } };
        save(client, user, [...current(), intent]);
        const body = JSON.stringify(intent.request);
        // All concurrent cleanup POSTs share the browser's keepalive budget.
        const bytes = new TextEncoder().encode(body).length;
        const keepalive = cleanupKeepaliveBytes + bytes < 60000;
        if (keepalive) cleanupKeepaliveBytes += bytes;
        void request<CleanupJob>(`${accountPath(account)}/sender-cleanups`, { method: "POST", body, keepalive }).then(job => {
          client.setQueryData<CleanupJob[]>(jobOptions(user, account).queryKey, old => [...(old || []).filter(row => row.request.id !== job.request.id), job]);
        }).catch(error => {
          // Preserve the exact request for internal reconciliation, never retry an
          // uncertain provider action or present optimistic state as completion.
          const rejected = error instanceof ApiFailure && error.status >= 400 && error.status < 500;
          save(client, user, current().map(row => row.request.id === intent.request.id ? { ...row, rejected, error: error instanceof ApiFailure ? error.code : "acceptance_unknown" } : row));
          if (rejected) void invalidateMail(client).catch(() => {});
        }).finally(() => {
          if (keepalive) cleanupKeepaliveBytes -= bytes;
          void client.invalidateQueries({ queryKey: jobOptions(user, account).queryKey });
        });
        return intent.request.id;
      },
    };
  });
}

import { useQueries } from "@tanstack/react-query";
import type { Message } from "@dakia/api-types";
import { api } from "./api";

export function threadCountBatches(rows: Message[]) {
  const accounts = new Map<string, Set<string>>();
  for (const row of rows) {
    if (!accounts.has(row.account_id)) accounts.set(row.account_id, new Set());
    accounts.get(row.account_id)!.add(row.id);
  }
  return [...accounts].flatMap(([account, values]) => {
    const ids = [...values];
    return Array.from({ length: Math.ceil(ids.length / 50) }, (_, i) => ({ account, ids: ids.slice(i * 50, (i + 1) * 50) }));
  });
}

export function useThreadCounts(rows: Message[]) {
  const batches = threadCountBatches(rows);
  const queries = useQueries({ queries: batches.map(({ account, ids }) => ({
    queryKey: ["thread-counts", account, ids],
    queryFn: ({ signal }: { signal: AbortSignal }) => api.threadCounts(account, ids, signal),
    staleTime: 30000,
    retry: false,
  })) });
  const counts: Record<string, number> = {};
  queries.forEach((query, index) => {
    for (const [id, count] of Object.entries(query.data || {})) {
      if (Number.isSafeInteger(count) && count > 1) counts[`${batches[index].account}/${id}`] = count;
    }
  });
  return counts;
}

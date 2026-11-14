import type { Message, SearchPage } from "@dakia/api-types";
import { identity } from "./mailRows";

// Freeze identities before any action; later arrivals never join the selection.
export async function collectMatchingMail(
  fetchPage: (continuation: string | null, signal: AbortSignal) => Promise<SearchPage>,
  accounts: readonly string[], signal: AbortSignal, progress: (count: number) => void,
  cutoff = Date.now(),
): Promise<Message[]> {
  const found = new Map<string, Message>(), cursors = new Set<string>();
  let continuation: string | null = null;
  do {
    signal.throwIfAborted();
    const page = await fetchPage(continuation, signal);
    signal.throwIfAborted();
    if (page.cache_preview || page.coverage.some(c => c.state === "error" || c.state === "partial"))
      throw new Error("Couldn’t check every match. Refresh mail and try again.");
    for (const row of page.rows) {
      if (!accounts.includes(row.account_id)) throw new Error("The selection returned an unexpected account.");
      if (!Number.isFinite(Date.parse(row.received_at))) throw new Error("Couldn’t verify the selection.");
      if (Date.parse(row.received_at) <= cutoff) found.set(identity(row), row);
    }
    progress(found.size);
    continuation = page.continuation;
    if (continuation) {
      if (cursors.has(continuation)) throw new Error("The mailbox repeated a page. Refresh mail and try again.");
      cursors.add(continuation);
    } else if (page.pending || accounts.some(id => !page.coverage.some(c => c.account_id === id && c.state === "complete"))) {
      throw new Error("The mailbox is still being checked. Refresh mail and try again.");
    }
  } while (continuation);
  return [...found.values()];
}

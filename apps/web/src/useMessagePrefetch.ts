import { useEffect, useMemo } from "react";
import { QueryClient, useQueryClient } from "@tanstack/react-query";
import type { Message } from "@dakia/api-types";
import { messageContent } from "./messageContent";

type Target = Pick<Message, "account_id" | "id">;
const same = (a: Target | undefined, b: Target) => a?.account_id === b.account_id && a.id === b.id;
function allowed() {
  const connection = (navigator as Navigator & { connection?: { saveData?: boolean; effectiveType?: string } }).connection;
  return navigator.onLine && document.visibilityState === "visible" && !connection?.saveData && !["slow-2g", "2g"].includes(connection?.effectiveType || "");
}

// One speculative request and one replaceable intent. No reader/image mounts,
// conversation discovery, retries, or mailbox mutations happen here.
export class MessagePrefetch {
  private timer?: ReturnType<typeof setTimeout>;
  private running?: Target;
  constructor(private client: QueryClient) {}
  request(target: Target) {
    clearTimeout(this.timer);
    this.timer = setTimeout(() => this.start(target), 150);
  }
  private start(target: Target) {
    if (!allowed()) return;
    if (this.running) {
      if (!same(this.running, target)) this.timer = setTimeout(() => this.start(target), 100);
      return;
    }
    // Never start background work while a foreground body is being fetched.
    if (this.client.isFetching({ queryKey: ["content"] })) {
      this.timer = setTimeout(() => this.start(target), 100);
      return;
    }
    const options = messageContent(target.account_id, target.id);
    const state = this.client.getQueryState(options.queryKey);
    // A speculative failure waits for an explicit open, never retries on hover.
    if (state?.status === "error" || (state?.dataUpdatedAt && Date.now() - state.dataUpdatedAt < 30000)) return;
    this.running = target;
    void this.client.prefetchQuery({ ...options, gcTime: 60000, meta: { speculative: true } }).finally(() => {
      this.running = undefined;
      const unused = this.client.getQueryCache().findAll({ queryKey: ["content"] })
        .filter(query => query.meta?.speculative && query.getObserversCount() === 0 && query.state.fetchStatus === "idle")
        .sort((a, b) => b.state.dataUpdatedAt - a.state.dataUpdatedAt);
      for (const query of unused.slice(3)) this.client.removeQueries({ queryKey: query.queryKey, exact: true });
    });
  }
  foreground(target: Target) {
    clearTimeout(this.timer);
    if (this.running && !same(this.running, target)) this.cancelInactive();
  }
  private cancelInactive() {
    if (!this.running) return;
    const queryKey = messageContent(this.running.account_id, this.running.id).queryKey;
    const query = this.client.getQueryCache().find({ queryKey, exact: true });
    // An already-open reader owns the request now. Do not abort it.
    if (query && query.getObserversCount() === 0) void this.client.cancelQueries({ queryKey, exact: true });
  }
  cancel() { clearTimeout(this.timer); this.cancelInactive(); }
}

export function useMessagePrefetch(scope: string, next?: Message) {
  const client = useQueryClient();
  const prefetch = useMemo(() => new MessagePrefetch(client), [client, scope]);
  useEffect(() => {
    const cancel = () => prefetch.cancel();
    document.addEventListener("visibilitychange", cancel);
    window.addEventListener("offline", cancel);
    return () => {
      cancel();
      document.removeEventListener("visibilitychange", cancel);
      window.removeEventListener("offline", cancel);
    };
  }, [prefetch]);
  useEffect(() => {
    if (next) prefetch.request(next);
    return () => prefetch.cancel();
  }, [prefetch, next?.account_id, next?.id]);
  return prefetch;
}

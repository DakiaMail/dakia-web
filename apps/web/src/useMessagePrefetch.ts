import { useEffect, useMemo, type RefObject } from "react";
import { QueryClient, useQueryClient } from "@tanstack/react-query";
import type { Message } from "@dakia/api-types";
import { messageContent } from "./messageContent";

type Target = Pick<Message, "account_id" | "id">;
const identity = (m: Target) => JSON.stringify([m.account_id, m.id]);
const same = (a: Target | undefined, b: Target) => a && identity(a) === identity(b);
function allowed() {
  const connection = (navigator as Navigator & { connection?: { saveData?: boolean; effectiveType?: string } }).connection;
  return navigator.onLine && document.visibilityState === "visible" && !connection?.saveData && !["slow-2g", "2g"].includes(connection?.effectiveType || "");
}

// A replaceable viewport queue, one request at a time. No images, read-marking,
// conversation discovery or automatic retries. Foreground requests always win.
export class MessagePrefetch {
  private timer?: ReturnType<typeof setTimeout>;
  private running?: Target;
  private visible: Target[] = [];
  private intent?: Target;
  private next?: Target;
  constructor(private client: QueryClient) {}
  request(target: Target) { this.intent = target; this.schedule(); }
  clearIntent() { this.intent = undefined; this.schedule(); }
  setNext(target?: Target) { this.next = target; this.schedule(); }
  setVisible(targets: Target[]) {
    this.visible = [...new Map(targets.map(m => [identity(m), m])).values()];
    if (this.running && !this.targets().some(m => same(m, this.running!))) this.cancelInactive();
    this.schedule();
  }
  private targets() { return [...(this.intent ? [this.intent] : []), ...(this.next ? [this.next] : []), ...this.visible]; }
  private schedule(delay = 150) { clearTimeout(this.timer); this.timer = setTimeout(() => this.start(), delay); }
  private start() {
    if (!allowed() || this.running) return;
    if (this.client.isFetching({ queryKey: ["content"] })) { this.schedule(100); return; }
    const target = this.targets().find(target => {
      const state = this.client.getQueryState(messageContent(target.account_id, target.id).queryKey);
      return state?.status !== "error" && !(state?.dataUpdatedAt && Date.now() - state.dataUpdatedAt < 30000);
    });
    if (!target) return;
    const options = messageContent(target.account_id, target.id, this.client);
    this.running = target;
    void this.client.prefetchQuery({ ...options, gcTime: 60000, meta: { speculative: true } }).finally(() => {
      this.running = undefined;
      const retained = new Set(this.targets().map(identity));
      const unused = this.client.getQueryCache().findAll({ queryKey: ["content"] })
        .filter(q => q.meta?.speculative && q.getObserversCount() === 0 && q.state.fetchStatus === "idle" && !retained.has(identity({ account_id: String(q.queryKey[1]), id: String(q.queryKey[2]) })))
        .sort((a, b) => b.state.dataUpdatedAt - a.state.dataUpdatedAt);
      for (const q of unused.slice(3)) this.client.removeQueries({ queryKey: q.queryKey, exact: true });
      this.schedule(0);
    });
  }
  foreground(target: Target) {
    this.intent = undefined;
    if (this.running && !same(this.running, target)) this.cancelInactive();
    this.schedule();
  }
  private cancelInactive() {
    if (!this.running) return;
    const queryKey = messageContent(this.running.account_id, this.running.id).queryKey;
    const query = this.client.getQueryCache().find({ queryKey, exact: true });
    if (query && query.getObserversCount() === 0) void this.client.cancelQueries({ queryKey, exact: true });
  }
  cancel() { clearTimeout(this.timer); this.visible = []; this.intent = this.next = undefined; this.cancelInactive(); }
}

export function useMessagePrefetch(scope: string, next?: Message, root?: RefObject<HTMLDivElement | null>, rows: Message[] = []) {
  const client = useQueryClient();
  const prefetch = useMemo(() => new MessagePrefetch(client), [client, scope]);
  useEffect(() => {
    const cancel = () => prefetch.cancel();
    document.addEventListener("visibilitychange", cancel);
    window.addEventListener("offline", cancel);
    return () => { cancel(); document.removeEventListener("visibilitychange", cancel); window.removeEventListener("offline", cancel); };
  }, [prefetch]);
  useEffect(() => { prefetch.setNext(next); }, [prefetch, next?.account_id, next?.id]);
  useEffect(() => {
    const element = root?.current;
    if (!element || typeof IntersectionObserver === "undefined") return;
    const messages = new Map(rows.map(m => [`${m.account_id}/${m.id}`, m]));
    const visible = new Map<Element, Message>();
    const publish = () => prefetch.setVisible([...visible.values()]);
    const observer = new IntersectionObserver(entries => {
      for (const entry of entries) {
        const message = messages.get((entry.target as HTMLElement).dataset.messageId || "");
        if (entry.isIntersecting && entry.intersectionRect.height > 0 && entry.intersectionRect.width > 0 && message) visible.set(entry.target, message);
        else visible.delete(entry.target);
      }
      publish();
    }, { root: element, threshold: 0 });
    const observed = new Set<Element>();
    const scan = () => {
      const present = new Set(element.querySelectorAll(".message-row[data-message-id]"));
      for (const node of observed) if (!present.has(node)) { observer.unobserve(node); observed.delete(node); visible.delete(node); }
      for (const node of present) if (!observed.has(node)) { observed.add(node); observer.observe(node); }
      publish();
    };
    const changes = new MutationObserver(scan);
    changes.observe(element, { childList: true, subtree: true }); scan();
    return () => { changes.disconnect(); observer.disconnect(); prefetch.setVisible([]); };
  }, [prefetch, root, rows]);
  return prefetch;
}

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, cleanup, renderHook } from "@testing-library/react";
import { createElement } from "react";
import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { useMailEvents } from "./useMailEvents";
import { api } from "./api";
import { accounts } from "../browser/fixtures";

class Source {
  static instances: Source[] = [];
  onmessage?: (e: {data: string}) => void;
  onerror?: () => void;
  listeners = new Map<string, (e: {data: string}) => void>();
  closed = false;
  constructor(public url: string) { Source.instances.push(this); }
  close() { this.closed = true; }
  addEventListener(name: string, fn: (e: {data: string}) => void) { this.listeners.set(name, fn); }
  emit(cursor: number, kind = "message_changed", id = "work") {
    this.onmessage?.({data: JSON.stringify({account_id: id, cursor, kind, target_id: "target", revision: 1})});
  }
  ready(cursor: number, reset = true) { this.listeners.get("ready")?.({data: JSON.stringify({cursor, reset})}); }
}
let client: QueryClient;
function mount() {
  client = new QueryClient({defaultOptions: {queries: {retry: false, staleTime: Infinity}}});
  const invalidate = vi.spyOn(client, "invalidateQueries");
  const wrapper = ({children}: {children: React.ReactNode}) => createElement(QueryClientProvider, {client}, children);
  const hook = renderHook(() => useMailEvents(accounts), {wrapper});
  return {...hook, invalidate, wrapper};
}
beforeEach(() => { vi.useFakeTimers(); Source.instances = []; vi.stubGlobal("EventSource", Source); });
afterEach(() => { cleanup(); client?.clear(); vi.unstubAllGlobals(); vi.restoreAllMocks(); vi.useRealTimers(); });
describe("mail event request budget", () => {
  it("coalesces a replay burst, ignores duplicates, scopes accounts and event resources", async () => {
    const {invalidate} = mount();
    const source = Source.instances.find(s => s.url.includes("/work/"))!;
    act(() => { source.ready(100); for (let i=101;i<601;i++) source.emit(i); source.emit(600); source.emit(999,"message_changed","foreign"); });
    await act(() => vi.advanceTimersByTimeAsync(2000));
    expect(invalidate).toHaveBeenCalledTimes(1);
    const predicate = invalidate.mock.calls[0][0]!.predicate!;
    const query = (queryKey: unknown[]) => ({queryKey}) as unknown as Parameters<typeof predicate>[0];
    expect(predicate(query(["mail","all","INBOX","","",["work","personal"]]))).toBe(true);
    expect(predicate(query(["mail","personal","INBOX","","",["personal"]]))).toBe(false);
    expect(predicate(query(["content","work","target"]))).toBe(true);
    expect(predicate(query(["content","personal","target"]))).toBe(false);
    expect(predicate(query(["drafts","work"]))).toBe(true); // one reset
    act(() => source.emit(601, "draft_saved"));
    await act(() => vi.advanceTimersByTimeAsync(2000));
    const draftPredicate = invalidate.mock.calls[1][0]!.predicate!;
    expect(draftPredicate(query(["mail","all","INBOX","","",["work","personal"]]))).toBe(false);
    expect(draftPredicate(query(["drafts","work"]))).toBe(true);
  });
  it("does not refetch on stream errors and reconnects with the retained cursor/backoff", async () => {
    const {invalidate} = mount();
    const source = Source.instances[0];
    act(() => source.ready(321));
    await act(() => vi.advanceTimersByTimeAsync(2000));
    invalidate.mockClear();
    act(() => source.onerror?.());
    await act(() => vi.advanceTimersByTimeAsync(1999));
    expect(Source.instances).toHaveLength(2);
    await act(() => vi.advanceTimersByTimeAsync(1));
    expect(Source.instances[2].url).toContain("after=321");
    act(() => Source.instances[2].onerror?.());
    await act(() => vi.advanceTimersByTimeAsync(3999));
    expect(Source.instances).toHaveLength(3);
    await act(() => vi.advanceTimersByTimeAsync(1));
    expect(Source.instances).toHaveLength(4);
    expect(invalidate).not.toHaveBeenCalled();
  });
  it("closes streams and removes queued invalidations/reconnects on unmount", async () => {
    const {unmount, invalidate} = mount();
    act(() => {Source.instances[0].emit(1); Source.instances[1].onerror?.();});
    unmount();
    await act(() => vi.advanceTimersByTimeAsync(60000));
    expect(Source.instances).toHaveLength(2);
    expect(Source.instances.every(s => s.closed)).toBe(true);
    expect(invalidate).not.toHaveBeenCalled();
  });
  it("does not cancel/restart a slow query when more events arrive", async () => {
    const {wrapper} = mount();
    let release!: () => void;
    const load = vi.fn(() => new Promise<void>(r => {release=r;}));
    renderHook(() => useQuery({queryKey:["mail","all","INBOX","","",["work","personal"]], queryFn:load}), {wrapper});
    for(let i=1;i<=5;i++) {
      act(() => Source.instances.find(s => s.url.includes("/work/"))!.emit(i));
      await act(() => vi.advanceTimersByTimeAsync(2000));
    }
    expect(load).toHaveBeenCalledTimes(1);
    await act(async () => release());
  });
  it("refreshes broad local snapshots with one final catch-up after slow work, never provider discovery", async () => {
    const {wrapper} = mount();
    let release!: (value: number) => void;
    const local = vi.fn().mockResolvedValueOnce(1)
      .mockImplementationOnce(() => new Promise<number>(resolve => {release = resolve;}))
      .mockResolvedValue(3);
    const provider = vi.fn(async () => 1);
    renderHook(() => {
      useQuery({queryKey:["conversation-snapshot","work","selected"],queryFn:local});
      useQuery({queryKey:["conversation","work","selected"],queryFn:provider});
    }, {wrapper});
    await act(() => vi.advanceTimersByTimeAsync(1));
    const source = Source.instances.find(s => s.url.includes("/work/"))!;
    act(() => source.emit(1));
    await act(() => vi.advanceTimersByTimeAsync(2000));
    expect(local).toHaveBeenCalledTimes(2);
    act(() => {for(let i=2;i<=501;i++) source.emit(i, "future_unknown_event");});
    await act(() => vi.advanceTimersByTimeAsync(8000));
    expect(local).toHaveBeenCalledTimes(2);
    await act(async () => release(2));
    await act(() => vi.advanceTimersByTimeAsync(2000));
    expect(local).toHaveBeenCalledTimes(3);
    expect(provider).toHaveBeenCalledTimes(1);
  });
  it("passes cancellation through list/search and keeps parameters isolated", async () => {
    const fetcher = vi.fn(async (_url: string, _init: RequestInit) => new Response(JSON.stringify({rows:[]})));
    vi.stubGlobal("fetch", fetcher);
    const controller = new AbortController();
    await api.search("folder:INBOX", ["work"], "page2", "people", controller.signal);
    await api.messages("personal", "Sent", null, controller.signal);
    expect(fetcher.mock.calls[0][0]).toContain("accounts=work");
    expect(fetcher.mock.calls[0][0]).toContain("continuation=page2");
    expect(fetcher.mock.calls[1][0]).toContain("/accounts/personal/messages");
    for(const call of fetcher.mock.calls) expect((call as unknown as [string, RequestInit])[1].signal).toBe(controller.signal);
  });
});

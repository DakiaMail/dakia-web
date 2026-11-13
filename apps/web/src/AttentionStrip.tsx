import { useEffect, useState } from "react";
import { useInfiniteQuery } from "@tanstack/react-query";
import type { Message, Change } from "@dakia/api-types";
import { api } from "./api";
import { classification, needsAttention } from "./classification";
import { mailRows } from "./mailRows";
export function AttentionStrip({ account, ids, overrides, removed, snoozed, open, act, showAll }: {
  account: string; ids: string[]; overrides: Record<string, Message>; removed: Record<string, unknown>; snoozed: Record<string, unknown>;
  showAll: () => void;
  open: (message: Message) => void; act: (messages: Message[], change: Change) => void;
}) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 15000); return () => clearInterval(timer); }, []);
  const attention = useInfiniteQuery({ queryKey: ["mail", account, "INBOX", "", "attention:personal-first-3-30d", ids], initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) => api.search("folder:INBOX", ids, pageParam, undefined, signal, false, false, false, { attention: true }),
    getNextPageParam: (page, _pages, _param, params) => page.continuation && !params.includes(page.continuation) ? page.continuation : undefined,
    enabled: ids.length > 0, refetchInterval: 30000, retry: false,
  });
  const rows = mailRows([...(attention.data?.pages || []), { rows: Object.values(overrides).filter(m => ids.includes(m.account_id)) }], overrides).filter(m => needsAttention(m, now) && !removed[`${m.account_id}/${m.id}`] && !snoozed[`${m.account_id}/${m.thread_id}`] && m.folder_ids.some(f => f.toUpperCase() === "INBOX"));
  if (!rows.length && !attention.isError) return null;
  return <section className="attention-strip" aria-label="Needs attention">
    <div className="attention-heading"><h2>Needs attention</h2>{(rows.length > 3 || attention.hasNextPage) && <button onClick={showAll}>Show more</button>}</div>
    {attention.isError && <p role="status">Couldn’t update attention items. <button onClick={() => void attention.refetch()}>Try again</button></p>}
    {rows.slice(0, 3).map(m => <div className="attention-item" key={`${m.account_id}/${m.id}`}>
      <button className="attention-open" onClick={() => open(m)}><span>{m.sender.name || m.sender.email}</span><strong>{m.subject || "(No subject)"}</strong><span>{classification(m).attention.filter(a => !a.expires_at || Date.parse(a.expires_at) > now).map(a => a.label).join(" · ") || "Marked for attention"}</span></button>
      <button aria-label={`Dismiss attention for ${m.subject || "message"}`} onClick={event => {
        const next = event.currentTarget.closest(".attention-item")?.nextElementSibling?.querySelector<HTMLButtonElement>(".attention-open");
        act([m], { kind: "attention", value: false });
        requestAnimationFrame(() => { if (document.activeElement === document.body) (next?.isConnected ? next : document.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]'))?.focus(); });
      }}>Dismiss</button>
    </div>)}

  </section>;
}

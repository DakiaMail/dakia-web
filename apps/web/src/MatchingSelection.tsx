import { useEffect, useRef, useState, type ReactNode } from "react";
import type { Message, SearchPage } from "@dakia/api-types";
import { identity } from "./mailRows";
import { collectMatchingMail } from "./collectMatchingMail";

export function MatchingSelection({ visible, checked, expanded, accounts, fetchPage, select, busy, setBusy, actions, clear }: {
  visible: Message[]; checked: Set<string>; expanded: Message[] | undefined; accounts: string[];
  fetchPage: (token: string | null, signal: AbortSignal) => Promise<SearchPage>;
  select: (rows: Message[], expanded: boolean) => void;
  actions: ReactNode; clear: () => void;
  busy: boolean; setBusy: (value: boolean) => void;
}) {
  const [batch, setBatch] = useState<string[]>([]);
  const [finding, setFinding] = useState<number | null>(null);
  const [error, setError] = useState("");
  const request = useRef<AbortController | null>(null);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => () => { const current = request.current; request.current = null; current?.abort(); setBusy(false); }, []);
  const allVisible = visible.length > 0 && visible.every(m => checked.has(identity(m)));
  const someVisible = visible.some(m => checked.has(identity(m)));
  const offer = !expanded && batch.length > 0 && batch.every(id => checked.has(id));
  async function expand() {
    const controller = new AbortController(); request.current = controller;
    setFinding(0); setBusy(true); setError("");
    try {
      const rows = await collectMatchingMail(fetchPage, accounts, controller.signal, setFinding);
      if (!controller.signal.aborted) select(rows, true);
    } catch (e) {
      if (!controller.signal.aborted) setError(e instanceof Error ? e.message : "Couldn’t select every match.");
    } finally {
      if (request.current === controller) { request.current = null; setFinding(null); setBusy(false); requestAnimationFrame(() => { if (root.current?.isConnected) (root.current.querySelector<HTMLButtonElement>(".matching-selection-banner button") || root.current.querySelector<HTMLInputElement>("input"))?.focus(); }); }
    }
  }
  return <div className="selection-actions matching-selection" role="group" aria-label="Selected message actions" ref={root} hidden={!checked.size}>
    <label className="visible-selection-control"><input type="checkbox" aria-label="Select all" checked={allVisible}
      ref={input => { if (input) input.indeterminate = someVisible && !allVisible; }} disabled={!visible.length || busy}
      onChange={event => { const rows = event.target.checked ? visible : []; setBatch(rows.map(identity)); setError(""); select(rows, false); }} />
      <span>Select all</span>
    </label>
    <button className="selection-count" onClick={clear}>{checked.size} selected · Clear</button>
    {actions}
    {finding !== null ? <span role="status">Finding all matches… {finding} found <button onClick={() => request.current?.abort()}>Cancel</button></span>
      : expanded ? <span className="matching-selection-banner" role="status">{checked.size === expanded.length ? `All ${checked.size} matching messages are selected.` : `${checked.size} matching messages selected, including messages beyond this page.`} <button disabled={busy} onClick={() => { setBatch(visible.map(identity)); select(visible, false); }}>Select visible only</button></span>
      : offer && <span className="matching-selection-banner">{batch.length} visible messages selected. <button disabled={busy} onClick={() => void expand()}>Select all messages matching this filter</button></span>}
    {error && <span role="alert">{error}</span>}
  </div>;
}

import { useEffect, useRef, useState } from "react";
import type { Account, Message } from "@dakia/api-types";
import { api, type SenderMailPage } from "./api";
import { cleanupLimit, loadCleanupCandidates, useSenderCleanup } from "./cleanupJobs";
import { senderAddress } from "./mailRules";

type CleanupResult = { rows: Message[]; complete: boolean };
// Provider rows can have provisional locations. Use only their IDs to reconcile coverage.
export async function discoverSenderMail(message: Message, signal: AbortSignal, cutoff = Date.now()) {
  const ids = new Set<string>();
  const seen = new Set<string>();

  const q = `from:${JSON.stringify(message.sender.email)}`;
  let token: string | null = null;
  for (let pages = 0; pages < 1000; pages++) {
    try {
      const page = await api.search(q, [message.account_id], token, undefined, signal, true);
      for (const row of page.rows) {
        if (row.account_id === message.account_id && senderAddress(row.sender.email) === senderAddress(message.sender.email)
          && Date.parse(row.received_at) <= cutoff) ids.add(row.id);
      }
      if (signal.aborted) throw new DOMException("Aborted", "AbortError");
      if (ids.size > cleanupLimit) break;
      if (!page.continuation) return { ids, complete: !page.pending
        && page.coverage.length === 1 && page.coverage[0].account_id === message.account_id
        && page.coverage[0].state === "complete" };
      if (seen.has(page.continuation)) break;
      seen.add(page.continuation);
      token = page.continuation;
    } catch (error) {
      if (signal.aborted) throw error;
      break;
    }
  }
  return { ids, complete: false };
}
export function SenderCleanup({ message, account, cached, cacheFailed, onClose }: {
  message: Message; account: Account;
  cached?: SenderMailPage; cacheFailed: boolean; onClose: (rows: Message[]) => void;
}) {
  const cleanup = useSenderCleanup(message.account_id);
  const [found, setFound] = useState<CleanupResult>({ rows: [], complete: false });
  const [checking, setChecking] = useState(true);
  const [cacheLoaded, setCacheLoaded] = useState(false);
  const [review, setReview] = useState(false);
  const [listPage, setListPage] = useState(0);
  const [selected, setSelected] = useState<Message | null>(null);
  const [preview, setPreview] = useState("");

  const controller = useRef<AbortController | null>(null);
  const locked = useRef(false);
  useEffect(() => {
    if (locked.current || (!cached && !cacheFailed)) return;
    setChecking(true);
    setCacheLoaded(false);
    const abort = new AbortController(); controller.current = abort;
    const cutoff = cached?.cutoff ?? new Date().toISOString();
    const localCandidates = (first?: SenderMailPage) => loadCleanupCandidates(message.account_id, message.sender.email,
      Date.parse(cutoff), after => api.senderMail(message, abort.signal, cutoff, after), first);
    void (async () => {
      try {
        const local = await localCandidates(cached);
        if (!abort.signal.aborted) setFound({ rows: local.rows, complete: false });
      } catch { /* Provider discovery can still populate the local catalogue. */ }
      if (abort.signal.aborted) return;
      setCacheLoaded(true);
      const discovered = await discoverSenderMail(message, abort.signal, Date.parse(cutoff));
      if (abort.signal.aborted) return;
      const local = await localCandidates();
      if (abort.signal.aborted) return;
      setFound({
        rows: discovered.complete ? local.rows.filter(row => discovered.ids.has(row.id)) : local.rows,
        complete: discovered.complete && !local.limited,
      });
    })().catch(() => { /* Keep the last validated local results when discovery fails. */ })
      .finally(() => { if (!abort.signal.aborted) { setCacheLoaded(true); setChecking(false); } });
    return () => abort.abort();
  }, [cached, cacheFailed, message.account_id, message.sender.email, message.id]);
  useEffect(() => {
    if (!selected) return;
    const abort = new AbortController();
    setPreview("Loading email…");
    // Deliberately do not mount Conversation, which marks opened mail as read.
    void api.contentById(selected.account_id, selected.id, abort.signal).then(content => {
      if (!abort.signal.aborted) setPreview(content.text || "No plain-text version available.");
    }).catch(() => { if (!abort.signal.aborted) setPreview("Email unavailable. Go back to review the other emails."); });
    return () => abort.abort();
  }, [selected]);
  const rows = found.rows.filter(row => !cleanup.unavailable.has(row.id));
  function remove() {
    if (locked.current || !cacheLoaded || !rows.length || !cleanup.start(message, rows, found.complete)) return;
    locked.current = true;
    // Freeze exactly the displayed targets. Never add late results to a move.
    controller.current?.abort();
    setChecking(false);
    onClose(rows);
  }
  const count = cacheLoaded ? rows.length : cached?.count;
  const emails = count === 1 ? "email" : "emails";
  const selectedIndex = selected ? rows.findIndex(row => row.id === selected.id) : -1;
  const visiblePage = Math.min(listPage, Math.max(0, Math.ceil(rows.length / 10) - 1));
  return <section aria-label="Previous sender mail">
    <p className="sender-cleanup-scope"><b>{message.sender.email}</b><br />{account.email}</p>
    <p role="status">{count === undefined ? "Loading saved emails…" : found.complete ? `${count} previous ${emails}` : `${count} ${emails} found so far`}</p>
    <p className="muted">{checking ? "Checking for more…" : found.complete ? "Provider search finished." : "Search incomplete. Other emails may remain."}</p>
    <div className="form-actions">
      <button title="Move matching emails to Trash" className="primary" disabled={!cacheLoaded || !rows.length} onClick={remove}>Move {count ?? "the"} {found.complete ? emails : `found ${emails}`} to Trash</button>
      <button title={review ? "Hide emails" : "Review emails"} disabled={!rows.length} onClick={() => { setReview(!review); setSelected(null); }}>{review ? "Hide emails" : "Review emails"}</button>
    </div>
    {review && <div className="sender-cleanup-review">
      {selected ? <>
        <button title="Back to emails" onClick={() => setSelected(null)}>Back to emails</button>
        <h3>{selected.subject || "(no subject)"}</h3>
        <p>{selected.received_at.slice(0, 10)}</p>
        <pre className="sender-cleanup-preview">{preview}</pre>
        <div className="form-actions">
          <button title="Previous email" disabled={selectedIndex <= 0} onClick={() => setSelected(rows[selectedIndex - 1])}>Previous email</button>
          <button title="Next email" disabled={selectedIndex < 0 || selectedIndex >= rows.length - 1} onClick={() => setSelected(rows[selectedIndex + 1])}>Next email</button>
        </div>
      </> : <>
        <ul>{rows.slice(visiblePage * 10, visiblePage * 10 + 10).map(row => <li key={row.id}><button title={`Open ${row.subject || "message"}`} onClick={() => setSelected(row)}>{row.subject || "(no subject)"}<small>{row.received_at.slice(0, 10)}</small></button></li>)}</ul>
        <div className="form-actions">
          <button title="Previous page" disabled={visiblePage === 0} onClick={() => setListPage(visiblePage - 1)}>Previous page</button>
          <span>Page {visiblePage + 1} of {Math.max(1, Math.ceil(rows.length / 10))}</span>
          <button title="Next page" disabled={(visiblePage + 1) * 10 >= rows.length} onClick={() => setListPage(visiblePage + 1)}>Next page</button>
        </div>
      </>}
    </div>}
  </section>;
}

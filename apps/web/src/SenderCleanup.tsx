import { useEffect, useRef, useState } from "react";
import type { Account, Message } from "@dakia/api-types";
import { api, type SenderMailPage } from "./api";
import { useSenderCleanup } from "./cleanupJobs";

export type CleanupResult = { rows: Message[]; complete: boolean };
// Provider FROM searches may be substring matches. Never use them as mutation
// targets until exact sender and account checks have been applied on every page.
export async function findSenderMail(message: Message, signal: AbortSignal, onProgress?: (rows: Message[]) => void, cutoff = Date.now()): Promise<CleanupResult> {
  const rows = new Map<string, Message>();
  const seen = new Set<string>();

  const q = `from:${JSON.stringify(message.sender.email)}`;
  let token: string | null = null;
  for (let pages = 0; pages < 1000; pages++) {
    try {
      const page = await api.search(q, [message.account_id], token, undefined, signal, true);
      for (const row of page.rows) {
        if (row.account_id === message.account_id && row.sender.email.toLowerCase() === message.sender.email.toLowerCase()
          && Date.parse(row.received_at) <= cutoff) rows.set(row.id, row);
      }
      if (signal.aborted) throw new DOMException("Aborted", "AbortError");
      onProgress?.([...rows.values()]);
      if (rows.size > 10000) return { rows: [...rows.values()], complete: false };
      if (!page.continuation) return { rows: [...rows.values()], complete: !page.pending
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
  return { rows: [...rows.values()], complete: false };
}
export function SenderCleanup({ message, account, cached, cacheFailed, onClose }: {
  message: Message; account: Account;
  cached?: SenderMailPage; cacheFailed: boolean; onClose: (rows: Message[]) => void;
}) {

  const cleanup = useSenderCleanup(message.account_id);
  const job = cleanup.jobs.data?.find(job => job.sender === message.sender.email);
  const submission = cleanup.submissions.find(row => row.sender === message.sender.email);
  useEffect(() => {
    if (job || submission) onClose([]);
  }, [job?.request.id, submission?.request.id]);
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
    if (locked.current || job || submission || (!cached && !cacheFailed)) return;
    setChecking(true);
    setCacheLoaded(false);
    const abort = new AbortController(); controller.current = abort;
    const cutoff = cached ? Date.parse(cached.cutoff) : Date.now();
    const known = new Map<string, Message>();
    const add = (rows: Message[]) => {
      for (const row of rows) if (row.account_id === message.account_id && row.sender.email.toLowerCase() === message.sender.email.toLowerCase()
        && Date.parse(row.received_at) <= cutoff) known.set(row.id, row);
    };
    const publish = () => { if (!abort.signal.aborted) setFound({ rows: [...known.values()], complete: false }); };
    void (async () => {
      try {
        if (cached) {
          let page = cached;
          const seen = new Set<string>();
          for (let n = 0; n < 1000; n++) {
            add(page.rows); publish();
            if (!page.after || known.size > 10000 || seen.has(page.after)) break;
            seen.add(page.after);
            page = await api.senderMail(message, abort.signal, cached.cutoff, page.after);
            if (abort.signal.aborted) return;
          }
        }
      } catch { /* Keep the available cache and reconcile through the provider. */ }
      if (abort.signal.aborted) return;
      setCacheLoaded(true);
      const reconciled = await findSenderMail(message, abort.signal, rows => { add(rows); publish(); }, cutoff);
      if (abort.signal.aborted) return;
      // Only a fully covered search can discard stale cached membership.
      if (reconciled.complete) setFound(reconciled);
      else { add(reconciled.rows); publish(); }
      setChecking(false);
    })().catch(() => { if (!abort.signal.aborted) { setCacheLoaded(true); setChecking(false); } });
    return () => abort.abort();
  }, [cached, cacheFailed, message.account_id, message.sender.email, message.id, job?.request.id, submission?.request.id]);
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
  async function remove() {
    if (locked.current || !cacheLoaded || !found.rows.length) return;
    locked.current = true;
    // Freeze exactly the displayed targets. Never add late results to a move.
    controller.current?.abort();
    setChecking(false);
    cleanup.start(message, found.rows, found.complete);
    onClose(found.rows);
  }
  const count = cacheLoaded ? found.rows.length : cached?.count;
  const emails = count === 1 ? "email" : "emails";
  const selectedIndex = selected ? found.rows.findIndex(row => row.id === selected.id) : -1;
  const visiblePage = Math.min(listPage, Math.max(0, Math.ceil(found.rows.length / 10) - 1));
  return <section aria-label="Previous sender mail">
    <p className="sender-cleanup-scope"><b>{message.sender.email}</b><br />{account.email}</p>
    {!job && !submission && <>
      <p role="status">{count === undefined ? "Loading saved emails…" : found.complete ? `${count} previous ${emails}` : `${count} ${emails} found so far`}</p>
      <p className="muted">{checking ? "Checking for more…" : found.complete ? "All folders checked, except Trash." : "Search incomplete. Other emails may remain."}</p>
      <div className="form-actions">
        <button title="Move matching emails to Trash" className="primary" disabled={!cacheLoaded || !found.rows.length} onClick={() => void remove()}>Move {count ?? "the"} {found.complete ? emails : `found ${emails}`} to Trash</button>
        <button title={review ? "Hide emails" : "Review emails"} disabled={!found.rows.length} onClick={() => { setReview(!review); setSelected(null); }}>{review ? "Hide emails" : "Review emails"}</button>
      </div>
      {review && <div className="sender-cleanup-review">
        {selected ? <>
          <button title="Back to emails" onClick={() => setSelected(null)}>Back to emails</button>
          <h3>{selected.subject || "(no subject)"}</h3>
          <p>{selected.received_at.slice(0, 10)}</p>
          <pre className="sender-cleanup-preview">{preview}</pre>
          <div className="form-actions">
            <button title="Previous email" disabled={selectedIndex <= 0} onClick={() => setSelected(found.rows[selectedIndex - 1])}>Previous email</button>
            <button title="Next email" disabled={selectedIndex < 0 || selectedIndex >= found.rows.length - 1} onClick={() => setSelected(found.rows[selectedIndex + 1])}>Next email</button>
          </div>
        </> : <>
          <ul>{found.rows.slice(visiblePage * 10, visiblePage * 10 + 10).map(row => <li key={row.id}><button title={`Open ${row.subject || "message"}`} onClick={() => setSelected(row)}>{row.subject || "(no subject)"}<small>{row.received_at.slice(0, 10)}</small></button></li>)}</ul>
          <div className="form-actions">
            <button title="Previous page" disabled={visiblePage === 0} onClick={() => setListPage(visiblePage - 1)}>Previous page</button>
            <span>Page {visiblePage + 1} of {Math.max(1, Math.ceil(found.rows.length / 10))}</span>
            <button title="Next page" disabled={(visiblePage + 1) * 10 >= found.rows.length} onClick={() => setListPage(visiblePage + 1)}>Next page</button>
          </div>
        </>}
      </div>}
    </>}

  </section>;
}

import { DropdownMenuItem } from "@dakia/ui";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useQueries } from "@tanstack/react-query";
import type { Account, MailRule, Message } from "@dakia/api-types";
import { ShieldBan } from "lucide-react";
import { Modal, ErrorNotice } from "./ui";
import { announceRuleUpdate, ruleEmail, rulesApi, senderAddress, useMailRules } from "./mailRules";
import { api } from "./api";
import { loadCleanupCandidates, useSenderCleanups } from "./cleanupJobs";
import "./blockedSenders.css";

export type SenderAction = () => ReactNode;

export function SenderBlock({ message, accounts, onCleanup, children }: {
  message: Message; accounts: Account[]; onCleanup?: (rows: Message[]) => void;
  children: (action?: SenderAction) => ReactNode;
}) {
  const rules = useMailRules();
  const blocked = rules.data?.find(rule => rule.enabled && ruleEmail(rule) === senderAddress(message.sender.email));
  const [open, setOpen] = useState(false);
  const [saved, setSaved] = useState(false);
  const [review, setReview] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>();
  const [actionRule, setActionRule] = useState<MailRule>();
  const submitting = useRef(false);
  const savedFocus = useRef<HTMLButtonElement>(null);
  const reviewFocus = useRef<HTMLDivElement>(null);
  useEffect(() => { if (saved) savedFocus.current?.focus(); }, [saved]);
  useEffect(() => { if (review) reviewFocus.current?.focus(); }, [review]);
  const cutoff = useRef(Date.now());
  if (!message.sender.email || accounts.some(a => senderAddress(a.email) === senderAddress(message.sender.email))) return <>{children()}</>;
  const close = () => { if (!submitting.current) setOpen(false); };
  async function submit() {
    if (submitting.current) return;
    submitting.current = true; setBusy(true); setError(undefined);
    try {
      if (actionRule) await rulesApi.enable(actionRule.id, false);
      else { const rule = await rulesApi.block(message.sender.email); cutoff.current = Date.parse(rule.activated_at); }
      // The write is confirmed. Background refresh must not prevent dismissal.
      submitting.current = false; setBusy(false);
      setSaved(true); announceRuleUpdate(); await rules.refresh();
    } catch (error) { setError(error); await rules.refresh().catch(() => {}); }
    finally { submitting.current = false; setBusy(false); }
  }
  return <>
    {children(() => <><DropdownMenuItem label={blocked ? "Unblock sender" : "Block sender"} icon={<ShieldBan size={18} />} disabled={rules.isPending || rules.isError} onClick={() => {
      setActionRule(blocked); setSaved(false); setReview(false); setError(undefined); cutoff.current = Date.now(); setOpen(true);
    }} />
    {rules.isError && <DropdownMenuItem label="Retry sender controls" onClick={() => void rules.refetch()} />}</>)}
    {open && <Modal title={saved ? actionRule ? "Sender unblocked" : "Sender blocked" : actionRule ? "Unblock sender?" : "Block sender?"} onClose={close}>
      <div className="rules-dialog-content">
      <p className="blocked-address">{message.sender.email}</p>
      <ErrorNotice error={error} />
      {!saved ? <>
        <p>{actionRule ? "Future emails from this address will arrive normally. Emails already in Trash will stay there." : "Future emails from this address will move to Trash across your mailboxes."}</p>
        <div className="form-actions"><button type="button" disabled={busy} onClick={close}>Cancel</button><button type="button" className="primary" disabled={busy} onClick={() => void submit()}>{busy ? "Saving…" : actionRule ? "Unblock sender" : "Block sender"}</button></div>
      </> : <>
        {!actionRule && <>
          <p>Older emails are unchanged. You can unblock this sender in Workspace.</p>
          {!review ? <button ref={savedFocus} type="button" onClick={() => setReview(true)}>Review older emails</button> : <div ref={reviewFocus} role="region" tabIndex={-1} aria-label="Older sender mail" className="blocked-cleanup">
            <OlderSenderMail accounts={accounts} sender={message.sender.email} cutoff={cutoff.current} onCleanup={onCleanup} onDone={close} />
          </div>}
        </>}
        <div className="form-actions"><button ref={actionRule ? savedFocus : undefined} type="button" onClick={close}>Done</button></div>
      </>}
      </div>
    </Modal>}
  </>;
}

function OlderSenderMail({ accounts, sender, cutoff, onCleanup, onDone }: {
  accounts: Account[]; sender: string; cutoff: number; onCleanup?: (rows: Message[]) => void; onDone: () => void;
}) {
  const [review, setReview] = useState(0);
  const [jobIds, setJobIds] = useState<Record<string, string>>({});
  const [listPage, setListPage] = useState(0);
  const locked = useRef(false);
  const heading = useRef<HTMLHeadingElement>(null);
  const cutoffDate = new Date(cutoff).toISOString();
  const cleanup = useSenderCleanups(accounts.map(account => account.id));
  const searches = useQueries({ queries: accounts.map(account => ({
    queryKey: ["blocked-sender-mail", account.id, sender, cutoffDate, review],
    queryFn: ({ signal }: { signal: AbortSignal }) => loadCleanupCandidates(account.id, sender, cutoff, after => api.senderCleanupMail(account.id, sender, cutoffDate, signal, after)),
    retry: false, staleTime: Infinity, gcTime: 0,
  })) });
  const checking = searches.some(query => query.isPending);
  const searchFailed = searches.some(query => query.isError);
  const batches = accounts.flatMap((account, index) => {
    const rows = searches[index].data?.rows.filter(row => jobIds[account.id] || !cleanup[index].unavailable.has(row.id));
    return rows?.length ? [{ account, rows, start: cleanup[index].start }] : [];
  });
  const rows = batches.flatMap(batch => batch.rows.map(row => ({ row, account: batch.account })))
    .sort((a, b) => Date.parse(b.row.received_at) - Date.parse(a.row.received_at));
  const count = rows.length;
  const noun = count === 1 ? "email" : "emails";
  const submitted = Object.keys(jobIds).length > 0;
  const outcomes = accounts.flatMap((account, index) => {
    const id = jobIds[account.id];
    if (!id) return [];
    return [{ account, job: cleanup[index].jobs.data?.find(job => job.request.id === id),
      intent: cleanup[index].submissions.find(intent => intent.request.id === id) }];
  });
  const totals = { pending: 0, applied: 0, failed: 0 };
  for (const { job } of outcomes) {
    totals.pending += job?.pending ?? 0;
    totals.applied += job?.applied ?? 0;
    totals.failed += job?.failed ?? 0;
  }
  const waiting = outcomes.some(({ job, intent }) => !job && !intent?.error);
  const uncertain = outcomes.some(({ job, intent }) => !job && intent?.error && !intent.rejected);
  const rejected = outcomes.filter(({ job, intent }) => !job && intent?.rejected);
  const canRefresh = (!submitted && !checking && searchFailed) || (rejected.length > 0 && !waiting && !uncertain);
  const completed = outcomes.length > 0 && outcomes.every(({ job }) => job && !job.pending && !job.failed);
  const pageSize = 20;
  const pages = Math.ceil(count / pageSize);
  const visiblePage = Math.min(listPage, Math.max(0, pages - 1));

  function refreshReview() {
    heading.current?.focus();
    locked.current = false;
    setJobIds({});
    setListPage(0);
    setReview(value => value + 1);
  }

  return <>
    <h3 ref={heading} tabIndex={-1}>Older emails</h3>
    <p role="status">{count ? `${count} synced ${noun} available to move` : checking ? "Checking synced emails…" : "No synced emails available to move."}</p>
    {count > 0 && checking && <p role="status">Checking remaining mailboxes…</p>}
    {searchFailed && <p role="status">Some mailboxes couldn’t be checked.</p>}
    {searches.some(query => query.data?.limited) && <p role="status">Review is limited to 10,000 emails per mailbox at a time.</p>}
    {count > 0 && <details>
      <summary>Review emails</summary>
      <ul className="blocked-review-list" tabIndex={0} aria-label="Matching emails">{rows.slice(visiblePage * pageSize, (visiblePage + 1) * pageSize).map(({ row, account }) =>
        <li key={`${account.id}/${row.id}`}>{row.subject || "(no subject)"}<small>{account.display_name} · {row.received_at.slice(0, 10)}</small></li>)}</ul>
      {pages > 1 && <div className="form-actions">
        <button type="button" disabled={!visiblePage} onClick={() => setListPage(visiblePage - 1)}>Previous page</button>
        <span>Page {visiblePage + 1} of {pages}</span>
        <button type="button" disabled={visiblePage + 1 >= pages} onClick={() => setListPage(visiblePage + 1)}>Next page</button>
      </div>}
    </details>}
    {!submitted && count > 0 && <button type="button" disabled={checking} onClick={() => {
      if (locked.current) return;
      const started = Object.fromEntries(batches.flatMap(({ account, rows, start }) => {
        const id = start(rows[0], rows, false);
        return id ? [[account.id, id]] : [];
      }));
      if (!Object.keys(started).length) return;
      locked.current = true;
      heading.current?.focus();
      setJobIds(started);
    }}>Move {count} {noun} to Trash</button>}
    {waiting && <p role="status">Submitting cleanup…</p>}
    {outcomes.some(({ job }) => job) && <p role="status">{totals.pending} pending · {totals.applied} moved · {totals.failed} failed</p>}
    {uncertain && <p role="status">Cleanup status is unknown for some mailboxes. Check the mailbox before trying again.</p>}
    {rejected.map(({ account, intent }) => <p role="status" key={account.id}>{account.display_name}: {intent?.error === "cleanup_move_unverified"
      ? "An earlier move is unresolved. Review available emails again."
      : intent?.error === "revision_conflict" || intent?.error === "not_found"
        ? "Emails changed before they could be moved. Review available emails again."
        : "The move was rejected. Review available emails again."}</p>)}
    {canRefresh && <button type="button" onClick={refreshReview}>Review again</button>}
    {completed && <button type="button" onClick={() => { onDone(); onCleanup?.(rows.map(({ row }) => row)); }}>Back to mail</button>}
    <ErrorNotice error={cleanup.find(entry => entry.jobs.error)?.jobs.error} />
  </>;
}

export function BlockedSenders({ accounts }: { accounts: Account[] }) {
  const rules = useMailRules();
  const [confirm, setConfirm] = useState<MailRule>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>();
  const executions = useQueries({ queries: accounts.map(account => ({ queryKey: ["rule-executions", account.id], queryFn: ({ signal }: { signal: AbortSignal }) => rulesApi.executions(account.id, signal), retry: false })) });
  const active = rules.data?.filter(rule => rule.enabled) || [];
  return <section aria-labelledby="blocked-senders-heading" className="blocked-senders">
    <h2 id="blocked-senders-heading">Blocked senders</h2>
    <p>Future emails from these addresses move to Trash across your mailboxes.</p>
    <ErrorNotice error={rules.error || executions.find(query => query.error)?.error} />
    {rules.isPending ? <p role="status">Loading blocked senders…</p> : rules.isError ? <button type="button" onClick={() => void rules.refetch()}>Retry blocked senders</button> : !active.length ? <p className="muted">No blocked senders. Open an email to block its sender.</p> : <ul className="blocked-senders-list">{active.map(rule => {
      return <li key={rule.id}><div><span className="blocked-address">{ruleEmail(rule)}</span>{executions.map((query, index) => {
        const activity = query.data?.find(row => row.rule_id === rule.id);
        return activity && (activity.pending > 0 || activity.failed > 0) ? <small key={accounts[index].id} role="status">{accounts[index].display_name}: {activity.pending} pending · {activity.failed} couldn’t be moved</small> : null;
      })}</div><button type="button" aria-label={`Unblock ${ruleEmail(rule)}`} onClick={() => { setError(undefined); setConfirm(rule); }}>Unblock</button></li>;
    })}</ul>}
    {confirm && <Modal title="Unblock sender?" onClose={() => { if (!busy) setConfirm(undefined); }}>
      <div className="rules-dialog-content">
      <p className="blocked-address">{ruleEmail(confirm)}</p><p>Future emails will arrive normally. Emails already in Trash will stay there.</p><ErrorNotice error={error} />
      <div className="form-actions"><button type="button" disabled={busy} onClick={() => setConfirm(undefined)}>Cancel</button><button type="button" className="primary" disabled={busy} onClick={async () => {
        if (busy) return; setBusy(true); setError(undefined);
        try { await rulesApi.enable(confirm.id, false); announceRuleUpdate(); await rules.refresh(); setConfirm(undefined); }
        catch (error) { setError(error); } finally { setBusy(false); }
      }}>{busy ? "Saving…" : "Unblock sender"}</button></div>
      </div>
    </Modal>}
  </section>;
}

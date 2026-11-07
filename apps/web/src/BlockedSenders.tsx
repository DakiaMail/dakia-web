import { useEffect, useRef, useState } from "react";
import { useQueries } from "@tanstack/react-query";
import type { Account, MailRule, Message } from "@dakia/api-types";
import { ShieldBan } from "lucide-react";
import { Modal, ErrorNotice } from "./ui";
import { announceRuleUpdate, ruleEmail, rulesApi, senderAddress, useMailRules } from "./mailRules";
import { findSenderMail, type CleanupResult } from "./SenderCleanup";
import { useSenderCleanup } from "./cleanupJobs";
import "./blockedSenders.css";

export function SenderBlock({ message, accounts, onCleanup }: {
  message: Message; accounts: Account[]; onCleanup?: (rows: Message[]) => void;
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
  if (!message.sender.email || accounts.some(a => senderAddress(a.email) === senderAddress(message.sender.email))) return null;
  const close = () => { if (!submitting.current) setOpen(false); };
  async function submit() {
    if (submitting.current) return;
    submitting.current = true; setBusy(true); setError(undefined);
    try {
      if (actionRule) await rulesApi.enable(actionRule.id, false);
      else { const rule = await rulesApi.block(message.sender.email); cutoff.current = Date.parse(rule.activated_at); }
      setSaved(true); announceRuleUpdate(); await rules.refresh();
    } catch (error) { setError(error); await rules.refresh().catch(() => {}); }
    finally { submitting.current = false; setBusy(false); }
  }
  return <div className="sender-block">
    <button type="button" className="sender-block-trigger" disabled={rules.isPending || rules.isError} title={blocked ? "Unblock sender" : "Block sender"} onClick={() => {
      setActionRule(blocked); setSaved(false); setReview(false); setError(undefined); cutoff.current = Date.now(); setOpen(true);
    }}><ShieldBan size={15} aria-hidden="true" />{blocked ? "Unblock sender" : "Block sender"}</button>
    {rules.isError && <button type="button" onClick={() => void rules.refetch()}>Retry sender controls</button>}
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
          {!review ? <button ref={savedFocus} type="button" onClick={() => setReview(true)}>Review older emails</button> : <div ref={reviewFocus} tabIndex={-1} aria-label="Older sender mail" className="blocked-cleanup-groups">
            {accounts.map(account => <OlderSenderMail key={account.id} account={account} message={message} cutoff={cutoff.current} onCleanup={onCleanup} />)}
          </div>}
        </>}
        <div className="form-actions"><button ref={actionRule ? savedFocus : undefined} type="button" onClick={close}>Done</button></div>
      </>}
      </div>
    </Modal>}
  </div>;
}

function OlderSenderMail({ account, message, cutoff, onCleanup }: { account: Account; message: Message; cutoff: number; onCleanup?: (rows: Message[]) => void }) {
  const [found, setFound] = useState<CleanupResult>();
  const [checking, setChecking] = useState(true);
  const cleanup = useSenderCleanup(account.id);
  const [submitted, setSubmitted] = useState(false);
  const locked = useRef(false);
  const job = cleanup.jobs.data?.find(row => senderAddress(row.sender) === senderAddress(message.sender.email));
  const intent = cleanup.submissions.find(row => senderAddress(row.sender) === senderAddress(message.sender.email));
  useEffect(() => {
    const abort = new AbortController();
    void findSenderMail({ ...message, account_id: account.id }, abort.signal, undefined, cutoff).then(result => {
      if (!abort.signal.aborted) { setFound(result); setChecking(false); }
    }).catch(() => { if (!abort.signal.aborted) { setFound({ rows: [], complete: false }); setChecking(false); } });
    return () => abort.abort();
  }, [account.id, message.sender.email, cutoff]);
  const count = found?.rows.length || 0;
  const noun = count === 1 ? "email" : "emails";
  return <section className="blocked-cleanup-account" aria-label={`Older emails in ${account.display_name}`}>
    <h3>{account.display_name}</h3><p className="muted">{account.email}</p>
    {checking ? <p role="status">Checking older emails…</p> : <>
      <p>{count} {noun}{!found?.complete && " found so far"}</p>
      {!found?.complete && <p role="status">Search incomplete. Other emails may remain.</p>}
      {!!found?.rows.length && <details><summary>Review emails</summary><ul className="blocked-review-list">{found.rows.map(row => <li key={row.id}>{row.subject || "(no subject)"}<small>{row.received_at.slice(0, 10)}</small></li>)}</ul></details>}
      {!submitted && !job && !intent && <button type="button" disabled={!found?.rows.length} onClick={() => {
        if (locked.current || !found?.rows.length) return;
        locked.current = true; cleanup.start(found.rows[0], found.rows, found.complete); setSubmitted(true);
        // Keep this review open so outcomes remain visible. The ordinary app
        // projection removes frozen targets without unmounting the dialog.
      }}>Move {count} {noun} to Trash</button>}
    </>}
    {(submitted || job || intent) && <p role="status">{intent?.error ? intent.rejected ? "Cleanup was rejected. No move was accepted." : "Cleanup status is unknown. Check again before trying another cleanup." : job ? `${job.pending} pending · ${job.applied} moved · ${job.failed} failed` : "Submitting cleanup…"}</p>}
    {job && !job.pending && !job.failed && <button type="button" onClick={() => onCleanup?.(found?.rows || [])}>Back to mail</button>}
    <ErrorNotice error={cleanup.jobs.error} />
  </section>;
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

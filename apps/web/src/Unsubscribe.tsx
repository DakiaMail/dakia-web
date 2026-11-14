import { useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Account, Draft, Message } from "@dakia/api-types";
import { MailMinus } from "lucide-react";
import { api, accountPath, ApiFailure, key, request } from "./api";
import { SenderCleanup } from "./SenderCleanup";

import { Modal } from "./ui";

export type UnsubscribeAction =
  | { kind: "web" | "one_click"; url: string }
  | { kind: "mailto"; to: string; subject: string; body: string };
// Defense in depth: never turn an unexpected API response into an active link.
export function validAction(value: unknown): value is UnsubscribeAction {
  if (!value || typeof value !== "object") return false;
  const a = value as UnsubscribeAction;
  if (a.kind === "web" || a.kind === "one_click") {
    try {
      const u = new URL(a.url);
      return ["https:", ...(a.kind === "web" ? ["http:"] : [])].includes(u.protocol)
        && !u.username && !u.password && !u.port && u.hostname.includes(".")
        && !/^(localhost|127\.|10\.|192\.168\.|169\.254\.)/.test(u.hostname)
        && !/\.(local|localhost|internal|test|invalid)$/.test(u.hostname);
    } catch { return false; }
  }
  return a.kind === "mailto" && typeof a.to === "string" && /^[^\s<>@,;]+@[^\s<>@,;]+\.[^\s<>@,;]+$/.test(a.to)
    && typeof a.subject === "string" && !/[\r\n\0]/.test(a.subject) && typeof a.body === "string" && !a.body.includes("\0");
}
type Info = UnsubscribeAction & { attempt_status?: string | null };
function outcomeError(status: string) {
  if (status === "not_sent") return "The unsubscribe request wasn’t sent.";
  if (status === "request_rejected") return "The sender did not accept the unsubscribe request.";
  return "Unsubscribe status is unavailable. No further request will be sent.";
}
export function Unsubscribe({ message, account, onCleanup, lazy = false }: { message: Message; account?: Account; onDraft?: (draft: Draft) => void; lazy?: boolean; onCleanup?: (rows: Message[]) => void }) {
  const path = `${accountPath(message.account_id)}/messages/${key(message.id)}/unsubscribe`;
  const info = useQuery({ queryKey: ["unsubscribe", message.account_id, message.id], queryFn: ({ signal }) => request<Info | null>(path, { signal }), staleTime: 0, retry: false, enabled: !lazy });
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);

  const [attempted, setAttempted] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState("");
  const trigger = useRef<HTMLButtonElement>(null);
  const sending = useRef(false);
  const action = validAction(info.data) ? info.data : null;
  const cached = useQuery({ queryKey: ["sender-mail", message.account_id, message.id], queryFn: ({ signal }) => api.senderMail(message, signal), enabled: busy || open, retry: false, staleTime: 0, refetchOnWindowFocus: false });
  const status = info.data?.attempt_status;
  const success = done || status === "request_accepted";
  const failure = busy ? "" : error || (status && status !== "request_accepted" ? outcomeError(status) : "");
  if (!action && !lazy) return null;
  const close = () => { if (busy) return false; setOpen(false); requestAnimationFrame(() => trigger.current?.focus()); };
  const confirm = async () => {
    if (sending.current) return;
    sending.current = true;
    setBusy(true);
    try {
      const current = validAction(info.data) ? info.data : (await info.refetch()).data;
      const action = validAction(current) ? current : null;
      const status = current?.attempt_status;
      if (!action) { setError("Unsubscribe isn’t available for this email."); return; }
      if (done || status === "request_accepted" || attempted || (status && status !== "attempted")) { setOpen(true); return; }
      // Web unsubscribe pages remain explicit links, opened only by a click.
      if (action.kind === "web") return;
      setAttempted(true);
      const expected = action.kind === "mailto" ? { kind: action.kind, to: action.to, subject: action.subject, body: action.body } : { kind: action.kind, url: action.url };
      const response = !status ? await request<{ queued: boolean } | undefined>(path, { method: "POST", body: JSON.stringify({ expected_action: expected }) }) : { queued: true };
      if (!response?.queued) { setDone(true); setOpen(true); return; }
      let recovered = await info.refetch();
      for (let reads = 0; reads < 30 && recovered.data?.attempt_status === "attempted"; reads++) {
        await new Promise(resolve => setTimeout(resolve, 1000));
        recovered = await info.refetch();
      }
      if (recovered.data?.attempt_status === "request_accepted") { setDone(true); setOpen(true); }
      else setError(outcomeError(recovered.data?.attempt_status || "result_unknown"));
    } catch (error) {
      // Lost browser acknowledgements are reconciled by GET, never a second POST.
      let recovered = await info.refetch();
      // A disconnected browser can arrive before the bounded server request
      // finishes. Wait briefly for its durable outcome using reads only.
      for (let reads = 0; reads < 20 && recovered.data?.attempt_status === "attempted" && !(error instanceof ApiFailure); reads++) {
        await new Promise(resolve => setTimeout(resolve, 1000));
        recovered = await info.refetch();
      }
      if (recovered.data?.attempt_status === "request_accepted") { setDone(true); setOpen(true); }
      else setError(outcomeError(recovered.data?.attempt_status || (error instanceof ApiFailure && error.code === "unsubscribe_not_confirmed" ? "request_rejected" : "result_unknown")));
    } finally { sending.current = false; setBusy(false); }
  };
  return <div className="unsubscribe">
    {action?.kind === "web" ? <a href={action.url} target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer">Unsubscribe <span>{new URL(action.url).hostname}</span></a> : <button title="Unsubscribe" ref={trigger} className="unsubscribe-trigger" disabled={busy} onClick={() => void confirm()}><MailMinus size={15} />Unsubscribe</button>}
    {failure && !open && <p role="alert" className="error">{failure}</p>}
    {open && <Modal title={success ? "Unsubscribe request sent" : "Unsubscribe request"} onClose={close}>
      <div className="unsubscribe-content">
      {success ? <>{account && <SenderCleanup message={message} account={account} cached={cached.data} cacheFailed={cached.isError} onClose={rows => { close(); onCleanup?.(rows); }} />}<div className="form-actions"><button title="Done" onClick={close}>Done</button></div></> : <><p role="alert">{failure}</p><button title="Close" onClick={close}>Close</button></>}
      </div>
    </Modal>}
  </div>;
}

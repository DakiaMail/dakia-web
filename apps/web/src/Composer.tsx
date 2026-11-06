import { useCallback, useEffect, useRef, useState } from "react";
import type {
  Account,
  Address,
  Attachment,
  Draft,
  Submission,
} from "@dakia/api-types";
import { RichEditor } from "@dakia/mail-composer";
import { MailReader, sanitizeMail } from "@dakia/mail-renderer";
import { Paperclip, Send, Trash2 } from "lucide-react";
import { api, ApiFailure } from "./api";
import { Modal, ErrorNotice } from "./ui";
import { RecipientField } from "./RecipientField";
import { SendLater } from "./SendLaterDialog";
import { localDateTime, validSendTime } from "./sendLater";
export function splitQuoted(html: string) {
  const doc = new DOMParser().parseFromString(html, "text/html");
  const quote = doc.body.querySelector('[data-dakia-quote="true"]');
  const quoted = quote?.innerHTML || "";
  quote?.remove();
  return { html: doc.body.innerHTML, quote: quoted };
}
function plainText(html: string) {
  const doc = new DOMParser().parseFromString(html, "text/html");
  for (const node of doc.body.querySelectorAll("p,div,br,li,blockquote"))
    node.append(doc.createTextNode("\n"));
  return doc.body.textContent || "";
}
export function parseRecipients(value: string): Address[] {
  return value
    .split(/[,;\n]/)
    .map((v) => v.trim())
    .filter(Boolean)
    .map((email) => {
      if (!/^[^\s<>@]+@[^\s<>@]+\.[^\s<>@]+$/.test(email))
        throw new Error(`Check recipient: ${email}`);
      return { email, name: null };
    });
}
export function newDraft(account: string): Draft {
  return {
    id: crypto.randomUUID(),
    account_id: account,
    revision: 0,
    to: [],
    cc: [],
    bcc: [],
    subject: "",
    text: "",
    html: "",
    in_reply_to: null,
    references: [],
    attachments: [],
    saved_at: new Date().toISOString(),
  };
}
export function Composer({
  initial,
  accounts,
  quote,
  forwardAttachments,
  initialDue,
  onClose,
  onSubmitted,
}: {
  initial: Draft;
  accounts: Account[];
  quote?: string;
  forwardAttachments?: Attachment[];
  initialDue?: string;
  onClose: () => void;
  onSubmitted: (s: Submission) => void;
}) {
  const [draft, setDraft] = useState(() => ({
    ...initial,
    html: splitQuoted(initial.html).html,
  }));
  const [quoted, setQuoted] = useState(
    () => quote || splitQuoted(initial.html).quote,
  );
  const quoteRef = useRef(quoted);
  quoteRef.current = quoted;
  const [editorVersion, setEditorVersion] = useState(0);
  const [fields, setFields] = useState({
    to: initial.to.map((a) => a.email).join(", "),
    cc: initial.cc.map((a) => a.email).join(", "),
    bcc: initial.bcc.map((a) => a.email).join(", "),
  });
  const [extras, setExtras] = useState(
    initial.cc.length + initial.bcc.length > 0,
  );
  const [status, setStatus] = useState(
    initial.revision ? "Saved" : "Unsaved changes",
  );
  const [error, setError] = useState<unknown>();
  const [busy, setBusy] = useState(false);
  const [due, setDue] = useState(() => {
    if (!initialDue) return "";
    const date = new Date(initialDue);
    return localDateTime(date);
  });
  const [conflict, setConflict] = useState(false);
  const current = useRef(draft);
  const fieldsRef = useRef(fields);
  fieldsRef.current = fields;
  const revision = useRef(initial.revision);
  const version = useRef(initial.revision ? 0 : 1);
  const saved = useRef(0);
  const flight = useRef<Promise<Draft> | null>(null);
  const mounted = useRef(true);
  const halted = useRef(false);
  const change = useCallback((patch: Partial<Draft>) => {
    current.current = { ...current.current, ...patch };
    setDraft(current.current);
    version.current++;
    setStatus("Unsaved changes");
  }, []);
  const save = useCallback(async (): Promise<Draft> => {
    if (halted.current)
      throw new Error("Resolve the draft conflict before saving.");
    if (flight.current) {
      await flight.current;
      return save();
    }
    if (saved.current === version.current && revision.current > 0)
      return current.current;
    const v = version.current;
    let recipients: Pick<Draft, "to" | "cc" | "bcc">;
    try {
      recipients = {
        to: parseRecipients(fieldsRef.current.to),
        cc: parseRecipients(fieldsRef.current.cc),
        bcc: parseRecipients(fieldsRef.current.bcc),
      };
    } catch (e) {
      setError(e);
      setStatus("Not saved");
      throw e;
    }
    const fullHtml =
      current.current.html +
      (quoteRef.current
        ? `<blockquote data-dakia-quote="true">${sanitizeMail(quoteRef.current)}</blockquote>`
        : "");
    const payload = {
      ...current.current,
      html: fullHtml,
      text: plainText(fullHtml),
      revision: revision.current,
      ...recipients,
    };
    setStatus("Saving…");
    const pending = api.saveDraft(payload);
    flight.current = pending;
    try {
      const result = await pending;
      revision.current = result.revision;
      saved.current = v;
      current.current = {
        ...current.current,
        revision: result.revision,
        saved_at: result.saved_at,
        to: payload.to,
        cc: payload.cc,
        bcc: payload.bcc,
      };
      if (mounted.current) {
        setDraft(current.current);
        setStatus(v === version.current ? "Saved" : "Unsaved changes");
        setError(undefined);
      }
      return current.current;
    } catch (e) {
      if (e instanceof ApiFailure && e.status === 409) {
        halted.current = true;
        if (mounted.current) setConflict(true);
      }
      if (mounted.current) {
        setError(e);
        setStatus("Not saved");
      }
      throw e;
    } finally {
      flight.current = null;
    }
  }, []);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    if (status !== "Unsaved changes" || conflict) return;
    const t = setTimeout(() => void save().catch(() => {}), 800);
    return () => clearTimeout(t);
  }, [draft, fields, status, conflict, save]);
  useEffect(() => {
    const unload = (e: BeforeUnloadEvent) => {
      if (saved.current !== version.current) {
        e.preventDefault();
        e.returnValue = "";
      }
    };
    window.addEventListener("beforeunload", unload);
    return () => window.removeEventListener("beforeunload", unload);
  }, []);
  async function send() {
    setBusy(true);
    setError(undefined);
    try {
      let snapshot = await save();
      while (saved.current !== version.current) snapshot = await save();
      if (!snapshot.to.length) throw new Error("Add at least one recipient.");
      if (due && !validSendTime(due))
        throw new Error("Choose a future send time.");
      // Preserve one operation ID across an uncertain HTTP retry for this exact draft revision.
      const operationKey = `${snapshot.id}:${snapshot.revision}:${due}`;
      if (sendKey.current?.key !== operationKey)
        sendKey.current = { key: operationKey, id: crypto.randomUUID() };
      const result = await api.submit(snapshot.account_id, {
        operation_id: sendKey.current.id,
        draft_id: snapshot.id,
        expected_revision: snapshot.revision,
        due_at: due ? new Date(due).toISOString() : null,
        timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      });
      onSubmitted(result);
      onClose();
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  const sendKey = useRef<{ key: string; id: string } | undefined>(undefined);
  async function close() {
    if (busy) return false;
    try {
      if (saved.current !== version.current) await save();
      onClose();
      return true;
    } catch { return false; }
  }
  async function attach(files: FileList | null) {
    if (!files) return;
    setBusy(true);
    setError(undefined);
    try {
      let total = current.current.attachments.reduce(
        (n, a) => n + a.size_bytes,
        0,
      );
      for (const file of files) {
        total += file.size;
        if (total > 20 * 1024 * 1024)
          throw new Error("Attachments exceed 20 MiB.");
        const a = await api.upload(current.current.account_id, file);
        change({ attachments: [...current.current.attachments, a] });
      }
    } catch (e) {
      setError(e);
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal
      title={initial.in_reply_to ? "Reply" : "New message"}
      initialFocus={initial.to.length ? '[aria-label="Message"]' : '[aria-label="To"]'}
      onClose={close}
      wide
    >
      <div className="compose">
        <ErrorNotice error={error} />
        {conflict && (
          <div className="conflict">
            <p>
              This draft changed in another tab. Your version is still here.
            </p>
            <button title="Save my version as a new draft"
              onClick={() => {
                const copy = {
                  ...current.current,
                  id: crypto.randomUUID(),
                  revision: 0,
                };
                revision.current = 0;
                halted.current = false;
                setConflict(false);
                change(copy);
              }}
            >
              Save my version as a new draft
            </button>
            <button title="Load server version"
              onClick={async () => {
                try {
                  const latest = await api.draft(draft.account_id, draft.id);
                  revision.current = latest.revision;
                  halted.current = false;
                  setConflict(false);
                  current.current = latest;
                  const parts = splitQuoted(latest.html);
                  current.current = { ...latest, html: parts.html };
                  setDraft(current.current);
                  setQuoted(parts.quote);
                  setEditorVersion((v) => v + 1);
                  setFields({
                    to: latest.to.map((a) => a.email).join(", "),
                    cc: latest.cc.map((a) => a.email).join(", "),
                    bcc: latest.bcc.map((a) => a.email).join(", "),
                  });
                  version.current = 0;
                  saved.current = 0;
                  setStatus("Saved");
                } catch (e) {
                  setError(e);
                }
              }}
            >
              Load server version
            </button>
          </div>
        )}
        <label className="compose-field">
          From
          <select title="Send from"
            value={draft.account_id}
            disabled={
              busy ||
              conflict ||
              !!flight.current ||
              draft.attachments.length > 0
            }
            onChange={(e) => {
              revision.current = 0;
              saved.current = 0;
              change({
                account_id: e.target.value,
                id: crypto.randomUUID(),
                revision: 0,
              });
            }}
          >
            {accounts
              .filter((a) => a.state === "ready")
              .map((a) => (
                <option key={a.id} value={a.id}>
                  {a.display_name} &lt;{a.email}&gt;
                </option>
              ))}
          </select>
        </label>
        {(
          ["to", ...(extras ? ["cc", "bcc"] : [])] as ("to" | "cc" | "bcc")[]
        ).map((name) => (
          <label className="compose-field" key={name}>
            {name === "to" ? "To" : name === "cc" ? "Cc" : "Bcc"}
            <RecipientField
              account={draft.account_id}
              label={name === "to" ? "To" : name === "cc" ? "Cc" : "Bcc"}
              value={fields[name]}
              disabled={busy}
              onChange={(value) => {
                setFields({ ...fields, [name]: value });
                version.current++;
                setStatus("Unsaved changes");
              }}
            />
            {name === "to" && !extras && (
              <button title="Cc / Bcc" onClick={event => {
                const composer = event.currentTarget.closest(".compose");
                setExtras(true);
                requestAnimationFrame(() => composer?.querySelector<HTMLInputElement>('[aria-label="Cc"]')?.focus());
              }}>Cc / Bcc</button>
            )}
          </label>
        ))}
        <label className="compose-field">
          Subject
          <input
            aria-label="Subject"
            value={draft.subject}
            disabled={busy}
            onChange={(e) => change({ subject: e.target.value })}
          />
        </label>
        <div inert={busy || conflict}>
          <RichEditor
            key={`${draft.id}:${editorVersion}`}
            initialHtml={draft.html}
            onChange={(html, text) => change({ html, text })}
          />
        </div>
        {quoted && (
          <details className="quote">
            <summary>Quoted message</summary>
            <MailReader html={quoted} text="" title="Quoted message" />
          </details>
        )}
        {forwardAttachments?.length ? (
          <fieldset className="forward-attachments">
            <legend>Forward attachments</legend>
            {forwardAttachments.map((a) => (
              <label key={a.id}>
                <input
                  type="checkbox"
                  disabled={busy}
                  checked={draft.attachments.some((x) => x.id === a.id)}
                  onChange={(e) =>
                    change({
                      attachments: e.target.checked
                        ? [...current.current.attachments, a]
                        : current.current.attachments.filter(
                            (x) => x.id !== a.id,
                          ),
                    })
                  }
                />
                {a.filename}
              </label>
            ))}
          </fieldset>
        ) : null}
        <div className="attachments">
          {draft.attachments.map((a) => (
            <span className="attachment" key={a.id}>
              {a.filename}
              <button title={`Remove ${a.filename}`}
                aria-label={`Remove ${a.filename}`}
                disabled={busy}
                onClick={() =>
                  change({
                    attachments: current.current.attachments.filter(
                      (x) => x.id !== a.id,
                    ),
                  })
                }
              >
                <Trash2 size={14} />
              </button>
            </span>
          ))}
        </div>
        <footer>
          <button title={due ? "Schedule send" : "Send"}
            className="primary"
            disabled={busy || conflict}
            onClick={() => void send()}
          >
            <Send size={16} />
            {busy ? "Working…" : due ? "Schedule send" : "Send"}
          </button>
          <label className="file-button" title="Attach files">
            <Paperclip size={18} />
            <span className="sr-only">Attach files</span>
            <input
              aria-label="Attach files"
              type="file"
              multiple
              disabled={busy}
              onChange={(e) => {
                void attach(e.target.files);
                e.target.value = "";
              }}
            />
          </label>
          <SendLater value={due} disabled={busy || conflict} onChange={setDue} />
          <span role="status" className="save-status">
            {status}
          </span>
          {status === "Not saved" && !conflict && (
            <button title="Retry save" onClick={() => void save().catch(() => {})}>
              Retry save
            </button>
          )}
        </footer>
      </div>
    </Modal>
  );
}

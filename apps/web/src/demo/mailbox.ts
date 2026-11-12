import { classification, matchesClassification, optimisticClassification, type MailFilter } from "../classification";
import type { Draft, MailRule, Message, MessageContent, Operation, SearchPage, Submission, SubmitRequest } from "@dakia/api-types";
import type { CleanupJob, CleanupRequest } from "../cleanupJobs";
import { defaultSettings, frenchTranslation, identity, paragraphs, seed, type DemoState, type StoredAttachment } from "./seed";

export const DATABASE = "dakia-sandbox";
export interface Persistence { read(): Promise<unknown>; write(value: DemoState): Promise<void> }
export function browserPersistence(): Persistence {
  let connection: Promise<IDBDatabase> | undefined;
  const open = () => connection ||= new Promise((resolve, reject) => {
    const request = indexedDB.open(DATABASE, 1);
    request.onupgradeneeded = () => request.result.createObjectStore("state");
    request.onsuccess = () => { request.result.onversionchange = () => request.result.close(); resolve(request.result); };
    request.onerror = () => reject(request.error);
    request.onblocked = () => reject(new Error("Storage is busy"));
  });
  return {
    async read() { const db = await open(); return new Promise((resolve, reject) => {
      const request = db.transaction("state").objectStore("state").get("mailbox");
      request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
    }); },
    async write(value) { const db = await open(); return new Promise((resolve, reject) => {
      const tx = db.transaction("state", "readwrite"); tx.objectStore("state").put(value, "mailbox");
      tx.oncomplete = () => resolve(); tx.onabort = tx.onerror = () => reject(tx.error);
    }); },
  };
}
function compatible(value: unknown): value is DemoState {
  if (!value || typeof value !== "object") return false;
  const s = value as DemoState;
  return s.version === 2 && typeof s.user === "string" && Number.isSafeInteger(s.cursor)
    && Array.isArray(s.accounts) && s.accounts.every(a => a && typeof a.id === "string" && typeof a.email === "string")
    && !!s.contents && Object.values(s.contents).every(c => c?.message && Array.isArray(c.message.folder_ids) && typeof c.text === "string" && typeof c.html === "string")
    && [s.drafts, s.submissions, s.attachments, s.operations, s.settings, s.jobs, s.snoozed].every(v => !!v && typeof v === "object" && !Array.isArray(v))
    && Array.isArray(s.rules) && Array.isArray(s.cleanups) && Array.isArray(s.unsubscribed);
}
const json = (value: unknown, status = 200) => Response.json(value, { status });
const fail = (status: number, message: string) => json({ code: status === 404 ? "not_found" : status === 409 ? "conflict" : "unavailable", message }, status);
const missing = () => fail(404, "This item is no longer available.");
const unsupported = () => fail(403, "This action is unavailable.");
const folders = ["INBOX", "Sent", "Archive", "Trash", "Spam", "Projects"];
function apply(message: Message, change: Operation["change"], state: DemoState): Message {
  const result = { ...message, revision: message.revision + 1 };
  switch (change.kind) {
    case "read": result.unread = !change.value; break;
    case "star": result.starred = change.value; break;
    case "inbox_kind":
    case "detected_type":
    case "attention": {
      const id = identity(message.account_id, message.id);
      const overrides = state.classificationOverrides[id] ||= {};
      if (change.kind === "inbox_kind") {
        if (change.sender) {
          const sender = identity(message.account_id, message.sender.email.toLowerCase());
          if (change.value) state.senderClassifications[sender] = change.value; else delete state.senderClassifications[sender];
          if (change.value) overrides.kind = null;
        } else overrides.kind = change.value;
      }
      if (change.kind === "attention") overrides.attention = change.value;
      if (change.kind === "detected_type") {
        overrides.added = (overrides.added || []).filter(t => t !== change.value);
        overrides.removed = (overrides.removed || []).filter(t => t !== change.value);
        (change.present ? overrides.added : overrides.removed).push(change.value);
      }
      const effective = (m: Message) => {
        const k = identity(m.account_id, m.id), o = state.classificationOverrides[k] || {};
        const c = structuredClone(state.automaticClassifications[k] || classification(m));
        const sender = state.senderClassifications[identity(m.account_id, m.sender.email.toLowerCase())];
        c.kind = o.kind || sender || c.kind; c.source = o.kind ? "message" : sender ? "sender" : "automatic";
        c.types = [...new Set([...c.types, ...(o.added || [])])].filter(t => !o.removed?.includes(t));
        c.attention_override = o.attention ?? null;
        return { ...m, classification: c };
      };
      if (change.kind === "inbox_kind" && change.sender) for (const content of Object.values(state.contents)) if (content.message.account_id === message.account_id && content.message.sender.email === message.sender.email && content.message.id !== message.id) content.message = { ...effective(content.message), revision: content.message.revision + 1 };
      return effective(result);
    }
    case "category": result.category = change.value; result.category_status = "ready"; break;
    case "archive": result.folder_ids = [state.settings[message.account_id]?.archive_folder || "Archive"]; break;
    case "trash": result.folder_ids = [state.settings[message.account_id]?.trash_folder || "Trash"]; break;
    case "spam": result.folder_ids = ["Spam"]; break;
    case "move": result.folder_ids = [change.folder_id]; break;
    case "snooze":
      for (const content of Object.values(state.contents)) {
        const row = content.message;
        if (row.account_id === message.account_id && row.thread_id === message.thread_id)
          state.snoozed[identity(row.account_id, row.id)] = { until: change.until, folders: row.folder_ids };
      }
      break;
  }
  return result;
}
export function matches(content: MessageContent, query: string): boolean {
  const m = content.message;
  const terms = query.match(/(?:[^\s"]+|"[^"]*")+/g) || [];
  return terms.every(term => {
    const negative = term.startsWith("-");
    const token = (negative ? term.slice(1) : term).replaceAll('"', "").toLowerCase();
    const colon = token.indexOf(":");
    const field = token.slice(0, colon), value = token.slice(colon + 1);
    let found: boolean;
    switch (colon > 0 ? field : "") {
      case "from": found = `${m.sender.name} ${m.sender.email}`.toLowerCase().includes(value); break;
      case "to": found = m.to.some(a => `${a.name} ${a.email}`.toLowerCase().includes(value)); break;
      case "subject": found = m.subject.toLowerCase().includes(value); break;
      case "has": found = value === "attachment" && content.attachments.length > 0; break;
      case "is": found = value === "unread" ? m.unread : value === "read" ? !m.unread : value === "starred" && m.starred; break;
      case "before": found = Date.parse(m.received_at) < Date.parse(value); break;
      case "after": found = Date.parse(m.received_at) >= Date.parse(value); break;
      case "in": case "folder": found = m.folder_ids.some(f => f.toLowerCase() === value); break;
      default: found = `${m.subject} ${m.sender.name} ${m.sender.email} ${content.text}`.toLowerCase().includes(token);
    }
    return negative ? !found : found;
  });
}

export class DemoMailbox {
  private state = seed();
  private memoryOnly = false;
  private queue: Promise<unknown> = Promise.resolve();
  readonly listeners = new Set<(state: DemoState) => void>();
  constructor(private persistence: Persistence = browserPersistence(), private now = () => Date.now()) {}
  private async load() {
    if (this.memoryOnly) return;
    try {
      const saved = await this.persistence.read();
      if (compatible(saved)) this.state = saved;
      else { this.state = seed(this.now()); await this.persistence.write(this.state); }
    } catch { this.memoryOnly = true; }
  }
  private async save() {
    this.state.cursor++;
    if (!this.memoryOnly) {
      try { await this.persistence.write(this.state); } catch { this.memoryOnly = true; }
    }
    for (const listener of this.listeners) listener(this.state);
  }
  private serial<T>(work: () => Promise<T>): Promise<T> {
    const run = async () => { await this.load(); return work(); };
    // Shared locks serialize tabs as well as requests within one tab.
    const next: Promise<T> = this.queue.then(async () => {
      if (typeof navigator !== "undefined" && navigator.locks) return await navigator.locks.request(DATABASE, run);
      return await run();
    });
    this.queue = next.catch(() => {});
    return next;
  }
  async snapshot() { return this.serial(async () => structuredClone(this.state)); }
  private advanceState() {
    let changed = false;
    for (const submission of Object.values(this.state.submissions)) {
      if (submission.state !== "scheduled" || Date.parse(submission.due_at) > this.now()) continue;
      const d = this.state.drafts[identity(submission.account_id, submission.draft_id)];
      if (!d) continue;
      const account = this.state.accounts.find(a => a.id === d.account_id)!;
      const id = `sent-${submission.id}`;
      const parent = Object.values(this.state.contents).find(c => c.message.account_id === account.id && c.message_id === d.in_reply_to);
      this.state.contents[identity(account.id, id)] = {
        message: { id, account_id: account.id, provider_id: id, thread_id: parent?.message.thread_id || id, sender: { name: "Alex Morgan", email: account.email }, to: d.to, subject: d.subject, preview: d.text.slice(0, 180), received_at: submission.due_at, folder_ids: [this.state.settings[account.id]?.sent_folder || "Sent"], unread: false, starred: false, category: "People", category_status: "ready", revision: 1 },
        text: d.text, html: d.html, attachments: d.attachments, message_id: `<${id}@northline.example>`, references: d.references, cc: d.cc, reply_to: [], detected_language: "en",
      };
      submission.state = "accepted"; submission.filing_status = "filed"; submission.revision++;
      changed = true;
    }
    for (const [key, snooze] of Object.entries(this.state.snoozed)) {
      if (Date.parse(snooze.until) > this.now()) continue;
      if (this.state.contents[key]?.message.folder_ids.length === 0) this.state.contents[key].message.folder_ids = snooze.folders;
      delete this.state.snoozed[key]; changed = true;
    }
    return changed;
  }
  async advance() { return this.serial(async () => { if (this.advanceState()) await this.save(); }); }
  async attachment(account: string, id: string): Promise<StoredAttachment | undefined> {
    return this.serial(async () => this.state.attachments[identity(account, id)]);
  }
  async fetch(request: Request): Promise<Response> {
    return this.serial(async () => {
      request.signal.throwIfAborted();
      if (this.advanceState()) await this.save();
      const result = await this.route(request);
      return result;
    });
  }
  private async route(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const path = url.pathname;
    const method = request.method;
    const get = method === "GET";
    const s = this.state;
    const body = async <T>() => request.json() as Promise<T>;
    const commit = async (value: unknown) => { await this.save(); return json(value); };
    if (path === "/api/session" && get) return json({ user_id: s.user, csrf_token: "local-only" });
    if (path === "/api/translation/settings" && get) return json({ mode: "host", can_manage: false });
    if (path === "/api/local-access" && get) return json({ can_invite: false });
    if (path === "/api/accounts/gmail" && get) return json({ enabled: true });
    if (path === "/api/accounts/microsoft" && get) return json({ enabled: false });
    if (path === "/api/accounts" && get) return json(s.accounts);
    if (path === "/api/rules") {
      if (get) return json(s.rules);
      if (method === "POST") {
        const input = await body<Pick<MailRule, "preset" | "scope" | "condition" | "actions">>();
        const rule: MailRule = { ...input, id: crypto.randomUUID(), enabled: true, activated_at: new Date(this.now()).toISOString() };
        s.rules.push(rule); return commit(rule);
      }
    }
    const ruleMatch = path.match(/^\/api\/rules\/([^/]+)$/);
    if (ruleMatch && method === "PATCH") {
      const rule = s.rules.find(r => r.id === ruleMatch[1]); if (!rule) return missing();
      rule.enabled = (await body<{enabled: boolean}>()).enabled; return commit(rule);
    }
    if (path === "/api/search" && get) return json(this.page(url));
    const match = path.match(/^\/api\/accounts\/([^/]+)(?:\/(.*))?$/);
    if (!match) return unsupported();
    const account = decodeURIComponent(match[1]);
    if (!s.accounts.some(a => a.id === account)) return missing();
    const parts = (match[2] || "").split("/").map(decodeURIComponent);
    const [resource, id, action] = parts;
    const key = identity(account, id);
    if (resource === "snoozes" && get) {
      const threads = new Map<string, { thread_id: string; until: string; message_count: number }>();
      for (const [key, snooze] of Object.entries(s.snoozed)) {
        const row = s.contents[key]?.message;
        if (!row || row.account_id !== account || Date.parse(snooze.until) <= this.now()) continue;
        const entry = threads.get(row.thread_id) || { thread_id: row.thread_id, until: snooze.until, message_count: 0 };
        entry.message_count++; threads.set(row.thread_id, entry);
      }
      return json([...threads.values()]);
    }
    if (resource === "sync-active" && get) return json(false);
    if (resource === "folders" && get) return json([...new Set([...folders, ...Object.values(s.settings[account] || {}).filter((v): v is string => typeof v === "string")])]);
    if (resource === "recipients" && get) {
      const q = (url.searchParams.get("q") || "").toLowerCase();
      const addresses = Object.values(s.contents).filter(c => c.message.account_id === account).flatMap(c => [c.message.sender, ...c.message.to]);
      return json([...new Map(addresses.filter(a => `${a.name} ${a.email}`.toLowerCase().includes(q)).map(a => [a.email, a])).values()].slice(0, 8));
    }
    if (resource === "settings") {
      if (get) return json(s.settings[account] || defaultSettings);
      if (method === "PUT") { s.settings[account] = await body(); return commit(s.settings[account]); }
    }
    if (resource === "rule-activity" && get) return json([]);
    if (resource === "messages") {
      if (!id && get) return json(this.page(url, account));
      const c = s.contents[key]; if (!c) return missing();
      if (!action && get) return json(c);
      if (action === "conversation" && get) return json({ rows: Object.values(s.contents).filter(item => item.message.account_id === account && item.message.thread_id === c.message.thread_id).map(item => item.message).sort((a, b) => a.received_at.localeCompare(b.received_at)), continuation: null, complete: true });
      if (action === "sender-mail" && get) {
        const cutoff = url.searchParams.get("cutoff") || new Date(this.now()).toISOString();
        const rows = Object.values(s.contents).map(item => item.message).filter(m => m.account_id === account && m.sender.email === c.message.sender.email && !m.folder_ids.includes("Trash") && Date.parse(m.received_at) <= Date.parse(cutoff));
        return json({ rows, count: rows.length, cutoff, after: null });
      }
      if (action === "translation") {
        if (c.detected_language !== "fr") return unsupported();
        if (get) return json({ ...c, text: frenchTranslation, html: paragraphs(frenchTranslation), detected_language: "en" });
        if (method === "POST") {
          const job = { id: crypto.randomUUID(), account_id: account, message_id: id, content_revision: c.message.revision, model_revision: "bundled", kind: "translate" as const, status: "succeeded" as const, deadline: new Date(this.now() + 60000).toISOString() };
          s.jobs[identity(account, job.id)] = job; return commit(job);
        }
      }
      if (action === "unsubscribe") {
        if (c.message.category !== "Newsletters") return json(null);
        const info = { kind: "mailto", to: `unsubscribe@${c.message.sender.email.split("@")[1]}`, subject: "Unsubscribe", body: "Please unsubscribe me.", attempt_status: s.unsubscribed.includes(c.message.sender.email) ? "request_accepted" : null };
        if (get) return json(info);
        if (method === "POST") { if (!s.unsubscribed.includes(c.message.sender.email)) s.unsubscribed.push(c.message.sender.email); return commit({ queued: false }); }
      }
    }
    if (resource === "operations") {
      if (get && id) return s.operations[key] ? json(s.operations[key]) : missing();
      if (method === "POST" && !id) {
        const op = await body<Operation>();
        const opKey = identity(account, op.id);
        if (s.operations[opKey]) return json(s.operations[opKey]);
        const c = s.contents[identity(account, op.target_id)]; if (!c) return missing();
        if (c.message.revision !== op.expected_revision) return fail(409, "This email has changed. Refresh and try again.");
        c.message = apply(c.message, op.change, s);
        if (op.change.kind === "move" && op.change.folder_id.toUpperCase() === "INBOX") {
          for (const [key] of Object.entries(s.snoozed)) {
            const row = s.contents[key]?.message;
            if (row?.account_id === account && row.thread_id === c.message.thread_id) delete s.snoozed[key];
          }
        }
        const result: Operation = { ...op, account_id: account, status: "applied", revision: c.message.revision };
        s.operations[opKey] = result; return commit(result);
      }
    }
    if (resource === "drafts") {
      if (!id && get) return json(Object.values(s.drafts).filter(d => d.account_id === account && !Object.values(s.submissions).some(sub => sub.account_id === account && sub.draft_id === d.id && sub.state !== "cancelled")));
      if (id && get) return s.drafts[key] ? json(s.drafts[key]) : missing();
      if (id && method === "PUT") {
        const d = await body<Draft>();
        if ((s.drafts[key]?.revision || 0) !== d.revision) return fail(409, "This draft has changed. Open the latest version.");
        s.drafts[key] = { ...d, id, account_id: account, revision: d.revision + 1, saved_at: new Date(this.now()).toISOString() };
        return commit(s.drafts[key]);
      }
      if (id && method === "DELETE") { delete s.drafts[key]; return commit(null); }
    }
    if (resource === "submissions" || resource === "scheduled") {
      const rows = Object.values(s.submissions).filter(sub => sub.account_id === account);
      if (!id && get) return json(resource === "scheduled" ? rows.filter(sub => sub.state === "scheduled").map(submission => ({ submission, subject: s.drafts[identity(account, submission.draft_id)]?.subject || "", to: s.drafts[identity(account, submission.draft_id)]?.to || [] })) : rows);
      if (id) {
        const sub = s.submissions[key]; if (!sub) return missing();
        if (get) return json(sub);
        if (method === "POST" && action === "cancel") {
          if (sub.state !== "scheduled") return fail(409, "This message has already been sent.");
          sub.state = "cancelled"; sub.revision++; return commit(sub);
        }
      } else if (method === "POST" && resource === "submissions") {
        const payload = await body<SubmitRequest>();
        const existing = rows.find(sub => sub.operation_id === payload.operation_id); if (existing) return json(existing);
        const draft = s.drafts[identity(account, payload.draft_id)]; if (!draft) return missing();
        if (draft.revision !== payload.expected_revision) return fail(409, "This draft has changed. Open the latest version.");
        if (!draft.to.length) return fail(422, "Add at least one recipient.");
        const sub: Submission = { id: crypto.randomUUID(), account_id: account, operation_id: payload.operation_id, draft_id: draft.id, draft_revision: draft.revision, state: "scheduled", due_at: payload.due_at || new Date(this.now() + 10000).toISOString(), timezone: payload.timezone, filing_status: "pending", revision: 1 };
        s.submissions[identity(account, sub.id)] = sub; return commit(sub);
      }
    }
    if (resource === "attachments") {
      if (!id && method === "POST") {
        const bytes = new Uint8Array(await request.arrayBuffer());
        if (bytes.byteLength > 5 * 1024 * 1024) return fail(413, "Choose an attachment smaller than 5 MB.");
        const meta = { id: crypto.randomUUID(), filename: request.headers.get("x-filename") || "attachment", content_type: request.headers.get("content-type") || "application/octet-stream", size_bytes: bytes.byteLength };
        s.attachments[identity(account, meta.id)] = { meta, bytes }; return commit(meta);
      }
      if (id && get) {
        const attachment = s.attachments[key]; if (!attachment) return missing();
        return new Response(new Uint8Array(attachment.bytes), { headers: { "content-type": attachment.meta.content_type, "content-disposition": `attachment; filename*=UTF-8''${encodeURIComponent(attachment.meta.filename)}` } });
      }
    }
    if (resource === "jobs" && get) return s.jobs[key] ? json(s.jobs[key]) : missing();
    if (resource === "sender-cleanups") {
      if (get && !id) return json(s.cleanups.filter(job => job.account_id === account));
      if (method === "POST" && !id) {
        const input = await body<CleanupRequest>();
        const existing = s.cleanups.find(job => job.account_id === account && job.request.id === input.id); if (existing) return json(existing);
        const source = s.contents[identity(account, input.source_id)]; if (!source) return missing();
        const projected: Message[] = [], failed: string[] = [];
        for (const target of input.targets) {
          const content = s.contents[identity(account, target.id)];
          if (!content || content.message.revision !== target.expected_revision || content.message.sender.email !== source.message.sender.email) { failed.push(target.id); continue; }
          content.message = apply(content.message, { kind: "trash" }, s); projected.push(content.message);
        }
        const job: CleanupJob = { request: input, account_id: account, sender: source.message.sender.email, created_at: new Date(this.now()).toISOString(), dismissed: false, applied: projected.length, failed: failed.length, pending: 0, failed_targets: failed, projected_rows: projected };
        s.cleanups.push(job); return commit(job);
      }
      if (method === "POST" && id && action === "dismiss") {
        const job = s.cleanups.find(job => job.account_id === account && job.request.id === id); if (!job) return missing();
        job.dismissed = true; return commit(job);
      }
    }
    return unsupported();
  }
  private page(url: URL, account?: string): SearchPage {
    const params = url.searchParams;
    const ids = account ? [account] : (params.get("accounts") || this.state.accounts.map(a => a.id).join(",")).split(",");
    let contents = Object.values(this.state.contents).filter(c => ids.includes(c.message.account_id));
    const folder = params.get("folder");
    if (folder) contents = contents.filter(c => folder === "Starred" ? c.message.starred : c.message.folder_ids.includes(folder));
    if (params.get("exclude_trash") === "true") contents = contents.filter(c => !c.message.folder_ids.includes("Trash"));
    if (params.get("category")) contents = contents.filter(c => c.message.category === params.get("category"));
    const filter = { inbox_kind: params.get("inbox_kind") || undefined, email_type: params.get("email_type") || undefined, attention: params.get("attention") === "true" } as MailFilter;
    contents = contents.filter(c => matchesClassification(c.message, filter, this.now()));
    const query = params.get("q") || "";
    const hideSnoozed = /(?:^|\s)(?:folder|in):INBOX(?:\s|$)/i.test(query);
    if (hideSnoozed) contents = contents.filter(c => !this.state.snoozed[identity(c.message.account_id, c.message.id)]);
    const snoozed = /(?:^|\s)is:snoozed(?:\s|$)/.test(query);
    contents = contents.filter(c => (!snoozed || !!this.state.snoozed[identity(c.message.account_id, c.message.id)]) && matches(c, query.replace(/(?:^|\s)is:snoozed(?=\s|$)/g, " ")));
    return { rows: contents.map(c => c.message).sort((a, b) => b.received_at.localeCompare(a.received_at)), continuation: null, coverage: ids.map(account_id => ({ account_id, folder_ids: folder ? [folder] : folders, state: "complete", error: null })), classification: { classified: contents.length, total: contents.length }, pending: false, counts_exact: true };
  }
}

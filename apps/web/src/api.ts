import type {
  Account,
  ApiError,
  Attachment,
  Change,
  ConnectAccount,
  DiscoveredSettings,
  Draft,
  Message,
  MessageContent,
  ConversationPage,
  MailboxSettings,
  RemoteImageRequest,
  Operation,
  SearchPage,
  Session,
  Submission,
  SubmitRequest,
} from "@dakia/api-types";

export class ApiFailure extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message);
  }
}
let csrf = "";
export async function request<T>(
  path: string,
  init: RequestInit = {},
): Promise<T> {
  const headers = new Headers(init.headers);
  if (init.method && init.method !== "GET") headers.set("X-CSRF-Token", csrf);
  if (typeof init.body === "string")
    headers.set("Content-Type", "application/json");
  let response: Response;
  try {
    response = await fetch(`/api${path}`, {
      ...init,
      headers,
      credentials: "same-origin",
      cache: "no-store",
    });
  } catch (error) {
    // Cancellation is intentional, and must not become a connection warning.
    if (init.signal?.aborted || (typeof error === "object" && error !== null && "name" in error && error.name === "AbortError")) throw error;
    // No HTTP response was observed. Preserve the network-error category so
    // lost acknowledgements still use read-only recovery, not rejection logic.
    throw new TypeError("Can’t reach Dakia. Check your internet connection or VPN.");
  }
  if (!response.ok) {
    if (response.status === 401) window.dispatchEvent(new CustomEvent("dakia-auth-expired", { detail: path }));
    const error = (await response.json().catch(() => ({
      code: "http_error",
      message: response.status >= 500
        ? "Can’t reach Dakia. Check your internet connection or VPN."
        : "Dakia couldn’t complete this request. Please try again.",
    }))) as ApiError;
    throw new ApiFailure(response.status, error.code, error.message);
  }
  if (response.status === 204) return undefined as T;
  return response.json() as Promise<T>;
}
export const key = (value: string) => encodeURIComponent(value);
export const accountPath = (id: string) => `/accounts/${key(id)}`;
export type SenderMailPage = { rows: Message[]; count: number; cutoff: string; after: string | null };
export const api = {
  senderMail: (message: Message, signal?: AbortSignal, cutoff?: string, after?: string) =>
    request<SenderMailPage>(`${accountPath(message.account_id)}/messages/${key(message.id)}/sender-mail?${new URLSearchParams({ ...(cutoff ? { cutoff } : {}), ...(after ? { after } : {}) })}`, { signal }),
  session: async () => {
    const s = await request<Session>("/session");
    csrf = s.csrf_token;
    return s;
  },
  logout: async () => {
    await request("/logout", { method: "POST" });
    csrf = "";
  },
  localAccess: () => request<{ can_invite: boolean }>("/local-access"),
  invitations: () => request<{ id: string; expires_at: number }[]>("/invitations"),
  createInvitation: () => request<{ id: string; expires_at: number; url: string }>("/invitations", { method: "POST" }),
  revokeInvitation: (id: string) => request<void>(`/invitations/${key(id)}`, { method: "DELETE" }),
  accounts: () => request<Account[]>("/accounts"),
  syncActive: (id: string, signal?: AbortSignal) => request<boolean>(`${accountPath(id)}/sync-active`, { signal }),
  discover: (email: string, signal?: AbortSignal) => request<DiscoveredSettings | null>("/accounts/discover", { method: "POST", body: JSON.stringify({ email }), signal }),
  removeAccount: (id: string, confirmEmail: string) => request<void>(accountPath(id), { method: "DELETE", body: JSON.stringify({ confirm_email: confirmEmail }) }),
  loadImage: (message: Message, payload: RemoteImageRequest, signal?: AbortSignal) => request<string>(`${accountPath(message.account_id)}/messages/${key(message.id)}/image`, { method: "POST", body: JSON.stringify(payload), signal }),
  mailboxSettings: (id: string) => request<MailboxSettings>(`${accountPath(id)}/settings`),
  configureMailbox: (id: string, settings: MailboxSettings) => request<MailboxSettings>(`${accountPath(id)}/settings`, { method: "PUT", body: JSON.stringify(settings) }),
  connect: (account: ConnectAccount) =>
    request<Account>("/accounts", {
      method: "POST",
      body: JSON.stringify(account),
    }),
  probe: (id: string) =>
    request<Account>(`${accountPath(id)}/probe`, { method: "POST" }),
  reconnect: (id: string, account: ConnectAccount) =>
    request<Account>(accountPath(id), {
      method: "PUT",
      body: JSON.stringify(account),
    }),
  messages: (id: string, folder: string, continuation?: string | null, signal?: AbortSignal) =>
    request<SearchPage>(
      `${accountPath(id)}/messages?${new URLSearchParams({ folder, ...(continuation ? { continuation } : {}) })}`,
      { signal },
    ),
  search: (
    q: string,
    ids: string[],
    continuation?: string | null,
    category?: string,
    signal?: AbortSignal,
    excludeTrash = false,
    refresh = false,
    cacheFirst = false,
  ) =>
    request<SearchPage>(
      `/search?${new URLSearchParams({ q, accounts: ids.join(","), ...(continuation ? { continuation } : {}), ...(category ? { category } : {}), ...(excludeTrash ? { exclude_trash: "true" } : {}), ...(refresh ? { refresh: "true" } : {}), ...(cacheFirst ? { cache_first: "true" } : {}) })}`,
      { signal },
    ),
  content: (m: Message) =>
    request<MessageContent>(
      `${accountPath(m.account_id)}/messages/${key(m.id)}`,
    ),
  contentById: (account: string, id: string, signal?: AbortSignal) =>
    request<MessageContent>(`${accountPath(account)}/messages/${key(id)}`, { signal }),
  conversation: (m: Message, continuation?: string | null, signal?: AbortSignal, mode?: "snapshot" | "refresh") => {
    const params = new URLSearchParams();
    if (continuation) params.set("continuation", continuation);
    if (mode) params.set(mode, "true");
    return request<ConversationPage>(`${accountPath(m.account_id)}/messages/${key(m.id)}/conversation${params.size ? `?${params}` : ""}`, { signal });
  },
  operate: (op: Operation) =>
    request<Operation>(`${accountPath(op.account_id)}/operations`, {
      method: "POST",
      body: JSON.stringify(op),
    }),
  operationStatus: (op: Operation) =>
    request<Operation>(`${accountPath(op.account_id)}/operations/${key(op.id)}`),
  draft: (account: string, id: string) =>
    request<Draft>(`${accountPath(account)}/drafts/${key(id)}`),
  saveDraft: (d: Draft) =>
    request<Draft>(`${accountPath(d.account_id)}/drafts/${key(d.id)}`, {
      method: "PUT",
      body: JSON.stringify(d),
    }),
  submit: (account: string, payload: SubmitRequest) =>
    request<Submission>(`${accountPath(account)}/submissions`, {
      method: "POST",
      body: JSON.stringify(payload),
    }),
  submission: (s: Submission) =>
    request<Submission>(
      `${accountPath(s.account_id)}/submissions/${key(s.id)}`,
    ),
  cancelSubmission: (s: Submission) =>
    request<Submission>(
      `${accountPath(s.account_id)}/submissions/${key(s.id)}/cancel`,
      { method: "POST" },
    ),
  upload: (account: string, file: File) =>
    request<Attachment>(`${accountPath(account)}/attachments`, {
      method: "POST",
      headers: {
        "Content-Type": file.type || "application/octet-stream",
        "x-filename": file.name,
      },
      body: file,
    }),
  download: (account: string, id: string) =>
    `/api${accountPath(account)}/attachments/${key(id)}`,
};
export function optimistic(message: Message, change: Change): Message {
  switch (change.kind) {
    case "read":
      return { ...message, unread: !change.value };
    case "star":
      return { ...message, starred: change.value };
    case "category":
      return { ...message, category: change.value, category_status: "ready" };
    case "archive":
      return {
        ...message,
        folder_ids: message.folder_ids.filter(
          (f) => f.toUpperCase() !== "INBOX" && f !== "\\Inbox",
        ),
      };
    case "trash":
      return { ...message, folder_ids: ["Trash"] };
    case "spam":
      // Do not invent a provider destination while the operation is pending.
      return { ...message, folder_ids: message.folder_ids.filter(f => f.toUpperCase() !== "INBOX" && f !== "\\Inbox") };
    case "move":
      return { ...message, folder_ids: [change.folder_id] };
    default:
      return message;
  }
}
export function actionLabel(change: Change): string {
  switch (change.kind) {
    case "read": return change.value ? "Mark as read" : "Mark as unread";
    case "star": return change.value ? "Star" : "Unstar";
    case "archive": return "Archive";
    case "trash": return "Move to trash";
    case "spam": return "Mark as spam";
    case "move": return "Move";
    case "category": return "Change category";
    case "snooze": return "Snooze";
  }
}
// Serialize each target's mutations. An older failure never restores stale state over a newer intent.
export class OperationQueue {
  private journalKey = "";
  private journal = new Map<string, Operation>();
  private chains = new Map<string, Promise<void>>();
  private revisions = new Map<string, number>();
  private latest = new Map<string, string>();
  restore(user: string): Operation[] {
    this.journalKey = `dakia-pending-operations:${user}`;
    try {
      const saved: Operation[] = JSON.parse(sessionStorage.getItem(this.journalKey) || "[]");
      this.journal = new Map(saved.filter(op => op && typeof op.id === "string" && typeof op.account_id === "string" && typeof op.target_id === "string" && op.change).map(op => [op.id, op]));
    } catch { this.journal.clear(); }
    return [...this.journal.values()];
  }
  private remember(op: Operation, remove = false) {
    if (remove) this.journal.delete(op.id);
    else this.journal.set(op.id, op);
    // Only operation identifiers/intents, never message bodies or credentials.
    // Storage denial must not prevent mail operations from completing.
    try { if (this.journalKey) sessionStorage.setItem(this.journalKey, JSON.stringify([...this.journal.values()])); } catch { /* private browsing/quota */ }
  }
  async resume(op: Operation): Promise<void> {
    // Read-only recovery. Never POST/replay after a lost acknowledgement.
    let result: Operation;
    try { result = await api.operationStatus(op); }
    catch (error) {
      if (error instanceof ApiFailure && error.status === 404) this.remember(op, true);
      throw error;
    }
    await this.outcome(op, result);
  }
  private async outcome(op: Operation, initial: Operation, subject?: string) {
    let result = initial;
    while (result.status === "pending") {
      await new Promise(resolve => setTimeout(resolve, 1000));
      try { result = await api.operationStatus(op); }
      catch {
        // A failed read does not say whether provider work was applied.
        // Leave the identifier journaled and never turn this into a POST retry.
        throw new Error(`${actionLabel(op.change)} could not be verified for ${subject ? `“${subject}”` : "a pending message"}. Refresh mail and check its current state before trying again.`);
      }
    }
    this.remember(op, true);
    this.revisions.set(`${op.account_id}/${op.target_id}`, result.revision);
    if (result.status === "failed") {
      throw new Error(`${actionLabel(op.change)} could not be verified for ${subject ? `“${subject}”` : "a pending message"}. ${op.change.kind === "spam"
        ? "Refresh and check Junk before trying again."
        : "Refresh mail and check its current state before trying again."}`);
    }
  }
  enqueue(
    message: Message,
    change: Change,
    update: (id: string, value: Message) => void,
    invalidate: () => void,
  ): Promise<void> {
    const target = `${message.account_id}/${message.id}`;
    const id = crypto.randomUUID();
    this.latest.set(target, id);
    update(target, optimistic(message, change));
    const run = (this.chains.get(target) || Promise.resolve())
      .catch(() => {})
      .then(async () => {
        const op: Operation = {
          id,
          account_id: message.account_id,
          target_id: message.id,
          expected_revision: Math.max(
            message.revision,
            this.revisions.get(target) || 0,
          ),
          change,
          status: "pending",
          revision: 0,
        };
        this.remember(op);
        let result: Operation;
        try { result = await api.operate(op); }
        catch (error) {
          // A known 4xx rejection was not queued. Network/5xx outcomes may
          // have been accepted, so retain the ID for read-only recovery.
          if (error instanceof ApiFailure && error.status >= 400 && error.status < 500) {
            this.remember(op, true);
            throw error;
          }
          throw new Error(`${actionLabel(change)} could not be verified for “${message.subject?.slice(0, 120) || "(no subject)"}”. ${error instanceof ApiFailure ? `${error.message} ` : ""}Refresh mail and check its current state before trying again.`);
        }
        // Acceptance only queues provider work. Keep the optimistic state and
        // serialize subsequent target actions until the durable outcome is known.
        await this.outcome(op, result, message.subject?.slice(0, 120) || "(no subject)");
      })
      .finally(() => {
        if (this.latest.get(target) === id) {
          this.chains.delete(target);
          invalidate();
        }
      });
    this.chains.set(target, run);
    return run;
  }
}

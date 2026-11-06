import type { Account, Message, MessageContent, SearchPage } from "@dakia/api-types";
import type { InfiniteData, QueryClient, QueryKey } from "@tanstack/react-query";

const DB = "dakia-local-mail-v1";
const LIMIT = 64 * 1024 * 1024;
const AGE = 7 * 86400000;
const MAX_ENTRIES = 2000;
type Entry = { key: string; owner: string; accounts: string[]; kind: "folder" | "body"; query: QueryKey; saved: number; bytes: number; json: string };
const bodyKey = (account: string, id: string) => JSON.stringify(["content", account, id]);
const request = <T>(r: IDBRequest<T>) => new Promise<T>((resolve, reject) => { r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
const completed = (t: IDBTransaction) => {
  const done = new Promise<void>((resolve, reject) => { t.oncomplete = () => resolve(); t.onerror = t.onabort = () => reject(t.error); });
  // A failed individual request can reject before the transaction completes.
  // Keep both failures handled even if the request exits the surrounding block.
  void done.catch(() => {});
  return done;
};

const address = (value: unknown): boolean => {
  const a = value as { email?: unknown; name?: unknown } | null;
  return !!a && typeof a.email === "string" && (a.name == null || typeof a.name === "string");
};
function message(value: unknown): value is Message {
  const m = value as Message | null;
  return !!m && [m.id, m.account_id, m.thread_id, m.provider_id, m.subject, m.preview, m.received_at, m.category_status].every(v => typeof v === "string") &&
    Number.isFinite(Date.parse(m.received_at)) && Number.isFinite(m.revision) && typeof m.unread === "boolean" && typeof m.starred === "boolean" &&
    (m.category == null || typeof m.category === "string") && Array.isArray(m.folder_ids) && m.folder_ids.every(f => typeof f === "string") && address(m.sender) && Array.isArray(m.to) && m.to.every(address);
}

export class LocalMailCache {
  private db?: Promise<IDBDatabase>;
  private owner = "";
  private epoch = -1;
  private accounts = new Set<string>();
  private client?: QueryClient;
  private unsubscribe?: () => void;
  private pending = new Map<string, Entry>();
  private queued = new Map<string, { query: QueryKey; data: unknown }>();
  private timer?: ReturnType<typeof setTimeout>;
  private serial: Promise<unknown> = Promise.resolve();
  private generation = 0;
  private authorizedOwner = "";
  private authorization = 0;
  ticket() { return this.authorization; }
  private channel?: BroadcastChannel;
  private localBodies = new Set<string>();
  private fingerprint = "";
  verifiedBody(account: string, id: string) { this.localBodies.delete(bodyKey(account, id)); }
  private blockedFolders = new Map<string, number>();
  constructor(broadcast = true) {
    if (broadcast && typeof window !== "undefined") {
      window.addEventListener("pagehide", () => { void this.flush(); });
      document.addEventListener("visibilitychange", () => { if (document.visibilityState === "hidden") void this.flush(); });
    }
    if (broadcast && typeof indexedDB !== "undefined" && typeof BroadcastChannel !== "undefined" && typeof window !== "undefined") {
      this.channel = new BroadcastChannel("dakia-local-mail");
      this.channel.onmessage = () => {
        this.resetMemory();
        // Other tabs must reauthorize before displaying or saving private mail.
        window.location.reload();
      };
    }
  }
  private open() {
    if (!this.db) this.db = new Promise<IDBDatabase>((resolve, reject) => {
      const r = indexedDB.open(DB, 1);
      r.onupgradeneeded = () => {
        r.result.createObjectStore("entries", { keyPath: "key" });
        r.result.createObjectStore("meta");
        r.result.createObjectStore("sizes", { keyPath: "key" });
      };
      r.onsuccess = () => { r.result.onversionchange = () => r.result.close(); resolve(r.result); };
      r.onerror = () => reject(r.error);
      r.onblocked = () => reject(new Error("Browser storage unavailable"));
    });
    return this.db;
  }
  private resetMemory() {
    this.generation++; this.authorization++; this.authorizedOwner = "";
    this.unsubscribe?.(); this.unsubscribe = undefined;
    clearTimeout(this.timer); this.timer = undefined; this.pending.clear(); this.queued.clear();
    this.owner = ""; this.accounts.clear(); this.localBodies.clear(); this.fingerprint = ""; this.blockedFolders.clear(); this.epoch = -1;
    this.client?.removeQueries({ predicate: q => !["session"].includes(String(q.queryKey[0])) });
  }
  async identify(owner: string, client: QueryClient, ticket = this.authorization) {
    if (ticket !== this.authorization) return false;
    if (this.owner && this.owner !== owner) {
      await this.clear();
      if (this.authorization !== ticket + 1) return false;
    }
    this.client = client; this.authorizedOwner = owner;
    return true;
  }
  async activate(owner: string, accounts: Account[], client: QueryClient, ticket: number) {
    if (this.authorizedOwner !== owner || ticket !== this.authorization) return;
    const ids = new Set(accounts.map(a => a.id));
    const fingerprint = JSON.stringify(accounts.map(a => [a.id, a.email, a.provider]).sort());
    if (this.owner === owner && this.fingerprint === fingerprint) return;
    this.unsubscribe?.(); this.pending.clear(); this.queued.clear(); clearTimeout(this.timer); this.timer = undefined;
    const changedIdentityInMemory = this.owner === owner && this.fingerprint !== fingerprint;
    if (this.owner === owner) client.removeQueries({ predicate: q =>
      (changedIdentityInMemory && ["content", "mail"].includes(String(q.queryKey[0]))) ||
      (q.queryKey[0] === "content" && !ids.has(String(q.queryKey[1]))) ||
      (q.queryKey[0] === "mail" && Array.isArray(q.queryKey[5]) && q.queryKey[5].some(id => !ids.has(id))) });
    this.owner = owner; this.accounts = ids; this.client = client; this.fingerprint = fingerprint;
    const generation = ++this.generation;
    try {
      const db = await this.open();
      const tx = db.transaction(["entries", "meta", "sizes"], "readwrite"), done = completed(tx);
      const entries = tx.objectStore("entries");
      const epoch = await request(tx.objectStore("meta").get("epoch")) as number | undefined;
      const prior = await request(tx.objectStore("meta").get("accounts")) as { owner: string; fingerprint: string } | undefined;
      const changedIdentity = prior?.owner === owner && prior.fingerprint !== fingerprint;
      tx.objectStore("meta").put({ owner, fingerprint }, "accounts");
      const all = await request(tx.objectStore("sizes").getAll()) as Entry[];
      const folders: Entry[] = [];
      for (const entry of all) {
        if (changedIdentity || entry.owner !== owner || entry.saved < Date.now() - AGE || !entry.accounts.every(id => ids.has(id))) {
          entries.delete(entry.key); tx.objectStore("sizes").delete(entry.key);
        } else if (entry.kind === "folder") folders.push(await request(entries.get(entry.key)) as Entry);
      }
      await done;
      if (generation !== this.generation) return;
      this.epoch = epoch || 0;
      // Folder headers hydrate first. Bodies stay on disk until visible/opened.
      for (const entry of folders) {
        try {
          const data = JSON.parse(entry.json) as InfiniteData<SearchPage>;
          const first = data.pages?.[0];
          if (entry.query[0] !== "mail" || entry.query[3] !== "" || !Array.isArray(first?.rows) || !Array.isArray(first.coverage) || !Number.isFinite(first.classification?.classified) || !Number.isFinite(first.classification?.total) || !first.rows.every(m => message(m) && ids.has(m.account_id))) continue;
          if (!client.getQueryData(entry.query)) client.setQueryData(entry.query, data, { updatedAt: 1 });
        } catch { /* Ignore a corrupt record without losing other folders. */ }
      }
    } catch { /* Storage denial, corruption or quota must not prevent online use. */ }
    if (generation !== this.generation) return;
    this.unsubscribe = client.getQueryCache().subscribe(event => {
      const query = event.query;
      if (query.queryKey[0] !== "content" && !(query.queryKey[0] === "mail" && query.queryKey[3] === "")) return;
      if (event.type !== "updated" || event.action.type !== "success") return;
      this.queued.set(JSON.stringify(query.queryKey), { query: query.queryKey, data: query.state.data });
      if (!this.timer) this.timer = setTimeout(() => { this.timer = undefined; void this.flush(); }, 200);
    });
  }
  private save(query: QueryKey, data: unknown) {
    if (!this.owner || !data) return;
    let payload = data, kind: Entry["kind"], accounts: string[];
    if (query[0] === "content" && typeof query[1] === "string" && typeof query[2] === "string") {
      kind = "body"; accounts = [query[1]];
      if (this.localBodies.has(bodyKey(query[1], query[2]))) return;
      if (((data as MessageContent).html?.length || 0) + ((data as MessageContent).text?.length || 0) > 2 * 1024 * 1024) return;
      if ((data as MessageContent).message?.account_id !== query[1] || (data as MessageContent).message?.id !== query[2]) return;
    } else if (query[0] === "mail" && query[3] === "" && Array.isArray(query[5])) {
      kind = "folder"; accounts = query[5] as string[];
      if (accounts.some(id => this.blockedFolders.has(id))) return;
      const pages = (data as InfiniteData<SearchPage>).pages;
      if (!pages?.length) return;
      // Never persist provider cursors or claims of current coverage. A saved
      // first page is a provisional view; the live first page replaces it.
      const first = pages[0];
      payload = { pages: [{ ...first, rows: first.rows.slice(0, 200), continuation: null, pending: false, counts_exact: false,
        coverage: first.coverage.map(c => ({ ...c, state: "pending" })), cache_preview: false }], pageParams: [null] };
    } else return;
    if (!accounts.length || !accounts.every(id => this.accounts.has(id))) return;
    try {
      const json = JSON.stringify(payload), bytes = json.length * 2;
      if (bytes > 4 * 1024 * 1024) return;
      const key = JSON.stringify([this.owner, query]);
      this.pending.set(key, { key, owner: this.owner, accounts, kind, query, saved: Date.now(), bytes, json });
    } catch { /* Invalid or nonserializable responses are not cacheable. */ }
  }
  flush(): Promise<void> {
    clearTimeout(this.timer); this.timer = undefined;
    const queued = [...this.queued.values()]; this.queued.clear();
    for (const { query, data } of queued) this.save(query, data);
    const entries = [...this.pending.values()]; this.pending.clear();
    const epoch = this.epoch, generation = this.generation;
    const work = async () => {
      if (!entries.length || epoch < 0 || generation !== this.generation) return;
      try {
        const db = await this.open();
        const tx = db.transaction(["entries", "meta", "sizes"], "readwrite"), done = completed(tx);
        if (((await request(tx.objectStore("meta").get("epoch"))) || 0) !== epoch) { await done; return; }
        const store = tx.objectStore("entries");
        for (const entry of entries) { store.put(entry); const { json: _json, ...size } = entry; tx.objectStore("sizes").put(size); }
        const all = (await request(tx.objectStore("sizes").getAll()) as Entry[]).sort((a, b) => b.saved - a.saved);
        let bytes = 0, count = 0, folders = 0;
        for (const entry of all) {
          if (entry.saved < Date.now() - AGE || (entry.kind === "folder" && ++folders > 32) || ++count > MAX_ENTRIES || (bytes += entry.bytes) > LIMIT) { store.delete(entry.key); tx.objectStore("sizes").delete(entry.key); }
        }
        await done;
      } catch { /* Quota/storage failures leave the in-memory reader usable. */ }
    };
    this.serial = this.serial.then(work, work);
    return this.serial as Promise<void>;
  }
  async body(account: string, id: string): Promise<MessageContent | undefined> {
    const generation = this.generation;
    if (!this.owner || !this.accounts.has(account)) return;
    try {
      const db = await this.open();
      const tx = db.transaction(["entries", "meta"], "readonly");
      const epoch = await request(tx.objectStore("meta").get("epoch"));
      if ((epoch || 0) !== this.epoch) return;
      const entry = await request(tx.objectStore("entries").get(JSON.stringify([this.owner, JSON.parse(bodyKey(account, id))]))) as Entry | undefined;
      if (generation !== this.generation || !entry || entry.saved < Date.now() - AGE || entry.bytes > 4 * 1024 * 1024) return;
      const data = JSON.parse(entry.json) as MessageContent;
      if (message(data.message) && data.message.id === id && data.message.account_id === account && typeof data.html === "string" && typeof data.text === "string" && Array.isArray(data.attachments) && data.attachments.every(a => typeof a.id === "string" && typeof a.filename === "string" && typeof a.content_type === "string" && Number.isFinite(a.size_bytes)) && Array.isArray(data.reply_to) && data.reply_to.every(address) && Array.isArray(data.cc) && data.cc.every(address) && Array.isArray(data.references) && data.references.every(r => typeof r === "string")) { this.localBodies.add(bodyKey(account, id)); return data; }
    } catch { /* Use the network when local storage is unavailable. */ }
  }
  async invalidateFolders(accounts: string[]) {
    for (const id of accounts) this.blockedFolders.set(id, (this.blockedFolders.get(id) || 0) + 1);
    const generation = ++this.generation;
    for (const [key, entry] of this.queued) if (entry.query[0] === "mail") this.queued.delete(key);
    for (const [key, entry] of this.pending) if (entry.kind === "folder" && entry.accounts.some(id => accounts.includes(id))) this.pending.delete(key);
    try {
      const db = await this.open();
      const tx = db.transaction(["entries", "sizes", "meta"], "readwrite"), done = completed(tx);
      const epoch = ((await request(tx.objectStore("meta").get("epoch"))) || 0) + 1;
      tx.objectStore("meta").put(epoch, "epoch");
      const entries = await request(tx.objectStore("sizes").getAll()) as Entry[];
      for (const entry of entries) if (entry.kind === "folder" && entry.accounts.some(id => accounts.includes(id))) {
        tx.objectStore("entries").delete(entry.key); tx.objectStore("sizes").delete(entry.key);
      }
      await done;
      if (generation === this.generation) this.epoch = epoch;
    } catch { /* A mail action must remain usable without browser storage. */ }
  }
  resumeFolders(accounts: string[], queryKey?: QueryKey, started = Infinity) {
    for (const id of accounts) { const count = this.blockedFolders.get(id) || 0; if (count > 1) this.blockedFolders.set(id, count - 1); else this.blockedFolders.delete(id); }
    const query = queryKey && this.client?.getQueryCache().find({ queryKey, exact: true });
    if (accounts.length && query && query.state.status === "success" && query.state.dataUpdatedAt >= started && !query.state.isInvalidated) {
      this.save(query.queryKey, query.state.data); void this.flush();
    }
  }
  async clear() {
    let announce = !!(this.owner || this.authorizedOwner);
    this.resetMemory();
    try {
      const db = await this.open();
      const tx = db.transaction(["entries", "meta", "sizes"], "readwrite"), done = completed(tx);
      announce ||= !!(await request(tx.objectStore("meta").get("accounts")));
      const epoch = ((await request(tx.objectStore("meta").get("epoch"))) || 0) + 1;
      tx.objectStore("entries").clear(); tx.objectStore("sizes").clear(); tx.objectStore("meta").delete("accounts"); tx.objectStore("meta").put(epoch, "epoch");
      await done;
    } catch { /* Browser may have disabled or removed its storage. */ }
    if (announce) this.channel?.postMessage("clear");
  }
}
export const localMailCache = new LocalMailCache();

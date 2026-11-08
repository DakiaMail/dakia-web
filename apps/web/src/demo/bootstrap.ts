import { setLocalTransport } from "../localTransport";
import { DemoMailbox, DATABASE } from "./mailbox";
import { identity, type DemoState } from "./seed";

async function reset() {
  // This runs only in the isolated demo build, never on a real mailbox origin.
  try { localStorage.clear(); } catch { /* Storage can be unavailable. */ }
  try { sessionStorage.clear(); } catch { /* Storage can be unavailable. */ }
  try {
    const databases = await indexedDB.databases();
    await Promise.all(databases.filter(db => db.name).map(db => new Promise<void>(resolve => {
      const request = indexedDB.deleteDatabase(db.name!);
      request.onsuccess = request.onerror = () => resolve();
      request.onblocked = () => resolve(); // Existing tabs close on versionchange.
    })));
  } catch {
    try { indexedDB.deleteDatabase(DATABASE); } catch { /* Memory-only fallback. */ }
  }
  try { await Promise.all((await caches.keys()).map(key => caches.delete(key))); } catch { /* No Cache Storage. */ }
}

export async function installDemo() {
  const url = new URL(location.href);
  if (url.searchParams.get("reset") === "1") {
    await reset(); url.searchParams.delete("reset");
    history.replaceState(null, "", url.pathname + url.search + url.hash);
  }
  const mailbox = new DemoMailbox();
  const downloads = new Map<string, string>();
  const updateDownloads = (state: DemoState) => {
    for (const [key, attachment] of Object.entries(state.attachments)) {
      if (!downloads.has(key)) downloads.set(key, URL.createObjectURL(new Blob([new Uint8Array(attachment.bytes)], { type: attachment.meta.content_type })));
    }
  };
  updateDownloads(await mailbox.snapshot());
  mailbox.listeners.add(updateDownloads);
  setLocalTransport({ download: (account, id) => downloads.get(identity(account, id)) || "about:blank" });

  const networkFetch = window.fetch.bind(window);
  window.fetch = async (input, init) => {
    const request = new Request(input instanceof Request ? input : new URL(String(input), location.origin), init);
    const target = new URL(request.url);
    request.signal.throwIfAborted();
    if (target.origin === location.origin && target.pathname.startsWith("/api/")) return mailbox.fetch(request);
    // Only the existing app release check needs HTTP. Everything else fails
    // closed, including future unimplemented API calls and remote images.
    if (target.origin === location.origin && target.pathname === "/app-version.json" && request.method === "GET") return networkFetch(request);
    return Response.json({ code: "unavailable", message: "This action is unavailable." }, { status: 403 });
  };
  const sources = new Set<LocalEvents>();
  class LocalEvents extends EventTarget {
    static readonly CONNECTING = 0; static readonly OPEN = 1; static readonly CLOSED = 2;
    readonly CONNECTING = 0; readonly OPEN = 1; readonly CLOSED = 2;
    readonly withCredentials = false;
    readyState = 0;
    onmessage: ((event: MessageEvent) => void) | null = null;
    onerror: ((event: Event) => void) | null = null;
    onopen: ((event: Event) => void) | null = null;
    readonly url: string;
    private account: string | undefined;
    constructor(value: string | URL) {
      super(); this.url = new URL(value, location.origin).href;
      const target = new URL(this.url);
      const match = target.origin === location.origin && target.pathname.match(/^\/api\/accounts\/([^/]+)\/events$/);
      this.account = match ? decodeURIComponent(match[1]) : undefined;
      sources.add(this);
      void mailbox.snapshot().then(state => {
        if (this.readyState === 2) return;
        if (!this.account || !state.accounts.some(a => a.id === this.account)) { this.close(); this.onerror?.(new Event("error")); return; }
        this.readyState = 1;
        this.onopen?.(new Event("open"));
        this.dispatchEvent(new MessageEvent("ready", { data: JSON.stringify({ cursor: state.cursor, reset: true }) }));
      });
    }
    changed(state: DemoState) {
      if (this.readyState !== 1) return;
      const event = new MessageEvent("message", { data: JSON.stringify({ account_id: this.account, cursor: state.cursor, kind: "reset", target_id: "" }) });
      this.onmessage?.(event); this.dispatchEvent(event);
    }
    close() { this.readyState = 2; sources.delete(this); }
  }
  window.EventSource = LocalEvents as unknown as typeof EventSource;
  const channel = typeof BroadcastChannel === "undefined" ? undefined : new BroadcastChannel(DATABASE);
  mailbox.listeners.add(state => { sources.forEach(source => source.changed(state)); channel?.postMessage("changed"); });
  if (channel) channel.onmessage = () => { void mailbox.snapshot().then(state => { updateDownloads(state); sources.forEach(source => source.changed(state)); }); };
  const timer = setInterval(() => void mailbox.advance(), 1000);
  window.addEventListener("pagehide", event => { if (!event.persisted) { clearInterval(timer); channel?.close(); sources.forEach(source => source.close()); } });
}

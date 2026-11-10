import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from "react";

const ReloadGuard = createContext<(id: symbol) => () => void>(() => () => {});
const BrowserUpdateVisible = createContext(false);
export const useBrowserUpdateVisible = () => useContext(BrowserUpdateVisible);
export function useReloadGuard() {
  const block = useContext(ReloadGuard);
  useEffect(() => block(Symbol()), [block]);
}
const POLL_MS = 5 * 60_000;
const MIN_CHECK_MS = 60_000;
async function release(signal: AbortSignal): Promise<string> {
  const response = await fetch("/app-version.json", { cache: "no-store", credentials: "omit", signal });
  if (!response.ok) throw new Error("version unavailable");
  const value: unknown = await response.json();
  if (!value || typeof value !== "object" || !("version" in value) || typeof value.version !== "string" || !/^[a-zA-Z0-9_-]{1,100}$/.test(value.version)) throw new Error("invalid version");
  return value.version;
}

export function AppUpdateProvider({ children, enabled = import.meta.env.PROD, buildId = import.meta.env.VITE_DAKIA_BUILD_ID, reload = () => window.location.reload() }: {
  children: ReactNode; enabled?: boolean; buildId?: string; reload?: () => void;
}) {
  const [available, setAvailable] = useState<string | null>(null);
  const [dismissed, setDismissed] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [blocked, setBlocked] = useState(false);
  const guards = useRef(new Set<symbol>());
  const registration = useRef<ServiceWorkerRegistration | null>(null);
  const active = useRef(true);
  const activationCleanup = useRef<(() => void) | null>(null);
  const block = useCallback((id: symbol) => {
    guards.current.add(id); setBlocked(true);
    return () => { guards.current.delete(id); setBlocked(guards.current.size > 0); };
  }, []);
  useEffect(() => {
    if (!enabled || !buildId) return;
    active.current = true;
    let stopped = false, inFlight = false, lastCheck = -Infinity;
    const requests = new Set<AbortController>();
    if ("serviceWorker" in navigator) {
      void navigator.serviceWorker.register("/sw.js", { updateViaCache: "none" }).then(value => {
        if (!stopped) registration.current = value;
      }).catch(() => {});
    }
    const check = async () => {
      if (stopped || inFlight || !navigator.onLine || document.visibilityState === "hidden" || Date.now() - lastCheck < MIN_CHECK_MS) return;
      lastCheck = Date.now(); inFlight = true;
      const controller = new AbortController(); requests.add(controller);
      const timeout = setTimeout(() => controller.abort(), 10_000);
      try {
        const version = await release(controller.signal);
        if (!stopped) setAvailable(version === buildId ? null : version);
        if (registration.current) void registration.current.update().catch(() => {});
      } catch { /* An update check must not turn a network interruption into an app error. */ }
      finally { clearTimeout(timeout); requests.delete(controller); inFlight = false; }
    };
    void check();
    const timer = setInterval(() => void check(), POLL_MS);
    const resume = () => void check();
    window.addEventListener("online", resume);
    window.addEventListener("visibilitychange", resume);
    return () => {
      stopped = true; active.current = false;
      clearInterval(timer); requests.forEach(c => c.abort());
      window.removeEventListener("online", resume);
      window.removeEventListener("visibilitychange", resume);
      activationCleanup.current?.();
    };
  }, [enabled, buildId]);

  async function apply() {
    if (busy || guards.current.size) return;
    setBusy(true); setError("");
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), 10_000);
    try {
      // Confirm connectivity before replacing the currently usable app.
      const version = await release(controller.signal);
      if (!active.current || guards.current.size) return;
      if (version === buildId) { setAvailable(null); return; }
      const waiting = registration.current?.waiting;
      if (waiting && "serviceWorker" in navigator) {
        await new Promise<void>((resolve, reject) => {
          const changed = () => { cleanup(); resolve(); };
          const timer = setTimeout(() => { cleanup(); reject(new Error("activation timed out")); }, 5000);
          const cleanup = () => { clearTimeout(timer); navigator.serviceWorker.removeEventListener("controllerchange", changed); activationCleanup.current = null; };
          activationCleanup.current = () => { cleanup(); reject(new Error("unmounted")); };
          navigator.serviceWorker.addEventListener("controllerchange", changed);
          waiting.postMessage({ type: "DAKIA_SKIP_WAITING" });
        });
      }
      if (active.current && !guards.current.size) { setBusy(false); reload(); }
    } catch {
      if (active.current) setError("Couldn’t load the update. Check your connection and try again.");
    } finally { clearTimeout(timeout); if (active.current) setBusy(false); }
  }
  return <ReloadGuard.Provider value={block}><BrowserUpdateVisible.Provider value={!!available && available !== dismissed}>
    {children}
    {available && available !== dismissed && <aside className="app-update" aria-label="App update">
      <p role="status">{error || "New version available"}</p>
      {blocked && <p>Close the open dialog to reload.</p>}
      <div><button title="Reload" className="primary" disabled={busy || blocked} onClick={() => void apply()}>{busy ? "Loading…" : "Reload"}</button>
        <button title="Later" disabled={busy} onClick={() => { setDismissed(available); setError(""); }}>Later</button></div>
    </aside>}
  </BrowserUpdateVisible.Provider></ReloadGuard.Provider>;
}

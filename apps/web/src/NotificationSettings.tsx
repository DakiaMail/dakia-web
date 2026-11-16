import { useCallback, useEffect, useRef, useState } from "react";
import { Bell } from "lucide-react";
import { extraApi } from "./extra-api";
import { ApiFailure } from "./api";
import { localTransport } from "./localTransport";

async function subscriptionId(endpoint: string) {
  const hash = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(endpoint));
  return Array.from(new Uint8Array(hash), byte => byte.toString(16).padStart(2, "0")).join("");
}
async function pushRegistration() {
  const registration = await navigator.serviceWorker.getRegistration();
  if (!registration?.active) throw new Error("Reload Dakia, then try again.");
  return registration;
}
export function NotificationSettings() {
  const supported = !localTransport && "serviceWorker" in navigator && "PushManager" in window && "Notification" in window;
  const busyRef = useRef(false);
  const generation = useRef(0);
  const [publicKey, setPublicKey] = useState("");
  const [enabled, setEnabled] = useState(false);
  const [permission, setPermission] = useState(supported ? Notification.permission : "default");
  const [busy, setBusy] = useState(false);
  const [checking, setChecking] = useState(supported);
  const [error, setError] = useState("");
  const [status, setStatus] = useState("");
  const refresh = useCallback(async () => {
    if (!supported || busyRef.current) return;
    const current = ++generation.current;
    setChecking(true);
    try {
      const permission = Notification.permission;
      const registration = await navigator.serviceWorker.getRegistration();
      const subscription = await registration?.pushManager.getSubscription();
      const enabled = !!subscription && await extraApi.pushStatus(await subscriptionId(subscription.endpoint));
      if (current !== generation.current) return;
      setPermission(permission); setEnabled(enabled);
      const key = await extraApi.pushKey();
      if (current !== generation.current) return;
      setPublicKey(key); setError("");
    } catch (error) {
      if (current !== generation.current) return;
      setPublicKey("");
      setError(error instanceof ApiFailure && error.code === "push_not_configured"
        ? "Notifications aren’t set up on this Dakia server yet."
        : error instanceof Error ? error.message : "Couldn’t check notifications. Try again.");
    } finally { if (current === generation.current) setChecking(false); }
  }, [supported]);
  useEffect(() => {
    void refresh();
    const resume = () => { if (document.visibilityState === "visible") void refresh(); };
    window.addEventListener("focus", resume);
    document.addEventListener("visibilitychange", resume);
    return () => { generation.current++; window.removeEventListener("focus", resume); document.removeEventListener("visibilitychange", resume); };
  }, [refresh]);
  async function run(action: "enable" | "disable" | "test") {
    if (busyRef.current) return;
    const trigger = document.activeElement as HTMLElement | null;
    busyRef.current = true; generation.current++;
    setBusy(true); setError(""); setStatus("");
    try {
      if (action === "enable") {
        // Permission must be requested directly from the user's click on iOS.
        const result = await Notification.requestPermission();
        setPermission(result);
        if (result !== "granted") throw new Error("Allow notifications in your browser or device settings, then return to Dakia.");
        const registration = await pushRegistration();
        const padded = publicKey.replace(/-/g, "+").replace(/_/g, "/");
        const applicationServerKey = Uint8Array.from(atob(padded + "=".repeat((4 - padded.length % 4) % 4)), c => c.charCodeAt(0));
        const subscription = await registration.pushManager.getSubscription() || await registration.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey });
        await extraApi.subscribe(subscription.toJSON());
        setEnabled(true);
      } else if (action === "disable") {
        const registration = await pushRegistration();
        const subscription = await registration.pushManager.getSubscription();
        if (subscription) {
          await extraApi.disablePush(await subscriptionId(subscription.endpoint));
          setEnabled(false);
          await subscription.unsubscribe();
        }
        setEnabled(false);
      } else {
        const registration = await pushRegistration();
        await registration.showNotification("Dakia", { body: "Notifications are working on this device.", icon: "/dakia-icon-192.png", tag: "dakia-test", data: { path: "/" } });
        setStatus("Test notification shown. This checks this device, not server delivery.");
      }
    } catch (error) { setError(error instanceof Error ? error.message : "Couldn’t change notifications. Try again."); }
    finally {
      busyRef.current = false; setBusy(false);
      requestAnimationFrame(() => { if (trigger?.isConnected && document.activeElement === document.body) trigger.focus(); });
    }
  }
  const blocked = permission === "denied";
  return <section className="workspace-notifications" aria-labelledby="workspace-notifications">
    <span className="workspace-account-icon"><Bell size={20} /></span>
    <div><h2 id="workspace-notifications">Notifications</h2>
      <p role="status">{!supported ? localTransport ? "Notifications aren’t available in the demo."
        : /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1)
          ? "Install Dakia and open it from your home screen to enable notifications."
          : "Notifications aren’t available in this browser."
        : checking ? "Checking this device…" : blocked ? "Blocked in your browser or device settings."
        : enabled ? "New-mail notifications are on for this device." : "New-mail notifications are off for this device."}</p>
      {supported && <div className="preference-actions">
        <button disabled={busy || checking || (!enabled && (blocked || !publicKey))} onClick={() => void run(enabled ? "disable" : "enable")}>{busy ? "Working…" : enabled ? "Disable notifications" : "Enable notifications"}</button>
        {enabled && !blocked && <button disabled={busy || checking} onClick={() => void run("test")}>Test notification</button>}
        {(error || blocked) && <button disabled={busy || checking} onClick={() => void refresh()}>Check again</button>}
      </div>}
      {status && <p role="status">{status}</p>}{error && <p role="alert">{error}</p>}
    </div>
  </section>;
}

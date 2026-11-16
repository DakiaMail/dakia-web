import { createContext, useCallback, useContext, useEffect, useState, type ReactNode } from "react";
import { useBrowserUpdateVisible } from "./AppUpdate";
import { InstallDialog } from "./InstallDialog";
import { Download, X } from "lucide-react";

type InstallPrompt = Event & {
  prompt: () => Promise<void>;
  userChoice: Promise<{ outcome: "accepted" | "dismissed" }>;
};
const InstallContext = createContext<{
  prompt: InstallPrompt | null;
  standalone: boolean;
  clear: () => void;
  showGuide: () => void;
}>({ prompt: null, standalone: false, clear: () => {}, showGuide: () => {} });
const isStandalone = () => matchMedia("(display-mode: standalone)").matches ||
  (navigator as Navigator & { standalone?: boolean }).standalone === true;

// Listen before the user opens Workspace; browsers may offer installation only once.
export function InstallProvider({ children }: { children: ReactNode }) {
  const [guide, setGuide] = useState(false);
  const [prompt, setPrompt] = useState<InstallPrompt | null>(null);
  const closeGuide = useCallback(() => { setGuide(false); setPrompt(null); }, []);
  const [standalone, setStandalone] = useState(isStandalone);
  useEffect(() => {
    const media = matchMedia("(display-mode: standalone)");
    const offered = (event: Event) => { event.preventDefault(); setPrompt(event as InstallPrompt); };
    const installed = () => { setPrompt(null); setGuide(false); setStandalone(true); };
    const changed = () => setStandalone(isStandalone());
    window.addEventListener("beforeinstallprompt", offered);
    window.addEventListener("appinstalled", installed);
    media.addEventListener("change", changed);
    return () => {
      window.removeEventListener("beforeinstallprompt", offered);
      window.removeEventListener("appinstalled", installed);
      media.removeEventListener("change", changed);
    };
  }, []);
  return <InstallContext.Provider value={{ prompt, standalone, clear: () => setPrompt(null), showGuide: () => setGuide(true) }}>{children}{guide && <InstallDialog prompt={prompt} onClose={closeGuide} />}</InstallContext.Provider>;
}

export function InstallApp() {
  const { prompt, standalone, clear, showGuide } = useContext(InstallContext);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState("");
  if (standalone) return null;
  const ios = /iPad|iPhone|iPod/.test(navigator.userAgent) ||
    (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
  const safari = /Safari/.test(navigator.userAgent) && !/CriOS|FxiOS|EdgiOS|OPiOS/.test(navigator.userAgent);
  async function install() {
    if (!prompt || busy) return;
    setBusy(true);
    try {
      await prompt.prompt();
      const choice = await prompt.userChoice;
      setStatus(choice.outcome === "accepted" ? "Finish installation in your browser." : "You can install later from your browser menu.");
    } catch {
      setStatus("Use your browser menu to install Dakia, if available.");
    } finally {
      clear();
      setBusy(false);
    }
  }
  return <section className="workspace-notifications install-app" aria-label="Install Dakia">
    <span className="workspace-account-icon"><Download size={20} /></span>
    <div><h2>Install Dakia</h2><p>Open your mail from your home screen. An internet connection is required.</p>
      {ios || /Android/.test(navigator.userAgent) ? <div><button onClick={showGuide}>Install Dakia</button>{ios && <p>Open Dakia from your home screen before enabling notifications.</p>}</div> : prompt ? <button title="Install Dakia" disabled={busy} onClick={() => void install()}>Install Dakia</button> :
        <details><summary>How to install</summary><p>{/Mac/.test(navigator.platform) && safari
          ? "In Safari, open File and choose Add to Dock. In Chrome or Edge, use the browser’s install option."
          : "In Chrome or Edge, use the install icon in the address bar or the browser menu. Other browsers may offer different installation options."}</p>
        </details>}
      {status && <p role="status">{status}</p>}
    </div>
  </section>;
}

const dismissKey = "dakia-install-invitation";
export function MobileInstallPrompt({ enabled }: { enabled: boolean }) {
  const { standalone, prompt, showGuide } = useContext(InstallContext);
  const updateVisible = useBrowserUpdateVisible();
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    if (!enabled || standalone) return;
    const mobile = /iPhone|iPad|iPod|Android/.test(navigator.userAgent) || (navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);
    if (!mobile) return;
    try { if (Number(localStorage.getItem(dismissKey)) > Date.now()) return; } catch { /* Session-only invitation when storage is unavailable. */ }
    const timer = setTimeout(() => setVisible(true), 8000);
    return () => clearTimeout(timer);
  }, [enabled, standalone]);
  if (!enabled || standalone || !visible || updateVisible) return null;
  const dismiss = () => {
    setVisible(false);
    try { localStorage.setItem(dismissKey, String(Date.now() + 30 * 86400000)); } catch { /* Still dismiss for this session. */ }
  };
  return <aside className="mobile-install-invitation" aria-label="Add Dakia to your home screen">
    <img src="/dakia-icon-192.png" alt="" width="48" height="48" />
    <div><h2>Your mail, one tap away.</h2><p>Add Dakia to your home screen.</p>
      <button className="primary" onClick={() => { showGuide(); dismiss(); }}>{prompt ? "Install Dakia" : "Show me how"}</button>
    </div>
    <button className="install-dismiss" aria-label="Not now" onClick={dismiss}><X size={18} /></button>
  </aside>;
}

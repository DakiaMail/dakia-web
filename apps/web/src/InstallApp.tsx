import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { Download } from "lucide-react";

type InstallPrompt = Event & {
  prompt: () => Promise<void>;
  userChoice: Promise<{ outcome: "accepted" | "dismissed" }>;
};
const InstallContext = createContext<{
  prompt: InstallPrompt | null;
  standalone: boolean;
  clear: () => void;
}>({ prompt: null, standalone: false, clear: () => {} });
const isStandalone = () => matchMedia("(display-mode: standalone)").matches ||
  (navigator as Navigator & { standalone?: boolean }).standalone === true;

// Listen before the user opens Workspace; browsers may offer installation only once.
export function InstallProvider({ children }: { children: ReactNode }) {
  const [prompt, setPrompt] = useState<InstallPrompt | null>(null);
  const [standalone, setStandalone] = useState(isStandalone);
  useEffect(() => {
    const media = matchMedia("(display-mode: standalone)");
    const offered = (event: Event) => { event.preventDefault(); setPrompt(event as InstallPrompt); };
    const installed = () => { setPrompt(null); setStandalone(true); };
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
  return <InstallContext.Provider value={{ prompt, standalone, clear: () => setPrompt(null) }}>{children}</InstallContext.Provider>;
}

export function InstallApp() {
  const { prompt, standalone, clear } = useContext(InstallContext);
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
      {prompt && !ios ? <button title="Install Dakia" disabled={busy} onClick={() => void install()}>Install Dakia</button> :
        <details><summary>How to install</summary><p>{ios
          ? safari
            ? "In Safari, tap Share, then Add to Home Screen and Add. If asked, keep Open as Web App enabled."
            : "Open this page in Safari, then tap Share, Add to Home Screen and Add."
          : "In your browser menu, choose Install Dakia or Add to Home screen, if available. Installation requires a supported browser and a secure connection."}</p>
          {ios && <p>On iOS 16.4 or later, open Dakia from your home screen before enabling notifications in Workspace.</p>}
        </details>}
      {status && <p role="status">{status}</p>}
    </div>
  </section>;
}

import { useEffect, useRef, useState } from "react";
import { useReloadGuard } from "./AppUpdate";
import { useBackLayer } from "./appHistory";
import type { PWAInstallElement } from "@khmyznikov/pwa-install";

type Prompt = Event & { prompt: () => Promise<void>; userChoice: Promise<{ outcome: "accepted" | "dismissed" }> };
export function InstallDialog({ prompt, onClose }: { prompt: Prompt | null; onClose: () => void }) {
  useReloadGuard();
  useBackLayer(onClose);
  const host = useRef<HTMLDialogElement>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    const dialog = host.current!;
    const originalFocus = document.activeElement as HTMLElement | null;
    dialog.showModal();
    let stopped = false, element: PWAInstallElement | undefined;
    const close = () => { element?.hideDialog(); onClose(); };
    const clicked = () => { void element?.updateComplete.then(() => { if (!stopped && element?.isDialogHidden) close(); }); };
    const keydown = (event: KeyboardEvent) => { if (event.key === "Escape") { event.preventDefault(); close(); } };
    dialog.addEventListener("click", clicked);
    dialog.addEventListener("keydown", keydown);
    void import("@khmyznikov/pwa-install").then(async () => {
      if (stopped) return;
      element = document.createElement("pwa-install") as PWAInstallElement;
      element.setAttribute("manual-apple", ""); element.setAttribute("manual-chrome", "");
      element.setAttribute("disable-screenshots", "");
      element.setAttribute("manifest-url", "/manifest.webmanifest");
      element.setAttribute("name", "Dakia");
      element.setAttribute("description", "Your mail, one tap away.");
      element.setAttribute("install-description", "Add Dakia to your home screen.");
      element.setAttribute("icon", "/dakia-icon-192.png");
      element.styles = { "--tint-color": "#B7432B" };
      if (prompt) element.externalPromptEvent = prompt as PWAInstallElement["externalPromptEvent"];
      element.addEventListener("pwa-user-choice-result-event", close);
      element.addEventListener("pwa-install-success-event", close);
      dialog.append(element);
      await element.updateComplete;
      const accessibilityStyle = document.createElement("style");
      accessibilityStyle.textContent = "button:focus-visible { outline: 2px solid currentColor !important; outline-offset: 3px; } @media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation: none !important; transition: none !important; } }";
      element.shadowRoot?.append(accessibilityStyle);
      // Platform detection is asynchronous; wait for the library's ready event.
      const open = async () => {
        if (stopped) return;
        element!.showDialog(true);
        await element!.updateComplete;
        if (stopped) return;
        const sheet = element!.shadowRoot?.querySelector("pwa-bottom-sheet");
        if (sheet) await (sheet as HTMLElement & { updateComplete: Promise<boolean> }).updateComplete;
        if (stopped) return;
        const focusFirst = (root: ShadowRoot | Element): HTMLElement | null => {
          for (const child of root.querySelectorAll<HTMLElement>("button, [tabindex], pwa-bottom-sheet")) {
            if (child.shadowRoot) { child.shadowRoot.append(accessibilityStyle.cloneNode(true)); const nested = focusFirst(child.shadowRoot); if (nested) return nested; }
            if (child.matches("button, [tabindex]") && child.getBoundingClientRect().height) return child;
          }
          return null;
        };
        focusFirst(element!.shadowRoot!)?.focus();
      };
      element.addEventListener("pwa-install-available-event", () => void open());
      if (element.isInstallAvailable) void open();
      // Desktop browsers without an install API still receive the library guide.
      if (!/iPhone|iPad|iPod|Android/.test(navigator.userAgent) && navigator.maxTouchPoints < 2) void open();
    }).catch(() => { if (!stopped) { setError("Couldn’t open the install guide. Use your browser menu to install Dakia."); } });
    return () => {
      stopped = true;
      dialog.removeEventListener("click", clicked); dialog.removeEventListener("keydown", keydown);
      dialog.close(); element?.remove();
      requestAnimationFrame(() => {
        if (originalFocus?.isConnected && originalFocus !== document.body) originalFocus.focus();
        else document.querySelector<HTMLElement>('.workspace-content > h1, .app.reading .reader, .message-list[tabindex], .mailbox')?.focus();
      });
    };
  }, [prompt, onClose]);
  return <dialog ref={host} className="install-guide-host" aria-label="Install Dakia" onClick={event => { if (event.target === event.currentTarget) onClose(); }} onCancel={event => { event.preventDefault(); onClose(); }}>
    {error && <div className="install-guide-error"><p role="alert">{error}</p><button onClick={onClose}>Close</button></div>}
  </dialog>;
}

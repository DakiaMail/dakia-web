import { ArrowLeft, Bell, Keyboard, Mail, Plus, Settings2, Shield } from "lucide-react";
import type { Account } from "@dakia/api-types";
import { useEffect, useRef, useState } from "react";
import { TranslationSettings } from "./TranslationSettings";
import { Invitations } from "./Invitations";
import { BlockedSenders } from "./BlockedSenders";
import { ReleaseSettings } from "./Releases";
import { InstallApp } from "./InstallApp";

export function Workspace({ updatesFocus = 0, userId, accounts, onBack, onConnect, onSettings, onRemove, onReconnect, onNotifications, notificationStatus, alwaysLoadImages, onImagePrivacyChange, blockHiddenImages, onHiddenImagesChange, keyboardHints, onKeyboardHintsChange, keyboardShortcuts, onKeyboardShortcutsChange, onShortcutHelp }: {
  updatesFocus?: number;
  userId?: string;
  keyboardHints: boolean;
  onKeyboardHintsChange: (enabled: boolean) => void;
  keyboardShortcuts: boolean;
  onKeyboardShortcutsChange: (enabled: boolean) => void;
  onShortcutHelp: () => void;
  accounts: Account[];
  onBack: () => void;
  onConnect: () => void;
  onSettings: (account: Account) => void;
  onRemove: (account: Account) => void;
  onReconnect: (account: Account) => void;
  onNotifications: () => void;
  notificationStatus: string;
  alwaysLoadImages: boolean;
  onImagePrivacyChange: (enabled: boolean) => void;
  blockHiddenImages: boolean;
  onHiddenImagesChange: (enabled: boolean) => void;
}) {
  const heading = useRef<HTMLHeadingElement>(null);
  const [privacyError, setPrivacyError] = useState("");
  const [keyboardError, setKeyboardError] = useState("");
  useEffect(() => { if (!updatesFocus) heading.current?.focus(); }, [updatesFocus]);
  return <main className="workspace-page" aria-label="Workspace management">
    <header className="workspace-header"><button title="Back to mail" onClick={onBack}><ArrowLeft size={18} />Back to mail</button></header>
    <div className="workspace-content">
      <h1 ref={heading} tabIndex={-1}>Workspace</h1>
      <p className="workspace-intro">Your accounts and preferences, in one place.</p>
      <ReleaseSettings userId={userId} focusRequest={updatesFocus} />
      <section aria-labelledby="workspace-accounts">
        <div className="workspace-section-heading"><h2 id="workspace-accounts">Email accounts</h2><button title="Connect account" className="primary" onClick={onConnect}><Plus size={16} />Connect account</button></div>
        <div className="workspace-accounts">
          {accounts.map((account) => <article className="workspace-account" key={account.id} aria-label={account.display_name}>
            <div className="workspace-account-identity"><span className="workspace-account-icon"><Mail size={20} /></span><div><h3>{account.display_name}</h3><p>{account.email}</p><small>{account.state === "ready" ? "Connected" : "Needs attention"}</small></div></div>
            <div className="workspace-account-actions"><button title="Mailbox settings" onClick={() => onSettings(account)}><Settings2 size={16} />Mailbox settings</button>{account.state !== "ready" && <button title="Reconnect" onClick={() => onReconnect(account)}>Reconnect</button>}<button title="Remove account" className="remove-account-link" onClick={() => onRemove(account)}>Remove account</button></div>
          </article>)}
          {!accounts.length && <p className="workspace-intro">Connect an email account to get started.</p>}
        </div>
      </section>
      <section className="workspace-notifications" aria-labelledby="workspace-privacy">
        <span className="workspace-account-icon"><Shield size={20} /></span><div><h2 id="workspace-privacy">Email privacy</h2><p>Recent mail is saved on this browser for faster opening. Signing out clears saved mail.</p><label className="privacy-toggle"><input type="checkbox" checked={!alwaysLoadImages} onChange={(event) => { try { onImagePrivacyChange(!event.target.checked); setPrivacyError(""); } catch { setPrivacyError("Could not save this preference. Check browser storage permissions."); } }} />Block external images by default</label><p>Off by default. Loading external images can tell senders when you open mail. Turn on to load images only when you choose.</p><label className="privacy-toggle"><input type="checkbox" checked={blockHiddenImages} onChange={(event) => { try { onHiddenImagesChange(event.target.checked); setPrivacyError(""); } catch { setPrivacyError("Could not save this preference. Check browser storage permissions."); } }} />Block hidden images and tracking pixels</label><p>Off by default. Skips explicitly hidden and zero/one-pixel images, including some legitimate assets. This cannot detect every tracker. Applies even when you choose Load images.</p><p>Saved separately for your signed-in user, for all your accounts on this browser. Existing image-loading choices are preserved. Network isolation and message sandboxing are always enforced.</p>{privacyError && <p role="alert">{privacyError}</p>}</div>
      </section>
      <BlockedSenders accounts={accounts} />
      {userId && <Invitations key={userId} userId={userId} />}
      <section className="workspace-notifications workspace-keyboard" aria-labelledby="workspace-keyboard">
        <span className="workspace-account-icon"><Keyboard size={20} /></span>
        <div><h2 id="workspace-keyboard">Keyboard</h2>
          <div className="keyboard-settings-options">
            <label className="privacy-toggle"><input type="checkbox" aria-label="Enable mail keyboard shortcuts" aria-describedby="keyboard-shortcuts-note" checked={keyboardShortcuts} onChange={event => { try { onKeyboardShortcutsChange(event.target.checked); setKeyboardError(""); } catch { setKeyboardError("Could not save this preference. Check browser storage permissions."); } }} /><span>Enable mail keyboard shortcuts<small id="keyboard-shortcuts-note">Tab and Enter always work.</small></span></label>
            <label className="privacy-toggle"><input type="checkbox" aria-label="Show extra keyboard hints" aria-describedby="keyboard-hints-note" checked={keyboardHints} onChange={event => { try { onKeyboardHintsChange(event.target.checked); setKeyboardError(""); } catch { setKeyboardError("Could not save this preference. Check browser storage permissions."); } }} /><span>Show extra keyboard hints<small id="keyboard-hints-note">Show keys beside actions and below the inbox.</small></span></label>
          </div>
          <button title="Keyboard shortcuts" onClick={onShortcutHelp}>Keyboard shortcuts</button>
          {keyboardError && <p role="alert">{keyboardError}</p>}
        </div>
      </section>
      {userId && <TranslationSettings userId={userId} />}
      <InstallApp />
      <section className="workspace-notifications" aria-labelledby="workspace-notifications">
        <span className="workspace-account-icon"><Bell size={20} /></span><div><h2 id="workspace-notifications">Notifications</h2><p>Get new-mail notifications on this browser.</p><button title="Enable notifications" onClick={onNotifications} disabled={notificationStatus === "Enabling…"}>Enable notifications</button>{notificationStatus && <p role="status">{notificationStatus}</p>}</div>
      </section>
    </div>
  </main>;
}

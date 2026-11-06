import { ArrowLeft, Bell, Mail, Plus, Settings2, Shield } from "lucide-react";
import type { Account } from "@dakia/api-types";
import { useEffect, useRef, useState } from "react";
import { Invitations } from "./Invitations";
import { InstallApp } from "./InstallApp";

export function Workspace({ userId, accounts, onBack, onConnect, onSettings, onRemove, onReconnect, onNotifications, notificationStatus, alwaysLoadImages, onImagePrivacyChange, blockHiddenImages, onHiddenImagesChange }: {
  userId?: string;
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
  useEffect(() => { heading.current?.focus(); }, []);
  return <main className="workspace-page" aria-label="Workspace management">
    <header className="workspace-header"><button title="Back to mail" onClick={onBack}><ArrowLeft size={18} />Back to mail</button></header>
    <div className="workspace-content">
      <h1 ref={heading} tabIndex={-1}>Workspace</h1>
      <p className="workspace-intro">Your accounts and preferences, in one place.</p>
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
        <span className="workspace-account-icon"><Shield size={20} /></span><div><h2 id="workspace-privacy">Email privacy</h2><label className="privacy-toggle"><input type="checkbox" checked={!alwaysLoadImages} onChange={(event) => { try { onImagePrivacyChange(!event.target.checked); setPrivacyError(""); } catch { setPrivacyError("Could not save this preference. Check browser storage permissions."); } }} />Block external images by default</label><p>Off by default. Loading external images can tell senders when you open mail. Turn on to load images only when you choose.</p><label className="privacy-toggle"><input type="checkbox" checked={blockHiddenImages} onChange={(event) => { try { onHiddenImagesChange(event.target.checked); setPrivacyError(""); } catch { setPrivacyError("Could not save this preference. Check browser storage permissions."); } }} />Block hidden images and tracking pixels</label><p>Off by default. Skips explicitly hidden and zero/one-pixel images, including some legitimate assets. This cannot detect every tracker. Applies even when you choose Load images.</p><p>Saved separately for your signed-in user, for all your accounts on this browser. Existing image-loading choices are preserved. Network isolation and message sandboxing are always enforced.</p>{privacyError && <p role="alert">{privacyError}</p>}</div>
      </section>
      {userId && <Invitations key={userId} userId={userId} />}
      <InstallApp />
      <section className="workspace-notifications" aria-labelledby="workspace-notifications">
        <span className="workspace-account-icon"><Bell size={20} /></span><div><h2 id="workspace-notifications">Notifications</h2><p>Get new-mail notifications on this browser.</p><button title="Enable notifications" onClick={onNotifications} disabled={notificationStatus === "Enabling…"}>Enable notifications</button>{notificationStatus && <p role="status">{notificationStatus}</p>}</div>
      </section>
    </div>
  </main>;
}

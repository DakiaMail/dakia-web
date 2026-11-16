import { useState } from "react";
import { Mail } from "lucide-react";
export function MailLinkSettings() {
  const [status, setStatus] = useState("");
  if (!("registerProtocolHandler" in navigator)) return null;
  return <section className="workspace-notifications" aria-labelledby="mail-link-heading">
    <span className="workspace-account-icon"><Mail size={20} /></span>
    <div><h2 id="mail-link-heading">Email links</h2><p>Open email links in Dakia with a draft ready to write.</p>
      <button onClick={() => {
        try {
          navigator.registerProtocolHandler("mailto", `${location.origin}/?mailto=%s`);
          setStatus("Confirm in your browser if asked. You can change your choice in browser settings.");
        } catch { setStatus("Use your browser settings to choose an app for email links, if supported."); }
      }}>Use Dakia for email links</button>
      {status && <p role="status">{status}</p>}
    </div>
  </section>;
}

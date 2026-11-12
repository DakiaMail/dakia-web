import { useEffect, useState } from "react";
import type { Message } from "@dakia/api-types";
import { classification, emailTypes, safeDetailLink } from "./classification";
export function MailDetails({ message }: { message: Message }) {
  const [status, setStatus] = useState("");
  const c = classification(message);
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const expiry = c.attention.map(a => a.expires_at ? Date.parse(a.expires_at) : Infinity).filter(t => Number.isFinite(t) && t > Date.now()).sort((a,b) => a-b)[0];
    if (!expiry) return;
    const timer = setTimeout(() => setNow(Date.now()), Math.min(expiry - Date.now() + 20, 2147483647));
    return () => clearTimeout(timer);
  }, [message.classification, now]);
  const details = c.details.filter(d => c.types.includes(d.email_type) && (d.email_type !== "verification_code" || c.attention.some(a => a.label === "Verification code" && (!a.expires_at || Date.parse(a.expires_at) > now))));
  if (!c.types.length && !details.length) return null;
  return <div className="mail-details" aria-label="Detected email information">
    <span className="detected-types">{c.types.map(t => emailTypes[t]).join(" · ")}</span>
    {details.map((detail, i) => detail.action === "copy" ? <button key={i} title={`Copy ${detail.label.toLowerCase()}`} onClick={() => void navigator.clipboard.writeText(detail.value).then(() => setStatus(`${detail.label} copied`)).catch(() => setStatus("Couldn’t copy. Try selecting the value."))}>{detail.label}: {detail.value}</button>
      : detail.action === "open" ? (() => { const url = safeDetailLink(detail.value); return url ? <a key={i} href={url.href} target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer">{detail.label} <span>{url.hostname}</span></a> : null; })()
      : <span key={i}>{detail.label}: {detail.value}</span>)}
    <span className="sr-only" role="status">{status}</span>
  </div>;
}

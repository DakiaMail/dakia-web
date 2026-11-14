import { CopyButton } from "@dakia/ui";
import type { Message } from "@dakia/api-types";
import { classification, safeDetailLink } from "./classification";
import { Unsubscribe } from "./Unsubscribe";
export function MailDetails({ message }: { message: Message }) {
  const c = classification(message);
  const details = c.details.filter(d => c.types.includes(d.email_type));
  const newsletter = c.types.includes("newsletter");
  if (!details.length && !newsletter) return null;
  return <div className={`mail-details${newsletter || details.some(d => d.action) ? " has-actions" : ""}`} aria-label="Detected email information">
    {details.map((detail, i) => detail.action === "copy" ? <CopyButton key={i} value={detail.value} title={`Copy ${detail.label.toLowerCase()}`}>{`${detail.label}: ${detail.value}`}</CopyButton>
      : detail.action === "open" ? (() => { const url = safeDetailLink(detail.value); return url ? <a key={i} href={url.href} target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer">{detail.label} <span>{url.hostname}</span></a> : null; })()
      : <span key={i}>{detail.label}: {detail.value}</span>)}
    {newsletter && <Unsubscribe message={message} lazy />}
  </div>;
}

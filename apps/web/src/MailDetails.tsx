import { CopyButton } from "@dakia/ui";
import type { Message } from "@dakia/api-types";
import { ExternalLink } from "lucide-react";
import { classification, safeDetailLink } from "./classification";
import { Unsubscribe } from "./Unsubscribe";

const referenceLabels: Partial<Record<string, string>> = {
  order: "Copy order number",
  flight_booking: "Copy booking reference",
  shipping: "Copy shipping reference",
};

export function MailDetails({ message, reader = false }: { message: Message; reader?: boolean }) {
  const c = classification(message);
  const details = c.details.filter(d => c.types.includes(d.email_type) && d.value.trim()
    && (d.action !== "open" || safeDetailLink(d.value)));
  // Use only supplied classification and facts. Never infer a new mail type here.
  const newsletter = !reader && c.types.includes("newsletter");
  if (!details.length && !newsletter) return null;
  return <div className={`${reader ? "reader-mail-details" : "mail-details"}${newsletter || details.some(d => d.action) ? " has-actions" : ""}`} aria-label="Detected email information">
    {details.map((detail, i) => {
      const label = detail.label.toLowerCase() === "reference" && referenceLabels[detail.email_type]
        || (detail.email_type === "order" && ["order reference", "order number"].includes(detail.label.toLowerCase())
          ? "Copy order number" : `Copy ${detail.label.toLowerCase()}`);
      const url = detail.action === "open" ? safeDetailLink(detail.value) : null;
      return <div className="mail-detail" key={i}>
        {detail.action === "copy" ? <CopyButton className="context-copy" value={detail.value} title={label}>{label}</CopyButton>
          : url ? <a href={url.href} title={`${detail.label} · ${url.hostname}`} target="_blank" rel="noopener noreferrer" referrerPolicy="no-referrer"><ExternalLink size={14} aria-hidden="true" />{detail.label}<span className="detail-domain">{url.hostname}</span></a>
          : <span>{detail.label}: {detail.value}</span>}
        {reader && detail.action === "copy" && <span className="detail-value" dir="auto">{detail.value}</span>}
      </div>;
    })}
    {newsletter && <Unsubscribe message={message} lazy />}
  </div>;
}

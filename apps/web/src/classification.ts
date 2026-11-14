import type { Classification, DetectedType, InboxKind, Message, Change } from "@dakia/api-types";
export const emailTypes: Record<DetectedType, string> = {
  receipt: "Receipts", invoice: "Invoices", order: "Orders", shipping: "Shipping", event_invitation: "Invitations",
  verification_code: "Codes", flight_booking: "Flights", newsletter: "Newsletters", offer: "Offers", security_alert: "Security alerts",
};
export type InboxTab = InboxKind | "all";
export type MailFilter = { inbox_kind?: InboxKind; email_type?: DetectedType; transactions?: boolean; attention?: boolean };
export function classification(message: Message): Classification {
  return message.classification || { kind: "personal", status: "pending", types: [], details: [], attention: [], attention_override: null, source: "automatic", policy_revision: "" };
}
export function needsAttention(message: Message, now = Date.now()): boolean {
  const received = Date.parse(message.received_at);
  if (!Number.isFinite(received) || received > now || received < now - 30 * 86400000) return false;
  const c = classification(message);
  return c.attention_override ?? c.attention.some(reason => !reason.expires_at || Date.parse(reason.expires_at) > now);
}
export function matchesClassification(message: Message, filter: MailFilter, now = Date.now()): boolean {
  const c = classification(message);
  return (!filter.inbox_kind || filter.inbox_kind === c.kind) && (!filter.email_type || c.types.includes(filter.email_type)) && (!filter.transactions || c.types.some(t => t === "receipt" || t === "invoice")) && (!filter.attention || needsAttention(message, now));
}
export function optimisticClassification(message: Message, change: Change): Message {
  const c = structuredClone(classification(message));
  if (change.kind === "inbox_kind" && !(change.sender && change.value === null && c.source === "message")) { c.kind = change.value || "personal"; c.source = change.value ? change.sender ? "sender" : "message" : "automatic"; }
  if (change.kind === "detected_type") c.types = change.present ? [...new Set([...c.types, change.value])] : c.types.filter(t => t !== change.value);
  if (change.kind === "attention") c.attention_override = change.value;
  return { ...message, classification: c };
}
export function safeDetailLink(value: string): URL | null {
  try {
    const u = new URL(value);
    const host = u.hostname.toLowerCase().replace(/\.$/, "");
    return ["https:", "http:"].includes(u.protocol) && !u.username && !u.password && !/[\u0000-\u001f]/.test(value) && host !== "localhost" && !host.endsWith(".localhost") && !host.endsWith(".local") && !host.startsWith("[") && !/^\d+(?:\.\d+){3}$/.test(host) ? u : null;
  } catch { return null; }
}
// Persisted browser rows are untrusted, including rows written by older clients.
export function isClassification(value: unknown): value is Classification {
  if (!value || typeof value !== "object") return false;
  const c = value as Classification;
  const text = (v: unknown, max: number) => typeof v === "string" && v.length <= max && !/[\u0000-\u001f]/.test(v);
  const type = (v: unknown) => typeof v === "string" && Object.hasOwn(emailTypes, v);
  return ["personal", "automated"].includes(c.kind) && ["pending", "ready", "failed"].includes(c.status) && ["automatic", "message", "sender"].includes(c.source)
    && text(c.policy_revision, 100) && (c.attention_override === null || typeof c.attention_override === "boolean")
    && Array.isArray(c.types) && c.types.length <= 10 && c.types.every(type)
    && Array.isArray(c.details) && c.details.length <= 16 && c.details.every(d => d && type(d.email_type) && text(d.label, 120) && text(d.value, 2048) && [null, "copy", "open"].includes(d.action))
    && Array.isArray(c.attention) && c.attention.length <= 10 && c.attention.every(a => a && text(a.label, 120) && (a.expires_at === null || (typeof a.expires_at === "string" && Number.isFinite(Date.parse(a.expires_at)))));
}

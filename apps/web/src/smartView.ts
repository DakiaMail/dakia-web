import type { Message } from "@dakia/api-types";

export const sectionSize = 8;
export const groups = ["people", "notifications", "transactions", "newsletters", "other", "pending"];
export type SectionState = { collapsed?: boolean; limit?: number };
export type MailItem =
  | { kind: "heading"; label: string; count: number; collapsed: boolean }
  | { kind: "more"; label: string }
  | { kind: "mail"; message: Message };

// Unclassified unread mail stays in Seen until classification is ready.
// Seen is the remainder, not a read-status filter, so nothing disappears.
export function smartItems(rows: Message[], state: Record<string, SectionState>, heldSections: Record<string, { label: string }> = {}): MailItem[] {
  const buckets = new Map<string, Message[]>();
  const seen = new Set<string>();
  for (const message of rows) {
    const id = `${message.account_id}/${message.id}`;
    if (seen.has(id)) continue;
    seen.add(id);
    const category = message.category?.trim().toLowerCase();
    const label = heldSections[id]?.label || (message.unread && message.category_status === "ready" && category
      ? category : "seen");
    const bucket = buckets.get(label) || [];
    bucket.push(message);
    buckets.set(label, bucket);
  }
  const labels = [...new Set([...groups, ...buckets.keys()])].filter(label => label !== "seen");
  labels.push("seen");
  return labels.flatMap((label): MailItem[] => {
    const messages = buckets.get(label);
    if (!messages?.length) return [];
    const { collapsed = false, limit = sectionSize } = state[label] || {};
    return [
      { kind: "heading", label, count: messages.length, collapsed },
      ...(!collapsed ? messages.slice(0, limit).map((message): MailItem => ({ kind: "mail", message })) : []),
      ...(!collapsed && messages.length > limit
        ? [{ kind: "more" as const, label }] : []),
    ];
  });
}

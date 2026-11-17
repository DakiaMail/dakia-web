import type { Message, SearchMetadata } from "@dakia/api-types";

export const identity = (message: Message) => `${message.account_id}/${message.id}`;

// Each federated page is sorted independently, not relative to earlier pages.
// Merge the complete loaded set before list virtualization.
export function mailRows(pages: { rows: Message[], cache_preview?: boolean | null, coverage?: { state: string }[], search?: SearchMetadata | null }[], overrides: Record<string, Message>): Message[] {
  const unique = new Map<string, Message>();
  const complete = pages.filter(p => p.coverage).at(-1)?.coverage?.every(c => c.state === "complete");
  const ranked=pages.some(p=>p.search?.ranked);
  const scores=new Map<string,number>();
  for (const page of pages) for (const match of page.search?.matches||[]) {
    if (Number.isFinite(match.score)) scores.set(`${match.account_id}/${match.message_id}`,Math.max(scores.get(`${match.account_id}/${match.message_id}`)??-Infinity,match.score));
  }
  // Live rows win. Only complete coverage can discard preview-only positives.
  for (const page of [...pages.filter(p => !p.cache_preview), ...pages.filter(p => p.cache_preview && (!complete || p.search?.ranked))]) {
    for (const message of page.rows) {
      const key = identity(message);
      if (!unique.has(key)) unique.set(key, overrides[key] || message);
    }
  }
  return [...unique.values()]
    .map(message => {
      const parsed = Date.parse(message.received_at);
      return { message, timestamp: Number.isFinite(parsed) ? parsed : -Infinity };
    })
    // Stable ties preserve the API's existing account/keyset order. Invalid
    // timestamps stay at the end without making the comparator return NaN.
    .sort((a,b)=>{
      if (ranked) {
        const left=scores.get(identity(a.message))??-Infinity;
        const right=scores.get(identity(b.message))??-Infinity;
        if(left!==right)return left>right?-1:1;
      }
      return a.timestamp===b.timestamp?0:a.timestamp>b.timestamp?-1:1;
    })
    .map(({ message }) => message);
}

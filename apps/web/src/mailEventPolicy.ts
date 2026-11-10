import type { Query } from "@tanstack/react-query";

// Federated mail queries carry their account list at index 5. Record lists
// carry account IDs at indices 1..n; content is keyed by account and message.
export function eventAffects(query: Query, id: string, kinds: Set<string>, targets: Set<string>) {
  const [resource, account] = query.queryKey;
  if (resource === "rules") return kinds.has("reset") || kinds.has("rules_changed");
  if (resource === "rule-executions") return account === id && (kinds.has("reset") || kinds.has("operation_changed") || kinds.has("message_changed") || kinds.has("rules_changed"));
  const all = kinds.has("reset");
  const mail = all || [...kinds].some(k => ["message_changed", "operation_changed", "snooze_woke", "rules_changed"].includes(k));
  if (resource === "sender-cleanups") return query.queryKey[2] === id && (all || (kinds.has("operation_changed") && Array.isArray(query.state.data) && query.state.data.some(job => job.pending > 0)));
  if (resource === "snoozes") return mail && Array.isArray(account) && account.includes(id);
  if (resource === "search-page") return mail && Array.isArray(query.queryKey[2]) && query.queryKey[2].includes(id);
  if (resource === "mail") return mail && Array.isArray(query.queryKey[5]) && query.queryKey[5].includes(id);
  if (resource === "content") return account === id && (all || kinds.has("operation_changed") || (kinds.has("message_changed") && targets.has(String(query.queryKey[2]))));
  // Unknown future events conservatively refresh the indexed local graph too.
  // Provider verification has a bounded lifecycle, not SSE invalidation.
  if (resource === "conversation-snapshot") return account === id;
  if (resource === "drafts") return query.queryKey.slice(1).includes(id) && (all || kinds.has("draft_saved"));
  if (resource === "submissions") return query.queryKey.slice(1).includes(id) && (all || [...kinds].some(k => k.startsWith("submission_")));
  if (resource === "submission") return account === id && (all || ([...kinds].some(k => k.startsWith("submission_")) && targets.has(String(query.queryKey[2]))));
  return false;
}

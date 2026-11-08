import { useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type { MailRule, RuleActivity, Session } from "@dakia/api-types";
import { accountPath, key, request } from "./api";

export const senderAddress = (email: string) => email.trim().toLowerCase();
export const ruleEmail = (rule: MailRule) => rule.condition.email;
export const rulesApi = {
  list: (signal?: AbortSignal) => request<MailRule[]>("/rules", { signal }).then(rules => {
    if (!Array.isArray(rules)) throw new Error("Sender controls are unavailable. Try again.");
    return rules;
  }),
  block: (email: string) => request<MailRule>("/rules", { method: "POST", body: JSON.stringify({
    preset: "block_sender", scope: "all_mailboxes", condition: { kind: "sender_is", email },
    actions: ["trash", "suppress_notifications"],
  }) }),
  enable: (id: string, enabled: boolean) => request<MailRule>(`/rules/${key(id)}`, { method: "PATCH", body: JSON.stringify({ enabled }) }),
  executions: (account: string, signal?: AbortSignal) => request<RuleActivity[]>(`${accountPath(account)}/rule-activity`, { signal }),
};
export function useMailRules() {
  const client = useQueryClient();
  const user = client.getQueryData<Session>(["session"])?.user_id || "";
  const query = useQuery({ queryKey: ["rules", user], enabled: !!user, queryFn: ({ signal }) => rulesApi.list(signal), retry: false, staleTime: 30000 });
  return { ...query, refresh: async () => { await client.invalidateQueries({ queryKey: ["rules", user] }); } };
}
// One listener per app, rather than one channel for every expanded message.
export function useRuleUpdates() {
  const client = useQueryClient();
  useEffect(() => {
    if (typeof BroadcastChannel === "undefined") return;
    const channel = new BroadcastChannel("dakia-rules");
    channel.onmessage = () => { void client.invalidateQueries({ queryKey: ["rules"] }); };
    return () => channel.close();
  }, [client]);
}
export function announceRuleUpdate() {
  if (typeof BroadcastChannel === "undefined") return;
  const channel = new BroadcastChannel("dakia-rules"); channel.postMessage("changed"); channel.close();
}

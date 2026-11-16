import type {
  Address,
  Draft,
  Message,
  MessageContent,
  ModelJob,
  Submission,
  ScheduledMessage,
} from "@dakia/api-types";
import { accountPath, key, request } from "./api";
// These thin routes reuse the generated Rust DTOs. See API-INTEGRATION.md for unimplemented backend surfaces.
const messagePath = (m: Message) =>
  `${accountPath(m.account_id)}/messages/${key(m.id)}`;
export const extraApi = {
  scheduled: (account: string) => request<ScheduledMessage[]>(`${accountPath(account)}/scheduled`),
  drafts: (account: string) =>
    request<Draft[]>(`${accountPath(account)}/drafts`),
  recipients: (account: string, q: string) =>
    request<Address[]>(
      `${accountPath(account)}/recipients?${new URLSearchParams({ q })}`,
    ),
  folders: (account: string) =>
    request<string[]>(`${accountPath(account)}/folders`),
  submissions: (account: string) =>
    request<Submission[]>(`${accountPath(account)}/submissions`),
  translate: (m: Message, language: string) =>
    request<ModelJob>(
      `${messagePath(m)}/translation?${new URLSearchParams({ language })}`,
      { method: "POST" },
    ),
  translation: (m: Message, language: string) =>
    request<MessageContent>(
      `${messagePath(m)}/translation?${new URLSearchParams({ language })}`,
    ),
  job: (account: string, id: string) =>
    request<ModelJob>(`${accountPath(account)}/jobs/${key(id)}`),
  pushStatus: (id: string) => request<boolean>(`/push/subscriptions/${key(id)}`),
  disablePush: (id: string) => request<void>(`/push/subscriptions/${key(id)}`, { method: "DELETE" }),
  pushKey: () => request<string>("/push/public-key"),
  subscribe: (subscription: PushSubscriptionJSON) =>
    request<void>("/push/subscriptions", {
      method: "POST",
      body: JSON.stringify(subscription),
    }),
};

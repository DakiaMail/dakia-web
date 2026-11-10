import { useQuery } from "@tanstack/react-query";
import type { Session } from "@dakia/api-types";
import { request } from "./api";
import type { TranslationMode, TranslationPolicy } from "./deviceTranslation";
export const translationPolicyKey = (userId?: string) => ["translation-settings", userId || "current-session"] as const;
export const translationSettingsApi = {
  get: () => request<TranslationPolicy>("/translation/settings"),
  save: (mode: TranslationMode) => request<TranslationPolicy>("/translation/settings", { method: "PUT", body: JSON.stringify({ mode }) }),
};
export function useTranslationPolicy() {
  const session = useQuery<Session>({ queryKey: ["session"], enabled: false });
  return useQuery({ queryKey: translationPolicyKey(session.data?.user_id), queryFn: translationSettingsApi.get,
    enabled: Boolean(session.data?.user_id), staleTime: 10_000, refetchInterval: 30_000, retry: false });
}

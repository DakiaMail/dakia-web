import { useQuery } from "@tanstack/react-query";
import type { Account } from "@dakia/api-types";
import { api } from "./api";

export function AccountSyncIndicator({ account, online }: { account: Account; online: boolean }) {
  const enabled = online && account.state === "ready";
  const activity = useQuery({
    queryKey: ["sync-active", account.id],
    queryFn: ({ signal }) => api.syncActive(account.id, signal),
    enabled,
    refetchInterval: 3000,
    retry: false,
  });
  // Unknown/offline/error is not evidence of activity. Coverage and cached mail
  // fetches deliberately do not drive this indicator.
  if (!enabled || activity.isError || activity.data !== true) return null;
  return <span className="account-sync" role="img" aria-label={`Syncing ${account.display_name}`} title={`Syncing ${account.display_name}`} />;
}

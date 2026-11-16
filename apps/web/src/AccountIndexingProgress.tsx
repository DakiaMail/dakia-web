import { useEffect, useState, type ReactElement } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Account } from "@dakia/api-types";
import { api } from "./api";

export function AccountIndexingProgress({ account, online, children }: { account: Account; online: boolean; children: ReactElement }) {
  const status = useQuery({
    queryKey: ["indexing", account.id],
    queryFn: ({ signal }) => api.indexingStatus(account.id, signal),
    enabled: online,
    refetchInterval: query => query.state.data?.state === "complete" ? false : 5000,
    refetchIntervalInBackground: false,
    retry: false,
  });
  const valid = status.data?.account_id === account.id && ["discovering", "indexing", "paused", "error", "complete"].includes(status.data.state);
  const active = valid && status.data?.state !== "complete";
  const [visible, setVisible] = useState(false);
  useEffect(() => {
    if (!active) { setVisible(false); return; }
    const timer = setTimeout(() => setVisible(true), 2000);
    return () => clearTimeout(timer);
  }, [active, account.id]);
  const data = valid ? status.data : undefined;
  const paused = !online || status.isError || data?.state === "paused" || data?.state === "error";
  const percent = data?.discovery_complete && data.eligible > 0
    ? Math.min(100, Math.floor(data.processed / data.eligible * 100)) : undefined;
  const description = paused ? "Updating paused" : "Updating mail";
  return <>{children}{active && visible && data && <div
    className={`account-indexing${paused ? " paused" : ""}${percent === undefined ? " indeterminate" : ""}`}
    role="progressbar"
    aria-label={`Updating ${account.display_name}`}
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={percent}
    aria-valuetext={description}
  ><span style={percent === undefined ? undefined : { width: `${percent}%` }} /></div>}</>;
}

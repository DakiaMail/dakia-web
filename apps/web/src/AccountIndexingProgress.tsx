import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Account } from "@dakia/api-types";
import { api } from "./api";

export function AccountIndexingProgress({ account, online }: { account: Account; online: boolean }) {
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
  const gaps = valid ? status.data?.gaps || 0 : 0;
  const gapMessage = gaps === 1 ? "1 message has unreadable content." : `${gaps} messages have unreadable content.`;
  const gapStatus = <span className="sr-only" id={`indexing-${account.id}-status`}>{gaps > 0 ? gapMessage : ""}</span>;
  if (!active || !visible || !status.data) return gapStatus;
  const data = status.data;
  const paused = !online || status.isError || data.state === "paused" || data.state === "error";
  const percent = data.discovery_complete && data.eligible > 0
    ? Math.min(100, Math.floor(data.processed / data.eligible * 100)) : undefined;
  const description = !online ? "Indexing paused" : data.state === "error" ? "Indexing will retry" : paused ? "Indexing paused"
    : percent === undefined ? "Preparing search" : percent === 100 && data.backlog > 0 ? "Finishing search" : `${percent}% indexed`;
  const detail = data.gaps ? `${description}. ${gapMessage}` : description;
  return <>{gapStatus}<div
    className={`account-indexing${paused ? " paused" : ""}${percent === undefined ? " indeterminate" : ""}`}
    role="progressbar"
    aria-label={`Indexing ${account.display_name}`}
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={percent}
    aria-valuetext={detail}
    title={detail}
  ><span style={percent === undefined ? undefined : { width: `${percent}%` }} /></div></>;
}

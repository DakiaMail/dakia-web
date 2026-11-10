import { useQuery } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import { api, request } from "./api";
import { useBrowserUpdateVisible } from "./AppUpdate";

export type ReleaseStatus = {
  installed_version: string | null;
  status: "unknown" | "disabled" | "checked" | "unavailable";
  checked_at: number | null;
  available: { version: string; revision: string; image: string; notes_url: string } | null;
};
function useReleases(userId?: string) {
  const owner = useQuery({ queryKey: ["local-access", userId], queryFn: api.localAccess, enabled: !!userId, retry: false });
  const releases = useQuery({ queryKey: ["releases", userId], queryFn: () => request<ReleaseStatus>("/releases"),
    enabled: !!userId && owner.data?.can_invite === true, staleTime: 3600_000, refetchInterval: 3600_000, retry: false });
  return { owner: owner.data?.can_invite === true, ...releases };
}
const dismissKey = (user: string) => `dakia:release-dismissed:${user}`;
export function ReleaseNotice({ userId, onDetails }: { userId?: string; onDetails: () => void }) {
  const release = useReleases(userId);
  const browserUpdate = useBrowserUpdateVisible();
  const [dismissed, setDismissed] = useState("");
  useEffect(() => {
    try { setDismissed(userId ? localStorage.getItem(dismissKey(userId)) || "" : ""); } catch { setDismissed(""); }
    const changed = (event: StorageEvent) => {
      if (userId && event.key === dismissKey(userId)) setDismissed(event.newValue || "");
    };
    window.addEventListener("storage", changed);
    return () => window.removeEventListener("storage", changed);
  }, [userId]);
  const next = release.data?.available;
  if (!release.owner || !userId || !next || release.data?.status !== "checked" || browserUpdate || next.version === dismissed) return null;
  return <aside className="release-notice" aria-label="Dakia release">
    <p role="status">Dakia {next.version} is available</p>
    <div><button type="button" onClick={onDetails}>Details</button>
      <button type="button" aria-label={`Dismiss release ${next.version}`} onClick={() => {
        setDismissed(next.version);
        try { localStorage.setItem(dismissKey(userId), next.version); } catch { /* Still dismiss for this session. */ }
        document.querySelector<HTMLButtonElement>('[aria-label="Manage workspace"]')?.focus();
      }}><X size={14} aria-hidden="true" /></button></div>
  </aside>;
}
export function ReleaseSettings({ userId, focusRequest = 0 }: { userId?: string; focusRequest?: number }) {
  const release = useReleases(userId);
  const heading = useRef<HTMLHeadingElement>(null);
  useEffect(() => {
    if (focusRequest && release.owner) { heading.current?.focus(); heading.current?.scrollIntoView({ block: "nearest" }); }
  }, [focusRequest, release.owner]);
  if (!release.owner) return null;
  const data = release.data;
  return <section className="release-settings" aria-labelledby="workspace-updates">
    <h2 id="workspace-updates" ref={heading} tabIndex={-1}>Updates</h2>
    <p>{data?.installed_version ? `Installed: Dakia ${data.installed_version}` : "Installed version unavailable."}</p>
    {data?.status === "disabled" ? <p>Automatic release checks are off.</p>
      : data?.status === "unknown" ? <p>This build doesn’t identify a published release.</p>
      : release.isError || data?.status === "unavailable" ? <p>Couldn’t check for releases. Try again later.</p>
      : !data ? <p>Checking for releases…</p>
      : data.available ? <p>Dakia {data.available.version} is available. <a href={data.available.notes_url} target="_blank" rel="noreferrer">Release notes</a></p>
      : <p>You’re using the latest release.</p>}
    <p><a href="https://github.com/DakiaMail/dakia-web/blob/main/docs/upgrading.md" target="_blank" rel="noreferrer">Upgrade instructions</a></p>
  </section>;
}

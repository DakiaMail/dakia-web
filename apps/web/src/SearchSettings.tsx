import { ConfirmationButton } from "@dakia/ui";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { Search } from "lucide-react";
import { request } from "./api";

type Mode = "automatic" | "on" | "off";
type Policy = { mode: Mode; enabled: boolean; can_manage: boolean; resources: { memory_bytes: number | null; cpus: number | null } };
export const searchSettingsApi = {
  get: () => request<Policy>("/search/settings"),
  save: (mode: Mode) => request<Policy>("/search/settings", { method: "PUT", body: JSON.stringify({ mode }) }),
};
const options: { value: Mode; label: string; description: string }[] = [
  { value: "automatic", label: "Automatic", description: "Enable with at least 8 GB RAM and 4 CPUs allocated to Dakia." },
  { value: "on", label: "On", description: "Enable regardless of available resources. Indexing can slow down smaller servers." },
  { value: "off", label: "Off", description: "Use keyword search only." },
];
export function SearchSettings({ userId }: { userId: string }) {
  const client = useQueryClient();
  const key = ["search-settings", userId];
  const policy = useQuery({ queryKey: key, queryFn: searchSettingsApi.get, retry: false });
  const [draft, setDraft] = useState<Mode | null>(null);
  const save = useMutation({ mutationFn: searchSettingsApi.save, onSuccess: value => {
    client.setQueryData(key, value); setDraft(null);
    void client.invalidateQueries({ predicate: query => ["indexing", "mail", "search-page"].includes(String(query.queryKey[0])) });
  } });
  const selected = draft ?? policy.data?.mode;
  const resources = policy.data?.resources;
  return <section className="workspace-search" aria-labelledby="workspace-search">
    <div className="workspace-section-heading"><h2 id="workspace-search"><Search size={20} aria-hidden="true" />Search</h2></div>
    <p>Find emails by meaning, even when the words differ. Keyword search is always available.</p>
    {policy.data && <>
      <p role="status">Search by meaning is {policy.data.enabled ? "enabled" : "off"}.</p>
      <p>{resources?.memory_bytes != null && resources?.cpus != null
        ? `Allocated to Dakia: ${Number((resources.memory_bytes / 1_000_000_000).toFixed(2))} GB RAM and ${Number(resources.cpus.toFixed(2))} CPUs.`
        : "Available resources could not be determined. Automatic keeps search by meaning off."}</p>
      {policy.data.can_manage ? <>
        <fieldset className="translation-mode-options" disabled={save.isPending}><legend>Search by meaning</legend>
          {options.map(option => <label key={option.value}><input type="radio" name="search-mode" value={option.value} checked={selected === option.value} onChange={() => { setDraft(option.value); save.reset(); }} /><span>{option.label}<small>{option.description}</small></span></label>)}
        </fieldset>
        <ConfirmationButton status={save.status} disabled={draft === null || draft === policy.data.mode} onClick={() => { if (draft && !save.isPending) save.mutate(draft); }}>Save search settings</ConfirmationButton>
      </> : <p>{options.find(option => option.value === policy.data.mode)?.label}. Your administrator controls this setting.</p>}
    </>}
    {policy.isPending && <p role="status">Loading search settings…</p>}
    {policy.isError && <p role="alert">Could not load search settings. <button onClick={() => void policy.refetch()}>Retry</button></p>}
    {save.isError && <p role="alert">Could not save search settings. Try again.</p>}
  </section>;
}

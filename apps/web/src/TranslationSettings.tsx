import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";
import { Languages, Trash2 } from "lucide-react";
import { formatBytes, removePack, savedPacks, type TranslationMode } from "./deviceTranslation";
import { translationPolicyKey, translationSettingsApi } from "./translationPolicy";
const options: { value: TranslationMode; label: string; description: string }[] = [
  { value: "device", label: "On devices", description: "Translate on each person’s device. No host translation." },
  { value: "automatic", label: "On devices with host fallback", description: "Use the host if a device cannot finish a translation." },
  { value: "host", label: "On the host", description: "Translate on your Dakia host. No language downloads on devices." },
];
export function TranslationSettings({ userId }: { userId: string }) {
  const client = useQueryClient();
  const heading = useRef<HTMLHeadingElement>(null);
  const policy = useQuery({ queryKey: translationPolicyKey(userId), queryFn: translationSettingsApi.get, retry: false });
  const packs = useQuery({ queryKey: ["translation-packs"], queryFn: savedPacks, refetchOnMount: "always" });
  const [draft, setDraft] = useState<TranslationMode | null>(null);
  const [status, setStatus] = useState("");
  const save = useMutation({ mutationFn: translationSettingsApi.save, onSuccess: value => {
    client.setQueryData(translationPolicyKey(userId), value); setDraft(null); setStatus("Translation settings saved.");
  } });
  const remove = useMutation({ mutationFn: removePack, onSuccess: () => { void packs.refetch(); setStatus("Language removed from this browser."); requestAnimationFrame(() => heading.current?.focus()); } });
  const selected = draft ?? policy.data?.mode;
  return <section className="workspace-translation" aria-labelledby="workspace-translation">
    <div className="workspace-section-heading"><h2 ref={heading} tabIndex={-1} id="workspace-translation"><Languages size={20} aria-hidden="true" />Translation</h2></div>
    {policy.data?.can_manage ? <>
      <p>Choose where translations run for everyone using this Dakia host.</p>
      <fieldset className="translation-mode-options" disabled={save.isPending}><legend className="sr-only">Where translations run</legend>
        {options.map(option => <label key={option.value}><input type="radio" name="translation-mode" value={option.value} checked={selected === option.value} onChange={() => { setDraft(option.value); setStatus(""); }} /><span>{option.label}<small>{option.description}</small></span></label>)}
      </fieldset>
      <button className="primary" aria-disabled={save.isPending || draft === null} onClick={() => { if (draft && !save.isPending) save.mutate(draft); }}>{save.isPending ? "Saving…" : "Save translation settings"}</button>
    </> : policy.data && <p>{options.find(option => option.value === policy.data.mode)?.label}. Your administrator controls this setting.</p>}
    {policy.isError && <p role="alert">Could not load translation settings. <button onClick={() => void policy.refetch()}>Retry</button></p>}
    {(save.isError || remove.isError || packs.isError) && <p role="alert">Could not update translation settings or saved languages. Try again.</p>}
    {!!packs.data?.length && <div className="translation-saved-packs"><h3>Saved on this browser</h3><ul>{packs.data.map(pack => <li key={pack.source}><span>{pack.name}<small>{formatBytes(pack.bytes)}</small></span><button disabled={remove.isPending} aria-label={`Remove ${pack.name} language`} onClick={() => remove.mutate(pack.source)}><Trash2 size={16} aria-hidden="true" />Remove</button></li>)}</ul><p>Saved languages work offline. Clearing browser data removes them.</p></div>}
    <p role="status" className="translation-settings-status">{status}</p>
  </section>;
}

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Account, MailboxSettings } from "@dakia/api-types";
import { api } from "./api";
import { extraApi } from "./extra-api";
import { ErrorNotice, Modal } from "./ui";
export function MailboxSettingsDialog({ account, onClose }: { account: Account; onClose: () => void }) {
  const settings = useQuery({ queryKey: ["mailbox-settings", account.id], queryFn: () => api.mailboxSettings(account.id) });
  const folders = useQuery({ queryKey: ["folders", account.id], queryFn: () => extraApi.folders(account.id) });
  const [error, setError] = useState<unknown>();
  const [busy, setBusy] = useState(false);
  return <Modal title={`Mailbox settings: ${account.display_name}`} onClose={() => { if (!busy) onClose(); }}>
    <ErrorNotice error={error || settings.error || folders.error} />
    {settings.data && folders.data ? <form className="setup" onSubmit={async (event) => {
      event.preventDefault();
      const data = new FormData(event.currentTarget);
      const folder = (key: string) => String(data.get(key) || "") || null;
      const payload: MailboxSettings = {
        full_imap_exposure_confirmed: data.get("exposure") === "on",
        sent_folder: folder("sent_folder"), archive_folder: folder("archive_folder"), trash_folder: folder("trash_folder"),
        received_cache_budget: Number(data.get("cache")) * 1024 * 1024,
      };
      setBusy(true); setError(undefined);
      try { await api.configureMailbox(account.id, payload); onClose(); }
      catch (error) { setError(error); }
      finally { setBusy(false); }
    }}>
      {(["sent_folder", "archive_folder", "trash_folder"] as const).map((key) => <label key={key} htmlFor={`mailbox-${key}`}>
        {key === "sent_folder" ? "Sent folder" : key === "archive_folder" ? "Archive folder" : "Trash folder"}
        <select title={key === "sent_folder" ? "Sent folder" : key === "archive_folder" ? "Archive folder" : "Trash folder"} id={`mailbox-${key}`} name={key} defaultValue={settings.data[key] || ""} disabled={busy}>
          <option value="">Automatic discovery</option>
          {folders.data.map((folder) => <option key={folder} value={folder}>{folder}</option>)}
        </select>
      </label>)}
      {account.provider === "gmail" && <label className="checkbox-label">
        <input type="checkbox" name="exposure" defaultChecked={settings.data.full_imap_exposure_confirmed} disabled={busy} />
        I checked Gmail settings: all labels are shown in IMAP and folder message limits are disabled.
      </label>}
      <label>Received content cache (MiB)
        <input type="number" name="cache" min="1" max="4095" required defaultValue={Math.floor(settings.data.received_cache_budget / 1024 / 1024)} disabled={busy} />
      </label>
      <footer><button title="Save settings" className="primary" disabled={busy}>{busy ? "Saving…" : "Save settings"}</button></footer>
    </form> : <p role="status">Loading mailbox settings…</p>}
  </Modal>;
}

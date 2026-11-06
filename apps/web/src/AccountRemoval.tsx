import { useState } from "react";
import type { Account } from "@dakia/api-types";
import { api, ApiFailure } from "./api";
import { ErrorNotice, Modal } from "./ui";

export function AccountRemoval({ account, onClose, onRemoved }: {
  account: Account; onClose: () => void; onRemoved: () => void;
}) {
  const [confirmation, setConfirmation] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<unknown>();
  return <Modal title="Remove account from Dakia?" onClose={() => { if (!busy) onClose(); }}>
    <form className="setup" onSubmit={async (event) => {
      event.preventDefault();
      if (confirmation !== account.email || busy) return;
      setBusy(true); setError(undefined);
      try { await api.removeAccount(account.id, confirmation); onRemoved(); }
      catch (error) {
        setError(error instanceof ApiFailure && error.code === "account_has_pending_actions"
          ? new Error("This account has pending mail actions or an unresolved delivery. Cancel scheduled sends and resolve pending actions before removing it.") : error);
      } finally { setBusy(false); }
    }}>
      <p><strong>{account.display_name}</strong><br />{account.email}</p>
      <p>This removes the connection, cached messages, and local drafts and attachments for this account from Dakia. Your other accounts stay connected.</p>
      <p>Your email account and mail at the provider will not be deleted.</p>
      <label htmlFor="remove-account-confirmation">Type {account.email} to confirm
        <input id="remove-account-confirmation" autoComplete="off" spellCheck={false} value={confirmation} onChange={(event) => setConfirmation(event.target.value)} disabled={busy} />
      </label>
      <ErrorNotice error={error} />
      <footer>
        <button title="Keep account" type="button" disabled={busy} onClick={onClose}>Keep account</button>
        <button title="Remove from Dakia" className="destructive" disabled={busy || confirmation !== account.email}>{busy ? "Removing…" : "Remove from Dakia"}</button>
      </footer>
    </form>
  </Modal>;
}

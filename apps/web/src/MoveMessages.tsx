import { useState } from "react";
import { useQueries } from "@tanstack/react-query";
import type { Account, Message } from "@dakia/api-types";
import { extraApi } from "./extra-api";
import { ErrorNotice, Modal } from "./ui";

// Snapshot targets when opening the dialog. Folder names belong to accounts,
// never to the currently open preview or an unrelated account's namespace.
export function MoveMessages({ messages, accounts, onMove, onClose }: {
  messages: Message[];
  accounts: Account[];
  onMove: (messages: Message[], folders: Record<string, string>) => void;
  onClose: () => void;
}) {
  const ids = [...new Set(messages.map(message => message.account_id))];
  const folders = useQueries({ queries: ids.map(id => ({
    queryKey: ["folders", id], queryFn: () => extraApi.folders(id),
  })) });
  const [chosen, setChosen] = useState<Record<string, string>>({});
  const valid = ids.every((id, index) => folders[index].data?.includes(chosen[id]));
  return <Modal title="Move messages" onClose={onClose}>
    <form className="setup" onSubmit={event => {
      event.preventDefault();
      if (!valid) return;
      onMove(messages, chosen);
      onClose();
    }}>
      <p>{messages.length} selected messages. Choose a destination for each account.</p>
      {ids.map((id, index) => <div key={id}>
        <ErrorNotice error={folders[index].error} />
        {folders[index].isPending && <p role="status">Loading folders…</p>}
        <label>Folder for {accounts.find(account => account.id === id)?.display_name || id}
          <select title="Choose destination folder" required value={chosen[id] || ""} disabled={!folders[index].data} onChange={event => setChosen(old => ({ ...old, [id]: event.target.value }))}>
            <option value="" disabled>Choose folder</option>
            {folders[index].data?.map(folder => <option key={folder} value={folder}>{folder}</option>)}
          </select>
        </label>
        {folders[index].error && <button title="Retry folders" type="button" onClick={() => void folders[index].refetch()}>Retry folders</button>}
      </div>)}
      <button title="Move" className="primary" disabled={!valid}>Move</button>
    </form>
  </Modal>;
}

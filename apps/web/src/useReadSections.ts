import { useEffect, useLayoutEffect, useState } from "react";
import type { Message } from "@dakia/api-types";

export const readSectionExitMs = 240;
type HeldSection = { label: string; leaving: boolean };
const identity = (message: Message) => `${message.account_id}/${message.id}`;

// Read status stays live. Only the row's section is held while reading.
export function useReadSections(rows: Message[], selected: Message | null, scope: string, enabled: boolean) {
  const [held, setHeld] = useState<{ scope: string; entries: Record<string, HeldSection> }>({ scope, entries: {} });
  useLayoutEffect(() => {
    setHeld(old => {
      const entries = enabled && old.scope === scope ? { ...old.entries } : {};
      const selectedId = selected && identity(selected);
      for (const [id, entry] of Object.entries(entries)) {
        const row = rows.find(message => identity(message) === id);
        if (!row || row.unread) delete entries[id];
        else entries[id] = { ...entry, leaving: selectedId ? selectedId !== id : entry.leaving };
      }
      if (enabled && selected?.unread && selected.category_status === "ready" && selected.category?.trim()
        && rows.some(message => identity(message) === selectedId)) {
        entries[selectedId!] = { label: selected.category.trim().toLowerCase(), leaving: false };
      }
      if (old.scope === scope && JSON.stringify(old.entries) === JSON.stringify(entries)) return old;
      return { scope, entries };
    });
  }, [rows, selected, scope, enabled]);
  useEffect(() => {
    const leaving = Object.keys(held.entries).filter(id => held.entries[id].leaving);
    if (!leaving.length) return;
    const timer = setTimeout(() => setHeld(old => {
      if (old !== held) return old;
      return { ...old, entries: Object.fromEntries(Object.entries(old.entries).filter(([id]) => !leaving.includes(id))) };
    }), readSectionExitMs);
    return () => clearTimeout(timer);
  }, [held]);
  return enabled && held.scope === scope ? held.entries : {};
}

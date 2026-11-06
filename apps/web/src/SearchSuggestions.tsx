import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Message, SearchPage } from "@dakia/api-types";
import { request } from "./api";

// Typeahead is strictly index-only. Typing must not start an IMAP traversal or
// make a failed/empty partial index look like an exhaustive search result.
export function SearchSuggestions({ value, ids, onOpen }: { value: string, ids: string[], onOpen: (message: Message) => void }) {
  const [settled, setSettled] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value.trim()), 180);
    return () => clearTimeout(timer);
  }, [value]);
  const result = useQuery({
    queryKey: ["search-suggestions", settled, ids],
    enabled: settled.length >= 2 && ids.length > 0,
    queryFn: ({ signal }) => request<SearchPage>(`/search?${new URLSearchParams({ q: settled, accounts: ids.join(","), suggest: "true" })}`, { signal }),
    retry: false, staleTime: 15000, refetchOnWindowFocus: false,
  });
  if (settled !== value.trim() || !result.data?.rows.length) return null;
  return <div className="search-suggestions" aria-label="Indexed search suggestions">
    <small>Indexed matches</small>
    {result.data.rows.slice(0, 5).map(message => <button title={`Open ${message.subject || "message"}`} type="button" key={`${message.account_id}/${message.id}`} onClick={() => onOpen(message)}>
      <span>{message.subject || "(No subject)"}</span><small>{message.sender.name || message.sender.email}</small>
    </button>)}
  </div>;
}

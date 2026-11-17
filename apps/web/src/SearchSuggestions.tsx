import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { SearchPage } from "@dakia/api-types";
import { request } from "./api";

// Typing is index-only: it must never start a provider traversal or imply exhaustive coverage.
export function useSearchSuggestions(value: string, ids: string[], enabled: boolean) {
  const [settled, setSettled] = useState("");
  useEffect(() => {
    const timer = setTimeout(() => setSettled(value.trim()), 180);
    return () => clearTimeout(timer);
  }, [value]);
  const result = useQuery({
    queryKey: ["search-suggestions", settled, ids],
    enabled: enabled && settled.length >= 2 && ids.length > 0,
    queryFn: ({ signal }) => request<SearchPage>(`/search?${new URLSearchParams({ q: settled, accounts: ids.join(","), suggest: "true" })}`, { signal }),
    retry: false, staleTime: 15000, refetchOnWindowFocus: false,
  });
  const current = settled === value.trim();
  return { rows: current ? result.data?.rows || [] : [], loading: enabled && value.trim().length >= 2 && (!current || result.isFetching), error: current && result.isError };
}

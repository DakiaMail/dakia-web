import { localMailCache } from "./localMailCache";
import type { QueryClient } from "@tanstack/react-query";
import { queryOptions } from "@tanstack/react-query";
import { api } from "./api";

// Foreground and speculative reads must share cancellation, timeout and keys.
export function messageContent(account: string, id: string, client?: QueryClient) {
  return queryOptions({
    queryKey: ["content", account, id],
    queryFn: async ({ signal }) => {
      const controller = new AbortController();
      const cancel = () => controller.abort();
      signal.addEventListener("abort", cancel, { once: true });
      const timer = setTimeout(cancel, 30000);
      try {
        const cached = await localMailCache.body(account, id);
        signal.throwIfAborted();
        if (cached && client) {
          client.setQueryData(["content", account, id], cached, { updatedAt: 1 });
          if (!navigator.onLine) return cached;
        }
        const fresh = await api.contentById(account, id, controller.signal);
        localMailCache.verifiedBody(account, id);
        return fresh;
      } catch (error) {
        if (controller.signal.aborted && !signal.aborted) throw new Error("This message couldn’t be loaded. Please try again.");
        throw error;
      } finally {
        clearTimeout(timer);
        signal.removeEventListener("abort", cancel);
      }
    },
    networkMode: "always",
    staleTime: 30000,
    retry: false,
  });
}

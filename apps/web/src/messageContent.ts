import { queryOptions } from "@tanstack/react-query";
import { api } from "./api";

// Foreground and speculative reads must share cancellation, timeout and keys.
export function messageContent(account: string, id: string) {
  return queryOptions({
    queryKey: ["content", account, id],
    queryFn: async ({ signal }) => {
      const controller = new AbortController();
      const cancel = () => controller.abort();
      signal.addEventListener("abort", cancel, { once: true });
      const timer = setTimeout(cancel, 30000);
      try {
        return await api.contentById(account, id, controller.signal);
      } catch (error) {
        if (controller.signal.aborted && !signal.aborted) throw new Error("This message couldn’t be loaded. Please try again.");
        throw error;
      } finally {
        clearTimeout(timer);
        signal.removeEventListener("abort", cancel);
      }
    },
    staleTime: 30000,
    retry: false,
  });
}

import { localMailCache } from "./localMailCache";
import type { QueryClient } from "@tanstack/react-query";
import { queryOptions } from "@tanstack/react-query";
import { api } from "./api";

// Share requests with prefetch, but allow the server's complete 60-second read.
export function messageContent(account: string, id: string, client?: QueryClient) {
  return queryOptions({
    queryKey: ["content", account, id],
    queryFn: async ({ signal }) => {
      signal.throwIfAborted();
      const controller = new AbortController();
      const cacheController = new AbortController();
      const cancel = () => { cacheController.abort(); controller.abort(signal.reason); };
      signal.addEventListener("abort", cancel, { once: true });
      let rejectAbort!: () => void;
      const aborted = new Promise<never>((_, reject) => {
        rejectAbort = () => reject(controller.signal.reason);
        controller.signal.addEventListener("abort", rejectAbort, { once: true });
      });
      const timer = setTimeout(() => controller.abort(), 65000);
      let cacheTimer: ReturnType<typeof setTimeout> | undefined;
      try {
        const cached = await Promise.race([
          localMailCache.body(account, id, cacheController.signal).catch(() => undefined),
          new Promise<undefined>(resolve => {
            // Offline mail depends on this read; only online cache lookup is optional.
            if (navigator.onLine) cacheTimer = setTimeout(() => resolve(undefined), 250);
          }),
          aborted,
        ]);
        clearTimeout(cacheTimer); cacheController.abort();
        controller.signal.throwIfAborted();
        if (cached && client) {
          client.setQueryData(["content", account, id], cached, { updatedAt: 1 });
          if (!navigator.onLine) return cached;
        }
        const fresh = await Promise.race([api.contentById(account, id, controller.signal), aborted]);
        localMailCache.verifiedBody(account, id);
        return fresh;
      } catch (error) {
        if (controller.signal.aborted && !signal.aborted) throw new Error("This message couldn’t be loaded. Please try again.");
        throw error;
      } finally {
        clearTimeout(timer); clearTimeout(cacheTimer); cacheController.abort();
        signal.removeEventListener("abort", cancel);
        controller.signal.removeEventListener("abort", rejectAbort);
      }
    },
    networkMode: "always",
    staleTime: 30000,
    retry: false,
  });
}

import { useEffect, useRef } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { eventAffects } from "./mailEventPolicy";
import type { Account, Event as MailEvent } from "@dakia/api-types";
export function useMailEvents(accounts: Account[]) {
  const client = useQueryClient();
  const cursors = useRef(new Map<string, number>());
  const accountIds = accounts
    .map((a) => a.id)
    .sort()
    .join(",");
  useEffect(() => {
    const cleanups = accountIds
      .split(",")
      .filter(Boolean)
      .map((id) => {
        let source: EventSource | undefined;
        let stopped = false;
        let retry: ReturnType<typeof setTimeout>;
        let debounce: ReturnType<typeof setTimeout>;
        let failures = 0;
        let bootstrapped = false;
        let scheduled = false;
        let flushing = false;
        const kinds = new Set<string>(), targets = new Set<string>();
        const refresh = (kind: string, target = "") => {
          kinds.add(kind); targets.add(target);
          schedule();
        };
        const schedule = () => {
          if (scheduled || flushing || stopped) return;
          scheduled = true;
          debounce = setTimeout(() => {
            scheduled = false;
            flushing = true;
            const changes = new Set(kinds), ids = new Set(targets);
            kinds.clear(); targets.clear();
            void client.invalidateQueries({ predicate: query => eventAffects(query, id, changes, ids) }, { cancelRefetch: false }).finally(() => {
              flushing = false;
              // Do not lose the last event in a slow local snapshot request.
              if (kinds.size) schedule();
            });
          }, 2000);
        };
        const connect = () => {
          if (stopped) return;
          source = new EventSource(
            `/api/accounts/${encodeURIComponent(id)}/events?after=${cursors.current.get(id) || 0}`,
          );
          source.addEventListener("ready", (event) => {
            try {
              const ready = JSON.parse((event as MessageEvent).data) as {cursor: number; reset: boolean};
              if (!Number.isSafeInteger(ready.cursor) || ready.cursor < 0) return;
              const previous = cursors.current.get(id);
              cursors.current.set(id, ready.cursor);
              failures = 0;
              if (ready.reset && (!bootstrapped || previous !== ready.cursor)) refresh("reset");
              bootstrapped = true;
            } catch { /* Ignore malformed stream records. */ }
          });
          source.onmessage = (event) => {
            try {
              const e = JSON.parse(event.data) as MailEvent;
              if (
                e.account_id !== id ||
                !Number.isSafeInteger(e.cursor) ||
                e.cursor < 0
              )
                return;
              if (e.cursor <= (cursors.current.get(id) || 0)) return;
              cursors.current.set(id, e.cursor);
              failures = 0;
              refresh(e.kind, e.target_id);
            } catch { /* Malformed records must not trigger provider scans. */ }
          };
          source.addEventListener("refetch_required", () => {
            cursors.current.set(id, 0);
            refresh("reset");
            source?.close();
            retry = setTimeout(connect, 1000);
          });
          source.onerror = () => {
            source?.close();
            failures++;
            // A transport error is not a mail change. Keep the replay cursor.
            retry = setTimeout(
              connect,
              Math.min(30000, 1000 * 2 ** Math.min(failures, 5)),
            );
          };
        };
        connect();
        return () => {
          stopped = true;
          source?.close();
          clearTimeout(retry);
          clearTimeout(debounce);
        };
      });
    return () => cleanups.forEach((stop) => stop());
  }, [accountIds, client]);
}

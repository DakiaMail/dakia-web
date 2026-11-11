import { DateTimeSelector, type DateTimeSelectorProps } from "@dakia/ui";
import { useEffect, useRef } from "react";
import { useBackLayer } from "./appHistory";
import { useReloadGuard } from "./AppUpdate";

/** The library owns the picker; this adapter owns mail navigation and focus after removal. */
export function DateTimeDialog(props: Omit<DateTimeSelectorProps, "opened" | "withinPortal" | "returnFocus">) {
  const returnFocus = useRef(document.activeElement as HTMLElement | null);
  const restoration = useRef<number | undefined>(undefined);
  useReloadGuard();
  useBackLayer(props.onClose);
  useEffect(() => {
    if (restoration.current !== undefined) cancelAnimationFrame(restoration.current);
    return () => {
      restoration.current = requestAnimationFrame(() => {
        const original = returnFocus.current;
        if (original && original !== document.body && original.isConnected && !original.matches(":disabled") && original.getClientRects().length) {
          original.focus(); return;
        }
        document.querySelector<HTMLElement>('.mailbox .message-list[tabindex], .mailbox')?.focus();
      });
    };
  }, []);
  return <DateTimeSelector {...props} opened withinPortal={false} returnFocus={false} />;
}

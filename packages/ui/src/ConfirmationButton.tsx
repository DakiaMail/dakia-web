import { forwardRef, useEffect, useState } from "react";
import { Button, type ButtonProps } from "./Button";

export interface ConfirmationButtonProps extends Omit<ButtonProps, "children" | "loading" | "aria-label" | "aria-labelledby"> {
  children: string;
  status: "idle" | "pending" | "success" | "error";
  pendingLabel?: string;
  successLabel?: string;
}

/** Status follows the request, so a failed action can never show a confirmation. */
export const ConfirmationButton = forwardRef<HTMLButtonElement, ConfirmationButtonProps>(function ConfirmationButton(
  { children, status, pendingLabel = "Saving…", successLabel = "Saved", disabled, ...props }, ref,
) {
  const [confirmed, setConfirmed] = useState(false);
  useEffect(() => {
    if (status !== "success") { setConfirmed(false); return; }
    setConfirmed(true);
    const duration = Number(getComputedStyle(document.documentElement).getPropertyValue("--ui-confirmation-duration")) || 1800;
    const timer = window.setTimeout(() => setConfirmed(false), duration);
    return () => window.clearTimeout(timer);
  }, [status]);
  const pending = status === "pending";
  const state = pending ? "pending" : status === "success" && confirmed ? "success" : "idle";
  const label = state === "pending" ? pendingLabel : state === "success" ? successLabel : children;
  return <>
    <Button {...props} ref={ref} keepFocusable disabled={disabled || pending} aria-busy={pending || undefined} aria-label={label} data-confirmed={state === "success" || undefined}>
      <span className="dakia-confirmation-labels" data-state={state} aria-hidden="true">
        <span data-label="idle">{children}</span>
        <span data-label="pending">{pendingLabel}</span>
        <span data-label="success">{successLabel}</span>
      </span>
    </Button>
    <span className="dakia-sr-only" role="status" aria-live="polite" aria-atomic="true">{state === "idle" ? "" : label}</span>
  </>;
});

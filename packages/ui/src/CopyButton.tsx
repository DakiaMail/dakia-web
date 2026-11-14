import { forwardRef, useEffect, useRef, useState } from "react";
import { Button, type ButtonProps } from "./Button";

export interface CopyButtonProps extends Omit<ButtonProps, "children" | "onClick" | "loading" | "leftSection" | "rightSection"> {
  value: string;
  children?: string;
}

export const CopyButton = forwardRef<HTMLButtonElement, CopyButtonProps>(function CopyButton(
  { value, children = value, className = "", disabled, title = "Copy", ...props }, ref,
) {
  const [state, setState] = useState<"idle" | "pending" | "copied" | "error">("idle");
  const request = useRef(0);
  useEffect(() => {
    setState("idle");
    return () => { request.current++; };
  }, [value]);
  useEffect(() => {
    if (state !== "copied") return;
    const duration = Number(getComputedStyle(document.documentElement).getPropertyValue("--ui-confirmation-duration")) || 1800;
    const timer = setTimeout(() => setState("idle"), duration);
    return () => clearTimeout(timer);
  }, [state]);
  async function copy() {
    const token = ++request.current;
    setState("pending");
    try {
      await navigator.clipboard.writeText(value);
      if (token === request.current) setState("copied");
    } catch {
      if (token === request.current) setState("error");
    }
  }
  return <>
    <Button variant="secondary" {...props} ref={ref} className={`dakia-copy-button ${className}`} keepFocusable
      disabled={disabled || state === "pending"} aria-busy={state === "pending" || undefined} data-copy-state={state}
      title={state === "copied" ? "Copied" : state === "error" ? "Couldn’t copy. Try selecting the value." : title} onClick={() => void copy()}>
      {children}
      <span className="dakia-copy-icons" aria-hidden="true">
        <svg data-icon="copy" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7"><rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h3"/></svg>
        <svg data-icon="copied" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="m5 12 4 4L19 6"/></svg>
        <svg data-icon="error" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="m6 6 12 12M6 18 18 6"/></svg>
      </span>
    </Button>
    <span className="dakia-sr-only" role={state === "error" ? "alert" : "status"} aria-live={state === "error" ? "assertive" : "polite"} aria-atomic="true">{state === "copied" ? "Copied" : state === "error" ? "Couldn’t copy. Try selecting the value." : ""}</span>
  </>;
});

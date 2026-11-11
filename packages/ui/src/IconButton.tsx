import { ActionIcon, type ActionIconProps } from "@mantine/core";
import { forwardRef, type ComponentPropsWithoutRef } from "react";
import type { ButtonVariant } from "./Button";

export interface IconButtonProps extends Omit<ActionIconProps, "variant">, Omit<ComponentPropsWithoutRef<"button">, keyof ActionIconProps> {
  /** Required accessible name, also used as the native tooltip. */
  label: string;
  variant?: ButtonVariant;
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, children, type = "button", title = label, disabled, loading, onClick, "aria-disabled": ariaDisabled, ...props }, ref,
) {
  const blocked = disabled || loading || ariaDisabled === true || ariaDisabled === "true";
  return <ActionIcon {...props} ref={ref} type={type} title={title} aria-label={label}
    disabled={disabled} loading={loading} aria-busy={loading || undefined}
    aria-disabled={blocked || undefined} data-disabled={blocked || undefined}
    onClick={event => { if (blocked) { event.preventDefault(); return; } onClick?.(event); }}>
    <span aria-hidden="true" className="dakia-icon-button-content">{children}</span>
  </ActionIcon>;
});

import { Button as MantineButton, Loader, type ButtonProps as MantineButtonProps } from "@mantine/core";
import { forwardRef, type ComponentPropsWithoutRef } from "react";

export type ButtonVariant = "primary" | "secondary" | "tertiary";
export interface ButtonProps extends Omit<MantineButtonProps, "variant">, Omit<ComponentPropsWithoutRef<"button">, keyof MantineButtonProps> {
  variant?: ButtonVariant;
  /** Keep the control in the Tab order during asynchronous actions. */
  keepFocusable?: boolean;
  tone?: "default" | "danger";
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { disabled, loading, keepFocusable = false, onClick, type = "button", "aria-disabled": ariaDisabled, leftSection, children, tone = "default", ...props }, ref,
) {
  const blocked = disabled || loading || ariaDisabled === true || ariaDisabled === "true";
  return <MantineButton {...props} ref={ref} type={type}
    disabled={keepFocusable ? false : disabled}
    loading={keepFocusable ? false : loading}
    leftSection={leftSection}
    data-tone={tone}
    aria-disabled={blocked || undefined} aria-busy={loading || props["aria-busy"] || undefined}
    data-disabled={blocked || undefined}
    onClick={event => {
      if (blocked) { event.preventDefault(); return; }
      onClick?.(event);
    }}>
      {keepFocusable && loading ? <><span style={{ opacity: 0 }}>{children}</span><span className="dakia-button-loader" aria-hidden="true"><Loader size={14} color="currentColor" /></span></> : children}
    </MantineButton>;
});

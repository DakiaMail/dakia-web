import { Button as MantineButton, Loader, type ButtonProps as MantineButtonProps } from "@mantine/core";
import { forwardRef, type ComponentPropsWithoutRef } from "react";

export type ButtonVariant = "primary" | "secondary" | "subtle" | "danger";
export interface ButtonProps extends Omit<MantineButtonProps, "variant">, Omit<ComponentPropsWithoutRef<"button">, keyof MantineButtonProps> {
  variant?: ButtonVariant;
  /** Keep the control in the Tab order during asynchronous actions. */
  keepFocusable?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { disabled, loading, keepFocusable = false, onClick, type = "button", "aria-disabled": ariaDisabled, leftSection, ...props }, ref,
) {
  const blocked = disabled || loading || ariaDisabled === true || ariaDisabled === "true";
  return <MantineButton {...props} ref={ref} type={type}
    disabled={keepFocusable ? false : disabled}
    loading={keepFocusable ? false : loading}
    leftSection={keepFocusable && loading ? <Loader size="xs" color="currentColor" aria-hidden="true" /> : leftSection}
    aria-disabled={blocked || undefined} aria-busy={loading || props["aria-busy"] || undefined}
    data-disabled={blocked || undefined}
    onClick={event => {
      if (blocked) { event.preventDefault(); return; }
      onClick?.(event);
    }} />;
});

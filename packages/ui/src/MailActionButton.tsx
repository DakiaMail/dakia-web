import { forwardRef, type ReactNode } from "react";
import { Button, type ButtonProps } from "./Button";

export interface MailActionButtonProps extends Omit<ButtonProps, "children" | "variant" | "leftSection" | "rightSection"> {
  label: string;
  icon?: ReactNode;
  /** Pass the application's hint component so shortcut preferences stay respected. */
  shortcutHint?: ReactNode;
  /** Toolbar labels collapse in compact readers; menu labels stay visible. */
  collapseLabel?: boolean;
  placement?: "toolbar" | "menu";
}

export const MailActionButton = forwardRef<HTMLButtonElement, MailActionButtonProps>(function MailActionButton(
  { label, icon, shortcutHint, collapseLabel = false, placement = "toolbar", className, title = label, "aria-label": accessibleLabel = label, ...props }, ref,
) {
  return <Button {...props} ref={ref} variant="tertiary" title={title} aria-label={accessibleLabel}
    className={["dakia-mail-action", `dakia-mail-action-${placement}`, className].filter(Boolean).join(" ")}>
    {icon && <span className="dakia-mail-action-icon" aria-hidden="true">{icon}</span>}
    <span className={collapseLabel ? "dakia-mail-action-text dakia-mail-action-text-compact" : "dakia-mail-action-text"}>{label}</span>
    {shortcutHint}
  </Button>;
});

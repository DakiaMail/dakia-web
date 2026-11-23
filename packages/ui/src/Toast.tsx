import { Notification } from "@mantine/core";
import type { ReactNode } from "react";

type ToastProps = {
  children?: string;
  announcementKey?: string;
  error?: boolean;
  actions?: ReactNode;
};

/** Keep the live region mounted so assistive technology observes updates. */
export function Toast({ children, announcementKey, error = false, actions }: ToastProps) {
  return (
    <Notification className="dakia-toast" color={error ? "red" : undefined} role="presentation" withCloseButton={false} withBorder data-empty={!children || undefined}>
      <div role={children ? (error ? "alert" : "status") : undefined} aria-live={error ? "assertive" : "polite"} aria-atomic="true">
        {children && <span key={announcementKey}>{children}</span>}
      </div>
      {children && actions && <div className="dakia-toast-actions">{actions}</div>}
    </Notification>
  );
}

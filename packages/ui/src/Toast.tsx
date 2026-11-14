import { Notification } from "@mantine/core";

/** Keep mounted so assistive technology observes each new confirmation. */
export function Toast({ children, announcementKey, error = false }: { children?: string; announcementKey?: string; error?: boolean }) {
  return <div className="dakia-toast-container" role={error ? "alert" : "status"} aria-live={error ? "assertive" : "polite"} aria-atomic="true">
    {children && <Notification key={announcementKey} className="dakia-toast" color={error ? "red" : undefined} role="presentation" withCloseButton={false} withBorder>{children}</Notification>}
  </div>;
}

import { Notification } from "@mantine/core";

/** Keep mounted so assistive technology observes each new confirmation. */
export function Toast({ children, announcementKey }: { children?: string; announcementKey?: string }) {
  return <div className="dakia-toast-container" role="status" aria-live="polite" aria-atomic="true">
    {children && <Notification key={announcementKey} className="dakia-toast" role="presentation" withCloseButton={false} withBorder>{children}</Notification>}
  </div>;
}

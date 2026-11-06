import { useId, useState } from "react";
import { RefreshCw } from "lucide-react";
import { plainKey } from "./keyboard";

export function RefreshButton({ updating, failed, status, onRefresh }: {
  updating: boolean;
  failed: boolean;
  status: string;
  onRefresh: () => void;
}) {
  const tooltipId = useId();
  const [dismissed, setDismissed] = useState(false);
  return (
    <div className="header-refresh-control">
      <button title="Refresh mail"
        className={`header-refresh${updating ? " is-updating" : ""}${failed ? " has-error" : ""}`}
        aria-label="Refresh mail"
        aria-describedby={tooltipId}
        onClick={onRefresh}
        onMouseEnter={() => setDismissed(false)}
        onFocus={() => setDismissed(false)}
        onKeyDown={event => {
          if (event.key === "Escape" && plainKey(event.nativeEvent)) {
            setDismissed(true);
            event.stopPropagation();
          }
        }}
      >
        <RefreshCw size={17} aria-hidden="true" />
        {failed && <span className="refresh-error-mark" aria-hidden="true">!</span>}
      </button>
      <span id={tooltipId} role="tooltip" className={`refresh-tooltip${dismissed ? " dismissed" : ""}`}>{status}</span>
      <span className="sr-only" role="status">{status}</span>
    </div>
  );
}

import { useState } from "react";
import { Clock3, X } from "lucide-react";
import { DateTimeDialog } from "./DateTimeDialog";
import { sendTimeLabel } from "./sendLater";

export function SendLater({ value, disabled, onChange }: {
  value: string;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const trigger = <button className={`schedule-trigger ${value ? "selected" : ""}`} disabled={disabled} aria-haspopup="dialog" aria-expanded={open}
    aria-label={value ? `Change send time: ${sendTimeLabel(value)}` : "Send later"}
    title={value ? "Change send time" : "Send later"} onClick={() => setOpen(true)}>
    <Clock3 size={19} aria-hidden="true" />
    {value && <span>{sendTimeLabel(value)}</span>}
  </button>;
  return <div className="send-later">
    {trigger}
    {open && <DateTimeDialog title="Send later" confirmLabel="Set send time" value={value}
      onChoose={onChange} onClose={() => setOpen(false)} />}
    {value && <button className="schedule-clear" aria-label="Remove send time" title="Remove send time" disabled={disabled} onClick={() => onChange("")}><X size={14} /></button>}
  </div>;
}

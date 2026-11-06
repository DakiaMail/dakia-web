import { useState } from "react";
import * as Dialog from "@radix-ui/react-dialog";
import { ArrowLeft, CalendarDays, ChevronRight, Clock3, X } from "lucide-react";
import { localDateTime, sendLaterPresets, sendTimeLabel, validSendTime } from "./sendLater";
import { plainKey } from "./keyboard";

export function SendLater({ value, disabled, onChange }: {
  value: string;
  disabled: boolean;
  onChange: (value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [custom, setCustom] = useState(false);
  const [candidate, setCandidate] = useState("");
  const [error, setError] = useState("");
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  function choose(next: string) {
    if (!validSendTime(next)) {
      setError("Choose a valid future date and time.");
      return;
    }
    onChange(next);
    setOpen(false);
  }
  return <div className="send-later">
    <Dialog.Root open={open} onOpenChange={(next) => {
      setOpen(next);
      if (next) {
        setCustom(false);
        setCandidate(value || localDateTime(sendLaterPresets()[0].date));
        setError("");
      }
    }}>
      <Dialog.Trigger asChild>
        <button className={`schedule-trigger ${value ? "selected" : ""}`} disabled={disabled}
          aria-label={value ? `Change send time: ${sendTimeLabel(value)}` : "Send later"}
          title={value ? "Change send time" : "Send later"}>
          <Clock3 size={19} aria-hidden="true" />
          {value && <span>{sendTimeLabel(value)}</span>}
        </button>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay className="overlay schedule-overlay" />
        <Dialog.Content className="schedule-panel" aria-describedby="schedule-timezone"
          onEscapeKeyDown={event => { if (!plainKey(event) || event.repeat) event.preventDefault(); }}>
          <header>
            {custom && <button title="Back to send later options" aria-label="Back to send later options" onClick={() => { setCustom(false); setError(""); }}><ArrowLeft size={18} /></button>}
            <Dialog.Title>{custom ? "Pick date & time" : "Send later"}</Dialog.Title>
            <Dialog.Close asChild><button title="Close send later" aria-label="Close send later"><X size={18} /></button></Dialog.Close>
          </header>
          <Dialog.Description id="schedule-timezone" className="schedule-timezone">{timezone.replaceAll("_", " ")}</Dialog.Description>
          {custom ? <form className="schedule-custom" onSubmit={(event) => { event.preventDefault(); choose(candidate); }}>
            <label>Date<input type="date" required min={localDateTime(new Date()).slice(0, 10)} value={candidate.split("T")[0]} onChange={(event) => { setCandidate(`${event.target.value}T${candidate.split("T")[1] || "09:00"}`); setError(""); }} /></label>
            <label>Time<input type="time" required value={candidate.split("T")[1] || ""} onChange={(event) => { setCandidate(`${candidate.split("T")[0]}T${event.target.value}`); setError(""); }} /></label>
            {error && <p role="alert" className="error">{error}</p>}
            <button title="Set send time" className="primary" type="submit">Set send time</button>
          </form> : <div className="schedule-options">
            {sendLaterPresets().map(({ label, date }) => <button title={label} key={label} onClick={() => choose(localDateTime(date))}>
              <span>{label}</span><time dateTime={date.toISOString()}>{date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}</time>
            </button>)}
            <button title="Pick date & time" className="schedule-pick" onClick={() => { setCustom(true); setError(""); }}><CalendarDays size={18} /><span>Pick date & time</span><ChevronRight size={17} /></button>
            {error && <p role="alert" className="error">{error}</p>}
          </div>}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
    {value && <button className="schedule-clear" aria-label="Remove send time" title="Remove send time" disabled={disabled} onClick={() => onChange("")}><X size={14} /></button>}
  </div>;
}

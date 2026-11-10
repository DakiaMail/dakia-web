import { Modal, TextInput } from "@mantine/core";
import { useEffect, useId, useRef, useState } from "react";
import { Button } from "./Button";
import { IconButton } from "./IconButton";
import { dateTimePresets, localDateTime, validFutureDateTime } from "./dateTime";

export interface DateTimeSelectorProps {
  opened: boolean;
  title: string;
  confirmLabel: string;
  /** Local wall-clock value, YYYY-MM-DDTHH:mm. */
  value?: string;
  onChoose: (value: string) => void;
  onClose: () => void;
  /** Render inside an enclosing dialog's focus boundary when nesting. */
  withinPortal?: boolean;
  /** Disable when the host manages focus after an action removes its trigger. */
  returnFocus?: boolean;
}

export function DateTimeSelector({ opened, title, confirmLabel, value = "", onChoose, onClose,
  withinPortal = true, returnFocus = true }: DateTimeSelectorProps) {
  const [custom, setCustom] = useState(false);
  const [candidate, setCandidate] = useState(() => value || localDateTime(dateTimePresets()[0].date));
  const [error, setError] = useState("");
  const dateRef = useRef<HTMLInputElement>(null);
  const pickRef = useRef<HTMLButtonElement>(null);
  const descriptionId = useId();
  const errorId = useId();
  const wasOpened = useRef(false);
  const opener = useRef(document.activeElement as HTMLElement | null);
  useEffect(() => {
    if (opened && !wasOpened.current) {
      opener.current = document.activeElement as HTMLElement | null;
      setCustom(false);
      setCandidate(value || localDateTime(dateTimePresets()[0].date));
      setError("");
    }
    wasOpened.current = opened;
  }, [opened, value]);
  useEffect(() => { if (custom) dateRef.current?.focus(); }, [custom]);
  function close() {
    onClose();
    if (returnFocus) requestAnimationFrame(() => { if (opener.current?.isConnected) opener.current.focus(); });
  }
  function choose(next: string) {
    if (!validFutureDateTime(next)) {
      setError("Choose a valid future date and time.");
      return;
    }
    onChoose(next);
    close();
  }
  return <Modal.Root opened={opened} onClose={close} centered withinPortal={withinPortal}
    returnFocus={returnFocus} closeOnEscape={false} zIndex={31} size="352px" padding={0}
    transitionProps={{ duration: 0 }} classNames={{ inner: "dakia-datetime-inner", content: "dakia-datetime-panel" }}>
    <Modal.Overlay className="dakia-datetime-overlay" backgroundOpacity={0.2} />
    <Modal.Content data-dakia-date-time aria-describedby={descriptionId}
      onKeyDownCapture={event => {
        if (event.key !== "Escape") return;
        event.stopPropagation();
        if (!event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && !event.repeat && !event.nativeEvent.isComposing) {
          event.preventDefault(); close();
        }
      }}>
      <header className="dakia-datetime-header">
        {custom && <IconButton label="Back to time options" onClick={() => {
          setCustom(false); setError(""); requestAnimationFrame(() => pickRef.current?.focus());
        }}><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="m12 5-7 7 7 7M5 12h14" /></svg></IconButton>}
        <Modal.Title>{custom ? "Pick date & time" : title}</Modal.Title>
        <IconButton label="Close time options" onClick={close}><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="m6 6 12 12M18 6 6 18" /></svg></IconButton>
      </header>
      <p id={descriptionId} className="dakia-datetime-timezone">{Intl.DateTimeFormat().resolvedOptions().timeZone.replaceAll("_", " ")}</p>
      <Modal.Body>
        {custom ? <form className="dakia-datetime-custom" onSubmit={event => { event.preventDefault(); choose(candidate); }}>
          <TextInput ref={dateRef} label="Date" type="date" required withAsterisk={false}
            min={localDateTime(new Date()).slice(0, 10)} value={candidate.split("T")[0]}
            aria-describedby={error ? errorId : undefined}
            onChange={event => { setCandidate(`${event.currentTarget.value}T${candidate.split("T")[1] || "09:00"}`); setError(""); }} />
          <TextInput label="Time" type="time" required withAsterisk={false} value={candidate.split("T")[1] || ""}
            aria-describedby={error ? errorId : undefined}
            onChange={event => { setCandidate(`${candidate.split("T")[0]}T${event.currentTarget.value}`); setError(""); }} />
          {error && <p id={errorId} role="alert" className="dakia-datetime-error">{error}</p>}
          <Button type="submit" fullWidth title={confirmLabel}>{confirmLabel}</Button>
        </form> : <div className="dakia-datetime-options">
          {dateTimePresets().map(({ label, date }, index) => <Button key={label} variant="subtle" fullWidth
            data-autofocus={index === 0 || undefined} title={label} onClick={() => choose(localDateTime(date))}>
            <span>{label}</span><time dateTime={date.toISOString()}>{date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}</time>
          </Button>)}
          <Button ref={pickRef} variant="subtle" fullWidth className="dakia-datetime-pick" title="Pick date & time"
            onClick={() => { setCustom(true); setError(""); }}>Pick date & time<span aria-hidden="true">›</span></Button>
          {error && <p role="alert" className="dakia-datetime-error">{error}</p>}
        </div>}
      </Modal.Body>
    </Modal.Content>
  </Modal.Root>;
}

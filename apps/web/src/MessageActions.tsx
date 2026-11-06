import { useEffect, useId, useRef, useState } from "react";
import { Archive, Clock, CornerUpRight, Mail, MoreHorizontal, Reply, ShieldAlert, Star, Trash2 } from "lucide-react";
import { plainKey } from "./keyboard";

interface Props {
  reply?: () => void;
  replyReady?: boolean;
  archive: () => void;
  unread: () => void;
  read: () => void;
  star: () => void;
  unstar: () => void;
  trash: () => void;
  spam: () => void;
  snooze: () => void;
  move: () => void;
  category?: string;
  categories: string[];
  setCategory: (value: string) => void;
  pending: boolean;
}

export function MessageActions(props: Props) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const id = useId();
  function close(restore = false) {
    setOpen(false);
    if (restore) trigger.current?.focus();
  }
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    const resize = () => setOpen(false);
    document.addEventListener("pointerdown", outside);
    window.addEventListener("resize", resize);
    panel.current?.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    return () => {
      document.removeEventListener("pointerdown", outside);
      window.removeEventListener("resize", resize);
    };
  }, [open]);
  const extras = <>
    <button aria-label="Move" title="Move" disabled={props.pending} onClick={() => { close(true); props.move(); }}><CornerUpRight size={18} />Move</button>
    <button aria-label="Mark as spam" title="Mark as spam" disabled={props.pending} onClick={() => { close(true); props.spam(); }}><ShieldAlert size={18} />Mark as spam</button>
    {props.category !== undefined && <label>Category <select aria-label="Category" title="Change category" value={props.category} disabled={props.pending} onChange={event => { props.setCategory(event.target.value); close(true); }}>
      <option value="" disabled>Category</option>
      {props.categories.map(category => <option key={category}>{category}</option>)}
    </select></label>}
    {props.reply && <button aria-label="Reply to selected message" title="Reply" disabled={props.pending || !props.replyReady} onClick={() => { close(true); props.reply?.(); }}><Reply size={18} />Reply</button>}
    <button aria-label="Mark read" title="Mark read" disabled={props.pending} onClick={() => { close(true); props.read(); }}><Mail size={18} />Mark read</button>
    <button aria-label="Star selected messages" title="Star" disabled={props.pending} onClick={() => { close(true); props.star(); }}><Star size={18} fill="currentColor" />Star</button>
    <button aria-label="Unstar selected messages" title="Unstar" disabled={props.pending} onClick={() => { close(true); props.unstar(); }}><Star size={18} />Unstar</button>
  </>;
  return <div className="reader-toolbar" aria-label="Message actions">
    <button aria-label="Archive" title="Archive" disabled={props.pending} onClick={props.archive}><Archive size={18} /><span className="message-action-label">Archive</span></button>
    <button aria-label="Mark unread" title="Mark unread" disabled={props.pending} onClick={props.unread}><Mail size={18} /><span className="message-action-label">Mark unread</span></button>
    <button aria-label="Move to trash" title="Move to trash" disabled={props.pending} onClick={props.trash}><Trash2 size={18} /><span className="message-action-label">Delete</span></button>
    <button aria-label="Snooze" title="Snooze" disabled={props.pending} onClick={props.snooze}><Clock size={18} /><span className="message-action-label">Snooze</span></button>
    <div className="message-actions-more" ref={root} onBlur={event => {
      if (!event.currentTarget.contains(event.relatedTarget as Node)) close();
    }} onKeyDown={event => {
      if (!plainKey(event.nativeEvent)) return;
      if (event.key === "Escape" && open) { event.preventDefault(); event.stopPropagation(); close(true); }
      if (!open || !["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key) || event.target instanceof HTMLSelectElement) return;
      event.preventDefault();
      const controls = Array.from(panel.current?.querySelectorAll<HTMLElement>("button:not(:disabled), select:not(:disabled)") || []);
      const index = controls.indexOf(document.activeElement as HTMLElement);
      const next = event.key === "Home" ? 0 : event.key === "End" ? controls.length - 1 : (index + (event.key === "ArrowDown" ? 1 : -1) + controls.length) % controls.length;
      controls[next]?.focus();
    }}>
      <button ref={trigger} aria-label="More actions" title="More actions" aria-expanded={open} aria-controls={id} disabled={props.pending} onClick={() => setOpen(value => !value)}><MoreHorizontal size={18} /><span className="message-action-label">More</span></button>
      {open && <div id={id} ref={panel} className="message-actions-popover" role="group" aria-label="More message actions">{extras}</div>}
    </div>

  </div>;
}

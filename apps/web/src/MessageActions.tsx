import { Button } from "@dakia/ui";
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { Archive, Clock, FolderInput, Mail, MoreHorizontal, ShieldAlert, Star, Trash2 } from "lucide-react";
import { KeyHint } from "./KeyboardHints";
import { plainKey } from "./keyboard";
import type { Account, Message } from "@dakia/api-types";
import { SenderBlock, type SenderAction } from "./BlockedSenders";

interface Props {
  sender?: Message;
  accounts?: Account[];
  onCleanup?: (rows: Message[]) => void;
  archive: () => void;
  unread: () => void;
  read: () => void;
  starred: boolean;
  toggleStar: () => void;
  trash: () => void;
  spam: () => void;
  snooze: () => void;
  restore?: () => void;
  move: () => void;
  category?: string;
  categories: string[];
  setCategory: (value: string) => void;
  pending: boolean;
}

export function MessageActions(props: Props) {
  return props.sender ? <SenderBlock key={`${props.sender.account_id}/${props.sender.id}`} message={props.sender} accounts={props.accounts || []} onCleanup={props.onCleanup}>
    {senderAction => <ActionToolbar {...props} senderAction={senderAction} />}
  </SenderBlock> : <ActionToolbar {...props} />;
}

function ActionToolbar(props: Props & { senderAction?: SenderAction }) {
  const extras = (close: () => void) => <>
    {props.restore && <button aria-label="Snooze" title="Snooze" disabled={props.pending} onClick={() => { close(); props.snooze(); }}><Clock size={18} />Snooze<KeyHint keys="B" /></button>}
    <button aria-label="Move" title="Move" disabled={props.pending} onClick={() => { close(); props.move(); }}><FolderInput size={18} />Move<KeyHint keys="M" /></button>
    <button aria-label="Mark as spam" title="Mark as spam" disabled={props.pending} onClick={() => { close(); props.spam(); }}><ShieldAlert size={18} />Mark as spam<KeyHint keys="!" /></button>
    {props.category !== undefined && <label>Category <select aria-label="Category" title="Change category" value={props.category} disabled={props.pending} onChange={event => { props.setCategory(event.target.value); close(); }}>
      <option value="" disabled>Category</option>
      {props.categories.map(category => <option key={category}>{category}</option>)}
    </select></label>}
    <button aria-label="Mark read" title="Mark read" disabled={props.pending} onClick={() => { close(); props.read(); }}><Mail size={18} />Mark read<KeyHint keys="Shift+U" /></button>
    <button aria-label={props.starred ? "Unstar selected messages" : "Star selected messages"} title={props.starred ? "Unstar" : "Star"} disabled={props.pending} onClick={() => { close(); props.toggleStar(); }}><Star size={18} fill={props.starred ? "currentColor" : "none"} />{props.starred ? "Unstar" : "Star"}<KeyHint keys="S" /></button>
    {props.senderAction?.(close)}
  </>;
  return <div className="reader-toolbar" aria-label="Message actions">
    <button aria-label="Archive" title="Archive" disabled={props.pending} onClick={props.archive}><Archive size={18} /><span className="message-action-label">Archive</span><KeyHint keys="E" /></button>
    <button aria-label="Mark unread" title="Mark unread" disabled={props.pending} onClick={props.unread}><Mail size={18} /><span className="message-action-label">Mark unread</span><KeyHint keys="U" /></button>
    <button aria-label="Move to trash" title="Move to trash" disabled={props.pending} onClick={props.trash}><Trash2 size={18} /><span className="message-action-label">Delete</span><KeyHint keys="Delete" /></button>
    {props.restore ? <Button className="restore-inbox-button" variant="subtle" aria-label="Move back to Inbox" title="Move back to Inbox" disabled={props.pending} onClick={props.restore}><FolderInput size={18} /><span>Move back to Inbox</span></Button> : <button aria-label="Snooze" title="Snooze" disabled={props.pending} onClick={props.snooze}><Clock size={18} /><span className="message-action-label">Snooze</span><KeyHint keys="B" /></button>}
    <MoreActions disabled={props.pending}>{close => extras(close)}</MoreActions>

  </div>;
}

function MoreActions({ children, disabled = false }: {
  children: (close: () => void) => ReactNode;
  disabled?: boolean;
}) {
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
  return <div className="message-actions-more" ref={root} onBlur={event => {
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
      <button ref={trigger} aria-label="More actions" title="More actions" aria-expanded={open} aria-controls={id} disabled={disabled} onClick={() => setOpen(value => !value)}><MoreHorizontal size={18} /><span className="message-action-label">More</span></button>
      {open && <div id={id} ref={panel} className="message-actions-popover" role="group" aria-label="More message actions">{children(() => close(true))}</div>}
    </div>;
}

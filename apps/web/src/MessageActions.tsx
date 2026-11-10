import { MailActionButton } from "@dakia/ui";
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
    {props.restore && <MailActionButton placement="menu" label="Snooze" icon={<Clock size={18} />} shortcutHint={<KeyHint keys="B" />} disabled={props.pending} onClick={() => { close(); props.snooze(); }} />}
    <MailActionButton placement="menu" label="Move" icon={<FolderInput size={18} />} shortcutHint={<KeyHint keys="M" />} disabled={props.pending} onClick={() => { close(); props.move(); }} />
    <MailActionButton placement="menu" label="Mark as spam" icon={<ShieldAlert size={18} />} shortcutHint={<KeyHint keys="!" />} disabled={props.pending} onClick={() => { close(); props.spam(); }} />
    {props.category !== undefined && <label>Category <select aria-label="Category" title="Change category" value={props.category} disabled={props.pending} onChange={event => { props.setCategory(event.target.value); close(); }}>
      <option value="" disabled>Category</option>
      {props.categories.map(category => <option key={category}>{category}</option>)}
    </select></label>}
    <MailActionButton placement="menu" label="Mark read" icon={<Mail size={18} />} shortcutHint={<KeyHint keys="Shift+U" />} disabled={props.pending} onClick={() => { close(); props.read(); }} />
    <MailActionButton placement="menu" label={props.starred ? "Unstar" : "Star"} aria-label={props.starred ? "Unstar selected messages" : "Star selected messages"} icon={<Star size={18} fill={props.starred ? "currentColor" : "none"} />} shortcutHint={<KeyHint keys="S" />} disabled={props.pending} onClick={() => { close(); props.toggleStar(); }} />
    {props.senderAction?.(close)}
  </>;
  return <div className="reader-toolbar" aria-label="Message actions">
    <MailActionButton label="Archive" icon={<Archive size={18} />} collapseLabel shortcutHint={<KeyHint keys="E" />} disabled={props.pending} onClick={props.archive} />
    <MailActionButton label="Mark unread" icon={<Mail size={18} />} collapseLabel shortcutHint={<KeyHint keys="U" />} disabled={props.pending} onClick={props.unread} />
    <MailActionButton label="Delete" aria-label="Move to trash" title="Move to trash" icon={<Trash2 size={18} />} collapseLabel shortcutHint={<KeyHint keys="Delete" />} disabled={props.pending} onClick={props.trash} />
    {props.restore ? <MailActionButton className="restore-inbox-button" label="Move back to Inbox" icon={<FolderInput size={18} />} disabled={props.pending} onClick={props.restore} /> : <MailActionButton label="Snooze" icon={<Clock size={18} />} collapseLabel shortcutHint={<KeyHint keys="B" />} disabled={props.pending} onClick={props.snooze} />}
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
      <MailActionButton ref={trigger} label="More" aria-label="More actions" title="More actions" collapseLabel icon={<MoreHorizontal size={18} />} aria-expanded={open} aria-controls={id} disabled={disabled} onClick={() => setOpen(value => !value)} />
      {open && <div id={id} ref={panel} className="message-actions-popover" role="group" aria-label="More message actions">{children(() => close(true))}</div>}
    </div>;
}

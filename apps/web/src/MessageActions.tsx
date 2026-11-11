import { DropdownMenu, DropdownMenuItem, DropdownSubmenu, MailActionButton } from "@dakia/ui";
import { Archive, Clock, FolderInput, Mail, MoreHorizontal, ShieldAlert, Star, Trash2 } from "lucide-react";
import { KeyHint } from "./KeyboardHints";
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
  const extras = <>
    {props.restore && <DropdownMenuItem label="Snooze" icon={<Clock size={18} />} shortcutHint={<KeyHint keys="B" />} disabled={props.pending} onClick={props.snooze} />}
    <DropdownMenuItem label="Move" icon={<FolderInput size={18} />} shortcutHint={<KeyHint keys="M" />} disabled={props.pending} onClick={props.move} />
    <DropdownMenuItem label="Mark as spam" icon={<ShieldAlert size={18} />} shortcutHint={<KeyHint keys="!" />} disabled={props.pending} onClick={props.spam} />
    {props.category !== undefined && <DropdownSubmenu label="Category" disabled={props.pending}>
      {props.categories.map(category => <DropdownMenuItem key={category} label={category.charAt(0).toUpperCase() + category.slice(1)} checked={category === props.category} disabled={props.pending} onClick={() => props.setCategory(category)} />)}
    </DropdownSubmenu>}
    <DropdownMenuItem label="Mark read" icon={<Mail size={18} />} shortcutHint={<KeyHint keys="Shift+U" />} disabled={props.pending} onClick={props.read} />
    <DropdownMenuItem label={props.starred ? "Unstar" : "Star"} aria-label={props.starred ? "Unstar selected messages" : "Star selected messages"} icon={<Star size={18} fill={props.starred ? "currentColor" : "none"} />} shortcutHint={<KeyHint keys="S" />} disabled={props.pending} onClick={props.toggleStar} />
    {props.senderAction?.()}
  </>;
  return <div className="reader-toolbar" aria-label="Message actions">
    <MailActionButton label="Archive" icon={<Archive size={18} />} collapseLabel shortcutHint={<KeyHint keys="E" />} disabled={props.pending} onClick={props.archive} />
    <MailActionButton label="Mark unread" icon={<Mail size={18} />} collapseLabel shortcutHint={<KeyHint keys="U" />} disabled={props.pending} onClick={props.unread} />
    <MailActionButton label="Delete" aria-label="Move to trash" title="Move to trash" icon={<Trash2 size={18} />} collapseLabel shortcutHint={<KeyHint keys="Delete" />} disabled={props.pending} onClick={props.trash} />
    {props.restore ? <MailActionButton className="restore-inbox-button" label="Move back to Inbox" icon={<FolderInput size={18} />} disabled={props.pending} onClick={props.restore} /> : <MailActionButton label="Snooze" icon={<Clock size={18} />} collapseLabel shortcutHint={<KeyHint keys="B" />} disabled={props.pending} onClick={props.snooze} />}
    <div className="message-actions-more"><DropdownMenu label="More message actions" disabled={props.pending}
      trigger={<MailActionButton label="More" aria-label="More actions" title="More actions" collapseLabel icon={<MoreHorizontal size={18} />} disabled={props.pending} />}>
      {extras}
    </DropdownMenu></div>

  </div>;
}

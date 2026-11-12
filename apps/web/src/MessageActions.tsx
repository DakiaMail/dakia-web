import { DropdownMenu, DropdownMenuItem, DropdownSubmenu, MailActionButton } from "@dakia/ui";
import { Archive, Clock, FolderInput, Mail, MoreHorizontal, ShieldAlert, Star, Trash2 } from "lucide-react";
import { KeyHint } from "./KeyboardHints";
import { classification, emailTypes, needsAttention } from "./classification";
import type { Change, DetectedType } from "@dakia/api-types";
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
  messages: Message[];
  classify: (change: Change) => void;
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
    {<>
      <DropdownSubmenu label="Personal or Automated" disabled={props.pending}>
        {(["personal", "automated"] as const).map(value => <DropdownMenuItem key={value} label={value === "personal" ? "Personal" : "Automated"} checked={props.messages.every(m => classification(m).kind === value)} onClick={() => props.classify({ kind: "inbox_kind", value, sender: false })} />)}
        <DropdownMenuItem label="Use automatic classification" onClick={() => props.classify({ kind: "inbox_kind", value: null, sender: false })} />
        {props.messages.length === 1 && <>
          <DropdownMenuItem label="Always Personal for this sender" onClick={() => props.classify({ kind: "inbox_kind", value: "personal", sender: true })} />
          <DropdownMenuItem label="Always Automated for this sender" onClick={() => props.classify({ kind: "inbox_kind", value: "automated", sender: true })} />
          <DropdownMenuItem label="Forget sender preference" onClick={() => props.classify({ kind: "inbox_kind", value: null, sender: true })} />
        </>}
      </DropdownSubmenu>
      <DropdownSubmenu label="Detected types" disabled={props.pending}>
        {Object.entries(emailTypes).map(([value,label]) => { const present = props.messages.every(m => classification(m).types.includes(value as DetectedType)) || false; return <DropdownMenuItem key={value} label={label} checkRole="menuitemcheckbox" checked={present} onClick={() => props.classify({ kind: "detected_type", value: value as DetectedType, present: !present })} />; })}
      </DropdownSubmenu>
      <DropdownMenuItem label={props.messages.every(m => needsAttention(m)) ? "Dismiss attention" : "Needs attention"} disabled={props.pending} onClick={() => props.classify({ kind: "attention", value: !props.messages.every(m => needsAttention(m)) })} />
    </>}
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

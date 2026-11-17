import { messageContent } from "./messageContent";
import { useEffect, useId, useState } from "react";
import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, CornerUpRight, FileText, Reply, ReplyAll } from "lucide-react";
import { CopyButton } from "@dakia/ui";
import type { Account, Address, Draft, Message, MessageContent } from "@dakia/api-types";
import { api } from "./api";
import { KeyHint } from "./KeyboardHints";
import { ErrorNotice } from "./ui";
import { plainKey } from "./keyboard";
import { Translation } from "./Translation";
import { Unsubscribe } from "./Unsubscribe";

import type { ReplyMode } from "./reply";
import "./conversation.css";

type Props = {
  selected: Message;
  accounts: Account[];
  dark: boolean;
  alwaysLoadImages: boolean;
  blockHiddenImages: boolean;
  onReply: (mode: ReplyMode, content: MessageContent) => void;
  onDraft: (draft: Draft) => void;
  onCleanup?: (rows: Message[]) => void;
  cleanupRows?: Record<string, Message>;

};
export function Conversation(props: Props) {
  const { selected } = props;
  const cleanupRows = props.cleanupRows || {};

  const snapshot = useQuery({
    queryKey: ["conversation-snapshot", selected.account_id, selected.id],
    queryFn: ({ signal }) => api.conversation(selected, null, signal, "snapshot"),
    staleTime: 30000,
    retry: false,
  });
  const thread = useInfiniteQuery({
    queryKey: ["conversation", selected.account_id, selected.id],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) => api.conversation(selected, pageParam, signal),
    getNextPageParam: p => p.continuation || undefined,
    staleTime: 30000,
    // SSE refreshes saved conversation rows. Provider discovery runs on open
    // never repeatedly while someone reads a message.
    refetchInterval: false,
    refetchOnWindowFocus: false,
    retry: false,
  });
  useEffect(() => {
    if (thread.hasNextPage && !thread.isFetching && !thread.isError) void thread.fetchNextPage();
  }, [thread.hasNextPage, thread.isFetching, thread.isError, thread.fetchNextPage]);
  // The final local read catches events during slow verification without
  // restarting provider work. SSE invalidates only the local snapshot key.
  useEffect(() => {
    if (thread.dataUpdatedAt) {
      void snapshot.refetch();
    }
  }, [thread.dataUpdatedAt, snapshot.refetch]);

  const discovered = thread.data?.pages.at(-1);
  const last = snapshot.data || discovered;
  const seen = new Set<string>();
  // The API returns a cumulative chronological snapshot. Keep the selected
  // message available during discovery/failure, without leaking another account.
  const rows = (last?.rows || [selected]).filter(m => {
    if (m.account_id !== selected.account_id || seen.has(m.id) || (m.id !== selected.id && cleanupRows[`${m.account_id}/${m.id}`])) return false;
    seen.add(m.id); return true;
  });
  return <article className="message conversation" aria-label="Conversation">
    <div className="conversation-title"><h2 dir="auto">{selected.subject || "(No subject)"}</h2>
    </div>

    <div className="conversation-messages">
      {rows.map((message, i) => <ConversationMessage key={`${message.account_id}/${message.id}`} {...props} message={message}
        initiallyOpen={message.id === selected.id || (!!last?.complete && i === rows.length - 1)} multiple={rows.length > 1} />)}
    </div>
  </article>;
}
function ConversationMessage({ message, initiallyOpen, multiple, ...props }: Props & { message: Message; initiallyOpen: boolean; multiple: boolean }) {
  // Automatic expansion follows the settled snapshot, not each temporary last
  // row. An explicit user toggle remains authoritative across reconciliation.
  const [userExpanded, setUserExpanded] = useState<boolean | null>(null);
  const expanded = userExpanded ?? initiallyOpen;
  const client = useQueryClient();
  const content = useQuery({
    ...messageContent(message.account_id, message.id, client),
    enabled: expanded,
    staleTime: 30000,
    retry: false,
  });
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    setSlow(false);
    if (!expanded || !content.isPending) return;
    const timer = setTimeout(() => setSlow(true), 10000);
    return () => clearTimeout(timer);
  }, [expanded, content.isPending]);
  const account = props.accounts.find(a => a.id === message.account_id);
  const self = account?.email.toLowerCase() === message.sender.email.toLowerCase();
  const sender = self ? "You" : message.sender.name || message.sender.email || "Unknown sender";
  const region = useId();
  const addresses = (values: Address[]) => values.map(a => a.name ? `${a.name} <${a.email}>` : a.email).join(", ");
  return <section className={`conversation-message ${expanded ? "expanded" : "collapsed"}`} aria-label={`Message from ${sender}`}>
    <header className="conversation-summary">
      <span className={`avatar ${self ? "self-avatar" : ""}`} aria-hidden="true">{sender.slice(0, 2).toUpperCase()}</span>
      <span className="conversation-sender"><b dir="auto">{sender}</b>{!expanded && <small dir="auto">{message.preview || message.subject}</small>}</span>
      <time dateTime={message.received_at}>{new Date(message.received_at).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", year: "numeric" })}</time>
      <button title={expanded ? "Collapse message" : "Expand message"} aria-label={expanded ? "Collapse message" : "Expand message"} className="conversation-toggle" aria-expanded={expanded} aria-controls={region} onClick={() => setUserExpanded(!expanded)}>
        <ChevronDown className="conversation-chevron" size={16} aria-hidden="true" />
      </button>
      {expanded && <details className="conversation-details" onKeyDown={event => {
        if (event.key !== "Escape" || !plainKey(event.nativeEvent) || event.repeat || !event.currentTarget.open || window.getSelection()?.toString()) return;
        event.preventDefault(); event.stopPropagation();
        event.currentTarget.open = false;
        event.currentTarget.querySelector("summary")?.focus();
      }}>
        <summary aria-label="Message details"><span dir="auto">to {message.to.map(a => a.email.toLowerCase() === account?.email.toLowerCase() ? "you" : a.name || a.email).join(", ") || "undisclosed recipients"}</span><ChevronDown size={12} aria-hidden="true" /></summary>
        <dl>
          <dt>From</dt><dd><span dir="auto">{addresses([message.sender])}</span>{message.sender.email && <CopyButton value={message.sender.email} aria-label="Copy sender email address" title="Copy sender email address">Copy</CopyButton>}</dd>
          <dt>To</dt><dd dir="auto">{addresses(message.to) || "Undisclosed recipients"}</dd>
          {!!content.data?.cc.length && <><dt>Cc</dt><dd dir="auto">{addresses(content.data.cc)}</dd></>}
          {!!content.data?.reply_to.length && <><dt>Reply to</dt><dd dir="auto">{addresses(content.data.reply_to)}</dd></>}
          <dt>Date</dt><dd>{new Date(message.received_at).toLocaleString()}</dd>
          <dt>Subject</dt><dd dir="auto">{message.subject || "(No subject)"}</dd>
          {content.data?.message_id && <><dt>Message ID</dt><dd dir="auto">{content.data.message_id}</dd></>}
        </dl>
      </details>}
    </header>
    <div id={region} hidden={!expanded} className="conversation-body">
      <ErrorNotice error={content.error} />
      {content.isError && <button title="Retry message" onClick={() => void content.refetch()}>Retry message</button>}
      {expanded && content.isPending && (slow
        ? <p role="status">This message is taking longer to load.</p>
        : <div className="message-body-placeholder" role="status" aria-label="Loading message body"><span /><span /><span /></div>)}
      {expanded && content.data && <>
        <Unsubscribe message={message} account={account} onDraft={props.onDraft} onCleanup={props.onCleanup} />
        <Translation message={message} html={content.data.html} text={content.data.text} detectedLanguage={content.data.detected_language} dark={props.dark} alwaysLoadImages={props.alwaysLoadImages} blockHiddenImages={props.blockHiddenImages} />
        <div className="attachments">{content.data.attachments.map(a => <a title={`Download ${a.filename}`} className="attachment" href={api.download(message.account_id, a.id)} key={a.id} download={a.filename}><FileText size={18} />{a.filename}<small>{Math.ceil(a.size_bytes / 1024)} KB</small></a>)}</div>
        <div className="reply-actions" aria-label={multiple ? `Actions for ${sender}’s message` : "Message actions"}>
          <button title="Reply" onClick={() => props.onReply("reply", { ...content.data!, message })}><Reply size={17} />Reply{message.id === props.selected.id && <KeyHint keys="R" />}</button>
          <button title="Reply all" onClick={() => props.onReply("all", { ...content.data!, message })}><ReplyAll size={17} />Reply all{message.id === props.selected.id && <KeyHint keys="Shift+R" />}</button>
          <button title="Forward" onClick={() => props.onReply("forward", { ...content.data!, message })}><CornerUpRight size={17} />Forward{message.id === props.selected.id && <KeyHint keys="F" />}</button>
        </div>
      </>}
    </div>
  </section>;
}

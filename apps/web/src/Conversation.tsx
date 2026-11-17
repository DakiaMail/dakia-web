import { messageContent } from "./messageContent";
import { useEffect, useId, useState } from "react";
import { useInfiniteQuery, useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronDown, CornerUpRight, FileText, MoreHorizontal, Reply, ReplyAll } from "lucide-react";
import { Button, DropdownMenu, DropdownMenuItem, FloatingPanel, IconButton } from "@dakia/ui";
import type { Account, Address, Draft, Message, MessageContent } from "@dakia/api-types";
import { api } from "./api";
import { KeyHint } from "./KeyboardHints";
import { ErrorNotice } from "./ui";
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
  const client = useQueryClient();
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
      void client.invalidateQueries({ queryKey: ["thread-counts", selected.account_id] });
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
        initiallyOpen={message.id === selected.id || (!!last?.complete && i === rows.length - 1)} />)}
    </div>
  </article>;
}
function ConversationMessage({ message, initiallyOpen, ...props }: Props & { message: Message; initiallyOpen: boolean }) {
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
  const [headersOpen, setHeadersOpen] = useState(false);
  const [replyError, setReplyError] = useState<unknown>(null);
  async function reply(mode: ReplyMode) {
    setReplyError(null);
    try {
      const metadata = content.data || await client.fetchQuery(messageContent(message.account_id, message.id, client));
      props.onReply(mode, { ...metadata, message });
    } catch (error) { setReplyError(error); }
  }
  const addresses = (values: Address[]) => values.map(a => a.name ? `${a.name} <${a.email}>` : a.email).join(", ");
  return <section className={`conversation-message ${expanded ? "expanded" : "collapsed"}`} aria-label={`Message from ${sender}`}>
    <header className="conversation-summary">
      {expanded ? <>
        <span className="conversation-sender"><b dir="auto">{sender}</b></span>
        <time dateTime={message.received_at}>{new Date(message.received_at).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", year: "numeric" })}</time>
      </> : <button className="conversation-expand-summary" aria-label={`Expand message from ${sender}`} aria-expanded={false} aria-controls={region} onClick={() => setUserExpanded(true)}>
        <b dir="auto">{sender}</b>
        <time dateTime={message.received_at}>{new Date(message.received_at).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", year: "numeric" })}</time>
        <small dir="auto">{message.preview || message.subject}</small>
      </button>}
      <button title={expanded ? "Collapse message" : "Expand message"} aria-label={expanded ? "Collapse message" : "Expand message"} className="conversation-toggle" aria-expanded={expanded} aria-controls={region} onClick={() => { setHeadersOpen(false); setUserExpanded(!expanded); }}>
        <ChevronDown className="conversation-chevron" size={16} aria-hidden="true" />
      </button>
      <div className="conversation-message-actions">
        <IconButton label="Reply" disabled={content.isFetching} onClick={() => void reply("reply")}><Reply size={16} /></IconButton>
        <DropdownMenu label="Message actions" trigger={<IconButton label="More message actions" disabled={content.isFetching}><MoreHorizontal size={16} /></IconButton>}>
          <DropdownMenuItem label="Reply" icon={<Reply />} onClick={() => void reply("reply")} shortcutHint={message.id === props.selected.id ? <KeyHint keys="R" /> : undefined} />
          <DropdownMenuItem label="Reply all" icon={<ReplyAll />} onClick={() => void reply("all")} shortcutHint={message.id === props.selected.id ? <KeyHint keys="Shift+R" /> : undefined} />
          <DropdownMenuItem label="Forward" icon={<CornerUpRight />} onClick={() => void reply("forward")} shortcutHint={message.id === props.selected.id ? <KeyHint keys="F" /> : undefined} />
        </DropdownMenu>
      </div>
      {expanded && <div className="conversation-recipient-line"><FloatingPanel opened={headersOpen} onChange={setHeadersOpen} label="Message details"
        trigger={<Button variant="tertiary" className="conversation-recipients" aria-label="Message details" onClick={() => setHeadersOpen(!headersOpen)}>
          <span dir="auto">to {message.to.map(a => a.email.toLowerCase() === account?.email.toLowerCase() ? "you" : a.name || a.email).join(", ") || "undisclosed recipients"}</span><ChevronDown size={12} aria-hidden="true" />
        </Button>}>
        <dl className="conversation-details">
          <dt>From</dt><dd dir="auto">{addresses([message.sender])}</dd>
          <dt>To</dt><dd dir="auto">{addresses(message.to) || "Undisclosed recipients"}</dd>
          {!!content.data?.cc.length && <><dt>Cc</dt><dd dir="auto">{addresses(content.data.cc)}</dd></>}
          {!!content.data?.reply_to.length && <><dt>Reply to</dt><dd dir="auto">{addresses(content.data.reply_to)}</dd></>}
          <dt>Date</dt><dd className="conversation-date">{new Date(message.received_at).toLocaleString()}</dd>
          <dt>Subject</dt><dd dir="auto">{message.subject || "(No subject)"}</dd>
          {content.data?.message_id && <><dt>Message ID</dt><dd dir="auto">{content.data.message_id}</dd></>}
        </dl>
      </FloatingPanel></div>}
    </header>
    {replyError != null && <div className="conversation-action-error"><ErrorNotice error={replyError} /></div>}
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
      </>}
    </div>
  </section>;
}

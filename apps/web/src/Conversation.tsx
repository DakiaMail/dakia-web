import { useEffect, useId, useRef, useState } from "react";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { ChevronDown, CornerUpRight, FileText, RefreshCw, Reply, ReplyAll } from "lucide-react";
import type { Account, Draft, Message, MessageContent } from "@dakia/api-types";
import { api, ApiFailure } from "./api";
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
  readFailed?: boolean;
  onReply: (mode: ReplyMode, content: MessageContent) => void;
  onDraft: (draft: Draft) => void;
  onCleanup?: (rows: Message[]) => void;
  cleanupRows?: Record<string, Message>;

};
export function Conversation(props: Props) {
  const { selected } = props;
  const cleanupRows = props.cleanupRows || {};
  const force = useRef(false);

  const snapshot = useQuery({
    queryKey: ["conversation-snapshot", selected.account_id, selected.id],
    queryFn: ({ signal }) => api.conversation(selected, null, signal, "snapshot"),
    staleTime: 30000,
    retry: false,
  });
  const thread = useInfiniteQuery({
    queryKey: ["conversation", selected.account_id, selected.id],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam, signal }) => api.conversation(selected, pageParam, signal, force.current ? "refresh" : undefined),
    getNextPageParam: p => p.continuation || undefined,
    staleTime: 30000,
    // SSE refreshes saved conversation rows. Provider discovery runs on open
    // and explicit refresh, never repeatedly while someone reads a message.
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
      if (!thread.hasNextPage) force.current = false;
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
      <button className="conversation-refresh" aria-label="Check for more messages" title="Check for more messages" disabled={thread.isFetching} onClick={() => { force.current = true; void thread.refetch(); }}><RefreshCw size={16} aria-hidden="true" /></button>
    </div>
    {props.readFailed && <p className="reader-status" role="status">Still marked unread. You can mark it as read from the toolbar.</p>}
    {(thread.isError || snapshot.isError) && <div className="reader-status" role="status"><span>Couldn’t check for more messages.</span> <button title="Retry conversation" disabled={thread.isFetching || snapshot.isFetching} onClick={() => {
      if (snapshot.isError) void snapshot.refetch();
      if (thread.isError) {
        // Continuations live in server memory and expire after 15 minutes (or
        // on restart). Replaying one forever cannot recover the conversation.
        const expired = thread.error instanceof ApiFailure && thread.error.code === "conversation_refresh_required";
        void (thread.hasNextPage && !expired ? thread.fetchNextPage() : thread.refetch());
      }
    }}>Retry conversation</button></div>}

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
  const content = useQuery({
    queryKey: ["content", message.account_id, message.id],
    queryFn: async ({ signal }) => {
      const controller = new AbortController();
      const cancel = () => controller.abort();
      signal.addEventListener("abort", cancel, { once: true });
      const timer = setTimeout(cancel, 30000);
      try {
        return await api.contentById(message.account_id, message.id, controller.signal);
      } catch (error) {
        if (controller.signal.aborted && !signal.aborted) throw new Error("This message couldn’t be loaded. Please try again.");
        throw error;
      } finally {
        clearTimeout(timer);
        signal.removeEventListener("abort", cancel);
      }
    },
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
  return <section className={`conversation-message ${expanded ? "expanded" : "collapsed"}`} aria-label={`Message from ${sender}`}>
    <button title={expanded ? "Collapse message" : "Expand message"} className="conversation-summary" aria-expanded={expanded} aria-controls={region} onClick={() => setUserExpanded(!expanded)}>
      <span className={`avatar ${self ? "self-avatar" : ""}`} aria-hidden="true">{sender.slice(0, 2).toUpperCase()}</span>
      <span className="conversation-sender"><b dir="auto">{sender}</b><small dir="auto">{expanded ? message.sender.email : message.preview || message.subject}</small></span>
      <time dateTime={message.received_at}>{new Date(message.received_at).toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit", year: "numeric" })}</time>
      <ChevronDown className="conversation-chevron" size={16} aria-hidden="true" />
    </button>
    <div id={region} hidden={!expanded} className="conversation-body">
      <p className="conversation-recipients" dir="auto">to {message.to.map(a => a.name || a.email).join(", ") || "undisclosed recipients"}{content.data?.cc.length ? ` · cc ${content.data.cc.map(a => a.name || a.email).join(", ")}` : ""}</p>
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

import { snoozeTime } from "./snoozeTime";
import { IconButton, Toast, useDakiaColorScheme } from "@dakia/ui";
import { DateTimeDialog } from "./DateTimeDialog";
import { deviceTranslator } from "./deviceTranslation";
import { useRuleUpdates } from "./mailRules";
import { localMailCache } from "./localMailCache";
import { useMessagePrefetch } from "./useMessagePrefetch";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";

import {
  type InfiniteData,
  useInfiniteQuery,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { measureElement, useVirtualizer } from "@tanstack/react-virtual";
import {
  Archive,
  FolderInput,
  ArrowUp,
  ArrowDown,
  ArrowLeft,
  ChevronDown,
  Clock,
  FileText,
  Inbox,
  LogOut,
  Mail,
  Menu,
  Moon,
  Pencil,
  Plus,
  RefreshCw,

  Search,
  Send,
  ShieldAlert,
  Star,
  Sun,
  Trash2,
  X,
} from "lucide-react";
import type {
  Account,
  Change,
  Draft,
  Message,
  MessageContent,
  SearchPage,
  Submission,
} from "@dakia/api-types";
import { escapeHtml } from "@dakia/mail-renderer";
import { api, ApiFailure, OperationQueue } from "./api";
import { SearchSuggestions } from "./SearchSuggestions";
import { Composer, newDraft } from "./Composer";
import { AccountSetup } from "./AccountSetup";
import { MailboxSettingsDialog } from "./MailboxSettings";
import { AccountRemoval } from "./AccountRemoval";
import { Empty, ErrorNotice, Modal } from "./ui";
import { DraftList, ScheduledList } from "./Records";

import { enableNotifications, extraApi } from "./extra-api";
import { useMailEvents } from "./useMailEvents";
import { usePullToRefresh } from "./usePullToRefresh";
import { Workspace } from "./Workspace";
import { ReleaseNotice } from "./Releases";
import { InstallApp } from "./InstallApp";
import { RefreshButton } from "./RefreshButton";
import { AccountSyncIndicator } from "./AccountSyncIndicator";
import { MessageActions } from "./MessageActions";
import { MoveMessages } from "./MoveMessages";
import { submissionLabel } from "./scheduled";
import { useImagePrivacy } from "./useImagePrivacy";
import { canDismissMail, canUseMailShortcut, plainKey } from "./keyboard";

import { KeyboardHints, KeyHint, ShortcutKeys, shortcutGroups, menuKeyboardHint, useKeyboardPreferences } from "./KeyboardHints";

import { Conversation } from "./Conversation";
import { useCleanupProjection } from "./cleanupJobs";
import { replyDraft } from "./reply";
import { emptySelection, reconcileSelection, selectMail } from "./selection";
import { groups, sectionSize, smartItems, type MailItem, type SectionState } from "./smartView";
import { mailRows } from "./mailRows";
import { useReadSections } from "./useReadSections";
import { useMailInfiniteLoading } from "./useMailInfiniteLoading";
import { notificationLink, rememberNotificationLink } from "./notificationLink";
import { useAppHistory, useBackLayer } from "./appHistory";

type Nav =
  | "INBOX"
  | "Starred"
  | "Snoozed"
  | "Sent"
  | "Drafts"
  | "Trash"
  | "Scheduled";
const navs = [
  { id: "INBOX", label: "Unified inbox", icon: Inbox },
  { id: "Starred", label: "Starred", icon: Star },
  { id: "Snoozed", label: "Snoozed", icon: Clock },
  { id: "Sent", label: "Sent", icon: Send },
  { id: "Drafts", label: "Drafts", icon: FileText },
  { id: "Scheduled", label: "Scheduled", icon: Clock },
  { id: "Trash", label: "Trash", icon: Trash2 },
] as const;
const identity = (m: Message) => `${m.account_id}/${m.id}`;
const failureSummary = (failures: string[]) => new Error(failures.length > 1
  ? `${failures[0]} ${failures.length - 1} other messages also need checking.` : failures[0]);

const time = (t: string) =>
  new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" }).format(
    new Date(t),
  );
const operations = new OperationQueue();
export function App() {
  const client = useQueryClient();
  const initialLink = useRef(notificationLink());
  const [online, setOnline] = useState(navigator.onLine);
  const session = useQuery({
    queryKey: ["session"],
    queryFn: async () => {
      const ticket = localMailCache.ticket();
      const session = await api.session();
      if (!await localMailCache.identify(session.user_id, client, ticket)) throw new ApiFailure(401, "session_changed", "Please sign in again.");
      return session;
    },
    retry: false,
    enabled: online,
  });
  const accounts = useQuery({
    queryKey: ["accounts"],
    queryFn: async () => {
      const ticket = localMailCache.ticket();
      const accounts = await api.accounts();
      await localMailCache.activate(session.data!.user_id, accounts, client, ticket);
      return accounts;
    },
    enabled: !!session.data,
  });
  const { alwaysLoadImages, setAlwaysLoadImages, blockHiddenImages, setBlockHiddenImages } = useImagePrivacy(session.data?.user_id);
  useEffect(() => () => deviceTranslator.reset(), [session.data?.user_id]);
  const keyboardPreferences = useKeyboardPreferences(session.data?.user_id);
  const [shortcutHelp, setShortcutHelp] = useState(false);
  const [account, setAccount] = useState("all");
  const [nav, setNav] = useState<Nav>("INBOX");
  const [view, setView] = useState<"smart" | "list">("smart");
  const [search, setSearch] = useState("");
  const [searchFocused, setSearchFocused] = useState(false);
  const categoryFilter = "";

  const [q, setQ] = useState("");
  const [sections, setSections] = useState<{ scope: string; values: Record<string, SectionState> }>({ scope: "", values: {} });
  const [selected, setSelected] = useState<Message | null>(null);
  const [selection, setSelection] = useState(() => emptySelection("", "smart"));
  const [overrides, setOverrides] = useState<Record<string, Message>>({});
  const [removed, setRemoved] = useState<Record<string, symbol>>({});
  const [restoredRows, setRestoredRows] = useState<Record<string, { message: Message; scope: string; token: symbol }>>({});
  const { colorScheme, setColorScheme } = useDakiaColorScheme();
  const dark = colorScheme === "dark";
  const [sidebar, setSidebar] = useState(false);
  const [workspace, setWorkspace] = useState(false);
  const [updatesFocus, setUpdatesFocus] = useState(0);
  const [setup, setSetup] = useState(false);
  const [reconnect, setReconnect] = useState<Account | null>(null);
  const [settingsAccount, setSettingsAccount] = useState<Account | null>(null);
  const [removingAccount, setRemovingAccount] = useState<Account | null>(null);
  const [compose, setCompose] = useState<{
    draft: Draft;
    quote?: string;
    initialDue?: string;
    forwardAttachments?: import("@dakia/api-types").Attachment[];
  } | null>(null);
  const [error, setError] = useState<unknown>(() => {
    const reason = new URLSearchParams(window.location.search).get("gmail_error");
    if (!reason) return undefined;
    const messages: Record<string, string> = {
      cancelled: "Google sign-in was cancelled. You can try connecting again.",
      expired: "Google sign-in expired. Start connecting again.",
      mismatch: "Choose the Google account that matches the email you entered.",
      permission: "Allow mail access when connecting your Google account.",
      offline: "Google didn’t grant ongoing access. Try connecting again.",
    };
    return new Error(messages[reason] || "Couldn’t finish connecting Gmail. Check your account in Workspace and try again.");
  });
  const [deepLinkError, setDeepLinkError] = useState<unknown>();
  const deepLinkPending = useRef(true);
  const [actionError, setActionError] = useState<unknown>();
  const [readFailures, setReadFailures] = useState<Record<string, boolean>>({});
  const attemptedReads = useRef(new Set<string>());
  const [pendingTargets, setPendingTargets] = useState<Record<string, number>>({});
  const [submission, setSubmission] = useState<Submission | null>(null);
  const [toast, setToast] = useState<{ id: string; message: string } | null>(null);
  useEffect(() => { if (!toast) return; const timer = setTimeout(() => setToast(null), 7000); return () => clearTimeout(timer); }, [toast]);
  const [snooze, setSnooze] = useState<Message[] | null>(null);
  const [move, setMove] = useState<Message[] | null>(null);
  const [notificationStatus, setNotificationStatus] = useState("");
  const [manualRefreshing, setManualRefreshing] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);
  const positions = useRef<Record<string, number>>({});
  const route = { account, nav, view, q, search, categoryFilter, selected, workspace };
  const navigation = useAppHistory(route, next => {
    deepLinkPending.current = false; setDeepLinkError(undefined);
    if (account !== next.account || nav !== next.nav || q !== next.q) clearSelection();
    setAccount(next.account); setNav(next.nav); setView(next.view);
    setQ(next.q); setSearch(next.search);
    setSelected(next.selected); setWorkspace(next.workspace); setSidebar(false);
  }, !!session.data);
  useBackLayer(() => setSidebar(false), sidebar);
  const replyIntent = useRef(0);
  useEffect(() => { replyIntent.current++; }, [account, nav, q, selected, workspace, compose]);
  const mutationTokens = useRef(new Map<string, symbol>());
  const settledTokens = useRef(new Map<string, symbol>());
  const ready = (accounts.data || []).filter(
    (a) => account === "all" || a.id === account,
  );
  const ids = ready.map((a) => a.id);
  const restoredUser = useRef<string | null>(null);
  useEffect(() => {
    const user = session.data?.user_id;
    if (!user || !accounts.data || restoredUser.current === user) return;
    restoredUser.current = user;
    const pending = operations.restore(user).filter(op => accounts.data!.some(account => account.id === op.account_id));
    if (!pending.length) return;
    const token = Symbol();
    const failures: string[] = [];
    setRemoved(old => ({ ...old, ...Object.fromEntries(pending.filter(op => ["archive", "trash", "spam", "move", "snooze"].includes(op.change.kind)).map(op => [`${op.account_id}/${op.target_id}`, token])) }));
    setPendingTargets(old => {
      const next = { ...old };
      for (const op of pending) {
        const id = `${op.account_id}/${op.target_id}`;
        next[id] = (next[id] || 0) + 1;
      }
      return next;
    });
    void Promise.all(pending.map(op => operations.resume(op).catch(error => {
      const account = accounts.data!.find(account => account.id === op.account_id)!;
      failures.push(`${account.display_name}: ${error instanceof Error ? error.message : "Pending action could not be verified."}`);
      setActionError(failureSummary(failures));
      setRemoved(old => {
        const next = { ...old };
        const id = `${op.account_id}/${op.target_id}`;
        if (next[id] === token) delete next[id];
        return next;
      });
    }).finally(() => releasePending(`${op.account_id}/${op.target_id}`))))
      .finally(async () => {
        await client.invalidateQueries({ queryKey: ["mail"] });
        setRemoved(old => Object.fromEntries(Object.entries(old).filter(([, value]) => value !== token)));
      });
  }, [session.data?.user_id, accounts.data, client]);
  useRuleUpdates();
  useMailEvents(accounts.data || []);
  const cleanupRows = useCleanupProjection(accounts.data || []);
  const deepLinkOpened = useRef(false);
  useEffect(() => {
    if (!accounts.data || deepLinkOpened.current || !deepLinkPending.current) return;
    const link = initialLink.current;
    const a = link?.account;
    const id = link?.message;
    if (!a || !id || !accounts.data.some((account) => account.id === a)) return;
    deepLinkOpened.current = true;
    void api
      .contentById(a, id)
      .then((result) => {
        if (!deepLinkPending.current) return;
        client.setQueryData(["content", a, id], result);
        void open(result.message, a);
      })
      .catch(error => { if (deepLinkPending.current) setDeepLinkError(error); });
  }, [accounts.data, client]);
  const snoozeState = useQuery({
    queryKey: ["snoozes", ids], enabled: !!session.data && ids.length > 0,
    queryFn: async ({ signal }) => (await Promise.all(ids.map(async account_id =>
      (await api.snoozes(account_id, signal)).map(row => ({ ...row, account_id }))))).flat(),
    refetchInterval: 30000, retry: false,
  });
  const [snoozeClock, setSnoozeClock] = useState(Date.now);
  useEffect(() => {
    if (!snoozeState.data?.length) return;
    const timer = setInterval(() => setSnoozeClock(Date.now()), 30000);
    return () => clearInterval(timer);
  }, [snoozeState.data?.length]);
  const snoozedThreads = useMemo(() => Object.fromEntries((snoozeState.data || [])
    .filter(row => Date.parse(row.until) > Date.now()).map(row => [`${row.account_id}/${row.thread_id}`, row])), [snoozeState.data, snoozeClock]);
  const snoozedCount = Object.values(snoozedThreads).reduce((sum, row) => sum + row.message_count, 0);
  const listKey = ["mail", account, nav, q, categoryFilter, ids];
  const explicitRefresh = useRef(false);
  const mail = useInfiniteQuery<SearchPage, Error, InfiniteData<SearchPage>, typeof listKey, string | null>({
    queryKey: listKey,
    initialPageParam: null as string | null,
    enabled:
      !!accounts.data &&
      !workspace &&
      ids.length > 0 &&
      !["Drafts", "Scheduled"].includes(nav),
    staleTime: 30000,
    refetchInterval: query => query.state.data?.pages.some(p => p.cache_preview) ? false : query.state.data?.pages.some(p => p.pending) ? 3000 : 60000,
    queryFn: async ({ pageParam, signal }): Promise<SearchPage> => {
      if (q) return client.fetchQuery({
        queryKey: ["search-page", q, ids, categoryFilter, pageParam],
        queryFn: ({ signal }) => api.search(q, ids, pageParam, categoryFilter, signal, false, false, true),
        staleTime: pageParam ? 30000 : 0,
        retry: false,
      });
      // The provider search endpoint handles federated coverage and pagination.
      if (ids.length > 1 || nav === "INBOX" || nav === "Starred" || nav === "Snoozed")
        return api.search(
          nav === "Starred"
            ? "is:starred"
            : nav === "Snoozed"
              ? "is:snoozed"
              : `folder:${nav}`,
          ids,
          pageParam,
          undefined,
          signal,
          false,
          explicitRefresh.current,
          true,
        );
      return api.messages(ids[0], nav, pageParam, signal);
    },
    // Repeated opaque cursors are not progress, including multi-page cycles.
    getNextPageParam: (page, _pages, _param, params) =>
      page.continuation && !params.includes(page.continuation) ? page.continuation : undefined,
    retry: false,
  });
  const rows = useMemo(() => {
    // A pending server projection may already have removed the cached row.
    // Keep failed originals in their source query until a fresh read reconciles.
    const restored = Object.values(restoredRows).filter(row => row.scope === JSON.stringify(listKey)).map(row => row.message);
    let all = mailRows([...(mail.data?.pages || []), { rows: restored }], overrides).filter(m => !removed[identity(m)]);
    // Frozen cleanup targets use the same account/message identity and Trash
    // projection as ordinary mail actions, independent of stale cached pages.
    if (nav === "Trash" && !q) all = mailRows([{ rows: all }, { rows: Object.values(cleanupRows).filter(m => ids.includes(m.account_id)) }], cleanupRows);
    else all = all.filter(m => !cleanupRows[identity(m)]);
    if (!q && nav === "INBOX")
      all = all.filter((m) =>
        m.folder_ids.some((f) => f.toUpperCase() === "INBOX") && !snoozedThreads[`${m.account_id}/${m.thread_id}`],
      );
    if (!q && nav === "Snoozed" && snoozeState.isSuccess) all = all.filter(m => !!snoozedThreads[`${m.account_id}/${m.thread_id}`]);
    if (nav === "Starred") all = all.filter((m) => m.starred);
    return all;
  }, [mail.data, overrides, removed, restoredRows, nav, q, account, categoryFilter, accounts.data, cleanupRows, snoozedThreads, snoozeState.isSuccess]);
  const lastContinuation = mail.data?.pages.at(-1)?.continuation;
  // Start provider reconciliation after a preview without waiting for scroll.
  useEffect(() => {
    if (mail.data?.pages.at(-1)?.cache_preview && !mail.data?.pages.at(-1)?.index_page && mail.hasNextPage && !mail.isFetching && !mail.isError)
      void mail.fetchNextPage({ cancelRefetch: false });
  }, [mail.data, mail.hasNextPage, mail.isFetching, mail.isError, mail.fetchNextPage]);
  // Warm exactly one following page without appending it or changing scroll.
  // Query keys bind both account scope and query. Foreground load reuses the
  // same promise, so prefetched provider continuations are never fetched twice.
  useEffect(() => {
    if (q && lastContinuation && !mail.isFetching && !mail.isError)
      void client.prefetchQuery({
        queryKey: ["search-page", q, ids, categoryFilter, lastContinuation],
        queryFn: ({ signal }) => api.search(q, ids, lastContinuation, categoryFilter, signal, false, false, true),
        staleTime: 30000,
        retry: false,
      });
  }, [q, ids.join(","), categoryFilter, lastContinuation, mail.isFetching, mail.isError, client]);
  const paginationStalled = !!lastContinuation && !!mail.data?.pageParams.includes(lastContinuation);
  const refreshMail = useCallback(async () => {
    setManualRefreshing(true);
    explicitRefresh.current = true;
    try {
      await reconcileMail();
    } finally {
      setManualRefreshing(false);
      explicitRefresh.current = false;
    }
  }, [client]);
  const pull = usePullToRefresh(listRef, !workspace && !mail.isPending && !["Drafts", "Scheduled"].includes(nav), refreshMail);
  const sectionScope = JSON.stringify([account, nav, q, categoryFilter, ids]);
  const heldReadSections = useReadSections(rows, selected, sectionScope, view === "smart" && !q && nav === "INBOX" && !workspace);
  function updateSection(label: string, change: (old: SectionState) => SectionState) {
    setSections(old => {
      const values = old.scope === sectionScope ? old.values : {};
      return { scope: sectionScope, values: { ...values, [label]: change(values[label] || {}) } };
    });
  }
  const groupedItems = useMemo<MailItem[]>(() => {
    if (view === "list" || q || nav !== "INBOX")
      return rows.map((message) => ({ kind: "mail", message }));
    return smartItems(rows, sections.scope === sectionScope ? sections.values : {}, heldReadSections);
  }, [rows, view, q, nav, sections, sectionScope, heldReadSections]);
  // Fold the final section's reveal action into the provider footer, so the
  // bottom never offers two adjacent Load more buttons. Reveal cached mail
  // before requesting another page; collapsed sections stay collapsed.
  const lastItem = groupedItems.at(-1);
  const bottomSection = lastItem?.kind === "more" ? lastItem.label : null;
  const items = useMemo(() => bottomSection === null ? groupedItems : groupedItems.slice(0, -1), [groupedItems, bottomSection]);
  function loadMore(section: string) {
    if (section) {
      updateSection(section, old => old.collapsed ? old : ({ ...old, limit: (old.limit || sectionSize) + sectionSize }));
    } else if (mail.hasNextPage && !mail.isFetching) {
      void mail.fetchNextPage({ cancelRefetch: false });
    }
  }
  const infiniteScroll = useMailInfiniteLoading(listRef, `${sectionScope}/${view}`, !workspace && !mail.isFetching && !mail.isError && !["Drafts", "Scheduled"].includes(nav), loadMore);
  const selectionOrder = useMemo(() => items.flatMap(item =>
    item.kind === "mail" ? [identity(item.message)] : []), [items]);
  const selectionScope = JSON.stringify([account, nav, q, categoryFilter, ids, workspace]);
  const currentSelection = useMemo(() => reconcileSelection(
    selection, [...selectionOrder, ...Object.keys(pendingTargets)], selectionScope, view,
  ), [selection, selectionOrder, selectionScope, view, pendingTargets]);
  const checked = currentSelection.checked;
  useEffect(() => {
    if (selection !== currentSelection) setSelection(currentSelection);
  }, [selection, currentSelection]);
  function clearSelection() {
    setSelection(emptySelection(selectionScope, view));
  }
  function selectMessage(message: Message, range: boolean, value: boolean) {
    setSelection(old => selectMail(
      reconcileSelection(old, selectionOrder, selectionScope, view),
      selectionOrder, identity(message), range, value,
    ));
  }
  const virtual = useVirtualizer({
    count: items.length,
    getScrollElement: () => listRef.current,
    estimateSize: (i) => (items[i].kind === "heading" ? 38 : items[i].kind === "more" ? 44 : 64),
    // display:none on mobile must not collapse every measured row to zero.
    measureElement: (element, entry, instance) => measureElement(element, entry, instance)
      || instance.measurementsCache[instance.indexFromElement(element)]?.size || 64,
    overscan: 8,
    getItemKey: (i) =>
      items[i].kind !== "mail"
        ? `${items[i].kind}:${items[i].label}`
        : identity((items[i] as { kind: "mail"; message: Message }).message),
  });
  const active = selected
    ? overrides[identity(selected)] ||
      rows.find((m) => identity(m) === identity(selected)) ||
      selected
    : null;

  const nextMessage = useMemo(() => {
    if (!active) return;
    let found = false;
    for (const item of items) {
      if (item.kind !== "mail") continue;
      if (found) return item.message;
      found = item.message.id === active.id && item.message.account_id === active.account_id;
    }
  }, [items, active?.id, active?.account_id]);
  const prefetch = useMessagePrefetch(JSON.stringify([session.data?.user_id, account, nav, q, categoryFilter, ids, workspace]), nextMessage, listRef, rows);
  const submitState = useQuery({
    queryKey: ["submission", submission?.account_id, submission?.id],
    queryFn: () => api.submission(submission!),
    enabled: !!submission,
    refetchInterval: (query) =>
      ["scheduled", "claimed", "submitting"].includes(
        query.state.data?.state || submission?.state || "",
      )
        ? 3000
        : false,
  });
  const latestSubmission = submitState.data || submission;

  useEffect(() => {
    const update = () => setOnline(navigator.onLine);
    window.addEventListener("online", update);
    window.addEventListener("offline", update);
    return () => {
      window.removeEventListener("online", update);
      window.removeEventListener("offline", update);
    };
  }, []);
  const listHeight = virtual.getTotalSize();
  useLayoutEffect(() => {
    // The virtualizer temporarily measures zero while the mobile list is
    // hidden. Restore again when its rows regain height, before paint.
    if (listRef.current && listHeight)
      listRef.current.scrollTop =
        positions.current[`${account}/${nav}/${q}/${view}`] || 0;
  }, [account, nav, q, view, mail.isPending, selected, listHeight]);
  useEffect(() => {
    const t = setTimeout(() => applySearch(search.trim()), 350);
    return () => clearTimeout(t);
  }, [search, q]);
  useEffect(() => {
    const restore = () => void reconcileMail();
    window.addEventListener("online", restore);
    window.addEventListener("focus", restore);
    return () => {
      window.removeEventListener("online", restore);
      window.removeEventListener("focus", restore);
    };
  }, [client]);

  useEffect(() => {
    const keyboard = (e: KeyboardEvent) => {
      if (compose || setup || workspace || !session.data) return;
      if (canDismissMail(e)) {
        if (selected) closeReader();
        setSidebar(false);
        clearSelection();
        return;
      }
      if (!keyboardPreferences.shortcuts || !canUseMailShortcut(e)) return;
      const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
      const focusedId = (e.target as Element).closest<HTMLElement>("[data-message-id]")?.dataset.messageId;
      const focused = rows.find(message => identity(message) === focusedId);
      const target = focused || active;
      const targets = checked.size ? rows.filter(message => checked.has(identity(message))) : target ? [target] : [];
      const pending = targets.some(message => pendingTargets[identity(message)] > 0 || !!cleanupRows[identity(message)]);
      const consume = (action: () => void) => { e.preventDefault(); action(); };
      if (key === "?") return consume(() => setShortcutHelp(true));
      if (key === "c" && !e.shiftKey) return consume(startCompose);
      if (key === "/" && !e.shiftKey) return consume(() => {
        setSidebar(true);
        setSearchFocused(true);
        requestAnimationFrame(() => document.querySelector<HTMLInputElement>('.sidebar .search input')?.focus());
      });
      if ((key === "j" || key === "k") && !e.shiftKey) return consume(() => navigateMessage(key === "j" ? 1 : -1, target));
      if (!targets.length || pending) return;
      if (key === "x" && !e.shiftKey && target) return consume(() => selectMessage(target, false, !checked.has(identity(target))));
      if ((key === "r" || (key === "f" && !e.shiftKey)) && !checked.size && target) {
        // Use the focused message's own loaded content, never another email's body.
        consume(() => { void shortcutReply(target, key === "f" ? "forward" : e.shiftKey ? "all" : "reply"); });
        return;
      }
      const change: Change | undefined = key === "e" && !e.shiftKey ? { kind: "archive" }
        : key === "Delete" && !e.shiftKey ? { kind: "trash" }
        : key === "u" && !e.shiftKey ? { kind: "read", value: false }
        : key === "u" && e.shiftKey ? { kind: "read", value: true }
        : key === "s" && !e.shiftKey ? { kind: "star", value: targets.some(message => !message.starred) }
        : key === "!" ? { kind: "spam" } : undefined;
      if (change) return consume(() => {
        void act(targets, change);
        if (["archive", "trash", "spam"].includes(change.kind) && active && targets.some(message => identity(message) === identity(active))) closeReader();
      });
      if (key === "m" && !e.shiftKey) return consume(() => setMove([...targets]));
      if (key === "b" && !e.shiftKey) return consume(() => setSnooze([...targets]));
    };
    window.addEventListener("keydown", keyboard);
    return () => window.removeEventListener("keydown", keyboard);
  });
  function releasePending(id: string) {
    setPendingTargets(old => {
      const next = { ...old };
      if (next[id] > 1) next[id]--;
      else delete next[id];
      return next;
    });
  }
  async function act(input: Message[], change: Change, destinations?: Record<string, string>, background = false) {
    const messages = [...new Map(input.map(message => [identity(message), message])).values()];
    if (!messages.length) return { applied: 0, failed: 0 };
    if (!background && change.kind === "read") {
      for (const message of messages) attemptedReads.current.delete(identity(message));
      setReadFailures(old => {
        const next = { ...old };
        for (const message of messages) delete next[identity(message)];
        return next;
      });
    }
    const cacheAccounts = ["archive", "trash", "spam", "move", "snooze"].includes(change.kind) ? [...new Set(messages.map(m => m.account_id))] : [];
    const cacheStarted = Date.now();
    const cacheInvalidation = cacheAccounts.length ? localMailCache.invalidateFolders(cacheAccounts) : Promise.resolve();
    const failures: string[] = [];
    setError(undefined);
    const removing = ["archive", "trash", "spam", "move", "snooze"].includes(change.kind);
    if (removing && document.activeElement instanceof Element) {
      const focused = document.activeElement.closest<HTMLElement>("[data-message-id]")?.dataset.messageId;
      const removedIds = new Set(messages.map(identity));
      if (focused && removedIds.has(focused)) {
        const index = selectionOrder.indexOf(focused);
        const next = selectionOrder.slice(index + 1).find(id => !removedIds.has(id))
          || selectionOrder.slice(0, index).reverse().find(id => !removedIds.has(id));
        requestAnimationFrame(() => {
          if (document.activeElement !== document.body) return;
          const row = Array.from(document.querySelectorAll<HTMLElement>("[data-message-id]")).find(el => el.dataset.messageId === next);
          (row?.querySelector<HTMLButtonElement>(".message-open") || listRef.current)?.focus();
        });
      }
    }
    const selectedTargets = new Set(messages.filter(m => checked.has(identity(m))).map(identity));
    if (removing) setSelection(old => ({ ...old, checked: new Set([...old.checked].filter(id => !selectedTargets.has(id))) }));
    setPendingTargets(old => {
      const next = { ...old };
      if (!background) for (const message of messages) next[identity(message)] = (next[identity(message)] || 0) + 1;
      return next;
    });
    await Promise.all(
      messages.map((m) => {
        const token = Symbol();
        mutationTokens.current.set(identity(m), token);
        if (removing) {
          setRemoved(old => ({ ...old, [identity(m)]: token }));
        }
        return operations
          .enqueue(
            m,
            destinations ? { kind: "move", folder_id: destinations[m.account_id] } : change,
            (_id, value) =>
              setOverrides((old) => ({ ...old, [identity(value)]: value })),
            () => {
              settledTokens.current.set(identity(m), token);
            },
          )
          .catch((e) => {
            failures.push(e instanceof Error ? e.message : "Mail action could not be verified.");
            if (background) setReadFailures(old => ({ ...old, [identity(m)]: true }));
            else setActionError(failureSummary(failures));
            // Restore this item immediately, without overwriting newer intent.
            if (mutationTokens.current.get(identity(m)) === token) {
              setOverrides(old => ({ ...old, [identity(m)]: m }));
              setRestoredRows(old => ({ ...old, [identity(m)]: { message: m, scope: JSON.stringify(listKey), token } }));
              if (selectedTargets.has(identity(m))) setSelection(old => old.scope === selectionScope
                ? { ...old, checked: new Set([...old.checked, identity(m)]) } : old);
              setRemoved(old => {
                const next = { ...old };
                if (next[identity(m)] === token) delete next[identity(m)];
                return next;
              });
            }
          }).finally(() => { if (!background) releasePending(identity(m)); });
      }),
    );
    await reconcileMail(background);
    await cacheInvalidation;
    localMailCache.resumeFolders(cacheAccounts, listKey, cacheStarted);
    return { applied: messages.length - failures.length, failed: failures.length };
  }
  async function reconcileMail(quiet = false) {
    try {
      await client.cancelQueries({ queryKey: ["mail"] });
      const completed = new Map(settledTokens.current);
      await Promise.all([client.invalidateQueries({ queryKey: ["mail"] }, { throwOnError: true }), client.invalidateQueries({ queryKey: ["snoozes"] })]);
      setOverrides(old => {
        const next = { ...old };
        for (const [id, done] of completed) {
          if (mutationTokens.current.get(id) === done) delete next[id];
          if (settledTokens.current.get(id) === done) settledTokens.current.delete(id);
        }
        return next;
      });
      setRemoved(old => {
        const next = { ...old };
        for (const [id, done] of completed) if (next[id] === done) delete next[id];
        return next;
      });
      setRestoredRows(old => {
        const next = { ...old };
        for (const [id, done] of completed) if (next[id]?.token === done) delete next[id];
        return next;
      });
    } catch (error) { if (!quiet) setError(error); }
  }
  async function snoozeMessages(messages: Message[], until: string) {
    const result = await act(messages, { kind: "snooze", until: new Date(until).toISOString() });
    if (!result.applied) return;
    const count = result.applied;
    setToast({ id: crypto.randomUUID(), message: `${count === 1 ? "Email" : `${count} emails`} snoozed until ${snoozeTime(until)}.` });
    if (active && messages.some(m => identity(m) === identity(active))) closeReader();
  }
  async function restoreInbox(messages: Message[]) {
    const result = await act(messages, { kind: "move", folder_id: "INBOX" });
    if (!result.applied) return;
    setToast({ id: crypto.randomUUID(), message: `${result.applied === 1 ? "Email" : `${result.applied} emails`} moved back to Inbox.` });
    if (active && messages.some(m => identity(m) === identity(active))) closeReader();
  }
  function navigateMessage(direction: number, from: Message | null = active) {
    const visible = items.flatMap(item => item.kind === "mail" ? [item.message] : []);
    const index = from ? visible.findIndex(message => identity(message) === identity(from)) : -1;
    const next = index < 0 ? (direction > 0 ? visible[0] : visible.at(-1)) : visible[index + direction];
    if (!next) return;
    virtual.scrollToIndex(items.findIndex(item => item.kind === "mail" && identity(item.message) === identity(next)), { align: "auto" });
    void open(next);
    requestAnimationFrame(() => document.querySelector<HTMLElement>(".reader")?.focus());
  }
  function closeReader() {
    navigation.backTo(previous => !previous.selected && !previous.workspace);
    requestAnimationFrame(() => listRef.current?.focus());
  }
  function removeFromReader(kind: "archive" | "trash" | "spam") {
    // Snapshot the existing toolbar targets before closing. Checkbox/bulk
    // actions keep their existing behavior, including a different open preview.
    void act(bulk, { kind });
    if (active && !checked.size) closeReader();
  }
  function actionFailure() {
    return actionError ? <div>
      <ErrorNotice error={actionError} />
      <button title="Dismiss mail action error" onClick={() => setActionError(undefined)}>Dismiss mail action error</button>
    </div> : null;
  }

  async function open(m: Message, linkedAccount?: string) {
    prefetch.foreground(m);
    if (!selected && listRef.current)
      positions.current[`${account}/${nav}/${q}/${view}`] = listRef.current.scrollTop;
    const url = new URL(location.href);
    url.searchParams.set("account", m.account_id); url.searchParams.set("message", m.id);
    if (!selected || identity(selected) !== identity(m)) navigation.push({ ...route, account: linkedAccount || account, selected: m, workspace: false }, url.href);
    setSelection(old => ({
      ...reconcileSelection(old, selectionOrder, selectionScope, view),
      anchor: identity(m),
    }));
    setSidebar(false);
    if (m.unread && !pendingTargets[identity(m)] && !attemptedReads.current.has(identity(m))) {
      attemptedReads.current.add(identity(m));
      void act([m], { kind: "read", value: true }, undefined, true);
    }
  }
  function startCompose() {
    const a = (accounts.data || []).find(
      (a) => a.state === "ready" && (account === "all" || a.id === account),
    );
    if (a) setCompose({ draft: newDraft(a.id) });
    else setSetup(true);
  }
  async function shortcutReply(target: Message, mode: "reply" | "all" | "forward") {
    const intent = ++replyIntent.current;
    try {
      const key = ["content", target.account_id, target.id];
      const metadata = client.getQueryData<MessageContent>(key) || await client.fetchQuery<MessageContent>({ queryKey: key, queryFn: () => api.content(target) });
      if (intent === replyIntent.current) reply(mode, metadata);
    } catch (error) { if (intent === replyIntent.current) setError(error); }
  }
  function reply(mode: "reply" | "all" | "forward", metadata: MessageContent) {
    const active = metadata.message;
    const d = replyDraft(mode, metadata, accounts.data || []);
    const quote = `<p>On ${escapeHtml(new Date(active.received_at).toLocaleString())}, ${escapeHtml(active.sender.email)} wrote:</p><blockquote>${metadata.html || escapeHtml(metadata.text).replace(/\n/g, "<br>")}</blockquote>`;
    setCompose({
      draft: d,
      quote,
      forwardAttachments:
        mode === "forward" ? metadata.attachments : undefined,
    });
  }
  function navigate(next: Nav) {
    positions.current[`${account}/${nav}/${q}/${view}`] =
      listRef.current?.scrollTop || 0;
    if (nav !== next || selected || workspace || q) navigation.push({ ...route, nav: next, selected: null, workspace: false, q: "", search: "" });
    clearSelection();
    setSidebar(false);
    setSearch("");
    setQ("");
  }
  function changeAccount(next: string) {
    if (account !== next || selected || workspace) navigation.push({ ...route, account: next, selected: null, workspace: false }, undefined, undefined, sidebar);
    setSidebar(sidebar);
  }
  function applySearch(next: string) {
    if (next === q) return;
    // A mobile search is submitted from a navigation history layer. Update its
    // underlying screen before closing it, or Back restores the previous query.
    if (!!next !== !!q || sidebar) {
      navigation.push({ ...route, q: next, search: next, selected: null, workspace: false }, undefined, { ...route, search: q }, sidebar);
      setSidebar(sidebar);
    }
    else setQ(next);
  }
  const searchForm = <form
    className={`search ${q ? "results-search" : ""}`}
    role="search"
    onFocus={() => setSearchFocused(true)}
    onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget)) setSearchFocused(false); }}
    onKeyDown={event => {
      if (event.key !== "Escape" || !plainKey(event.nativeEvent) || event.repeat) return;
      event.preventDefault(); event.stopPropagation();
      setSearchFocused(false); setSidebar(false);
      requestAnimationFrame(() => (listRef.current?.matches("[tabindex]") ? listRef.current : document.querySelector<HTMLElement>(".mailbox"))?.focus());
    }}
    onSubmit={event => {
      event.preventDefault();
      applySearch(search.trim());
      setSearchFocused(false);
      setSidebar(false);
    }}
  >
    {!q && <Search size={16} />}
    <input
      placeholder="Search all mail"
      aria-label="Search all mail"
      value={search}
      autoFocus={searchFocused}
      onChange={event => { setSearch(event.target.value); setSearchFocused(true); }}
    />
    <KeyHint keys="/" />
    {q && <button title="Search" type="submit" aria-label="Search"><Search size={18} /></button>}
    {searchFocused && <SearchSuggestions value={search} ids={ids} onOpen={message => { setSearchFocused(false); setSidebar(false); void open(message, message.account_id); }} />}
  </form>;
  const bulk = checked.size
    ? rows.filter((m) => checked.has(identity(m)))
    : active
      ? [active]
      : [];
  const allStarred = bulk.every(message => message.starred);
  const messageActions = bulk.length > 0 ? <MessageActions
    sender={checked.size === 0 ? active || undefined : undefined}
    accounts={accounts.data || []}
    onCleanup={targets => { clearSelection(); if (active && targets.some(m => identity(m) === identity(active))) closeReader(); }}
    pending={bulk.some(message => pendingTargets[identity(message)] > 0 || !!cleanupRows[identity(message)])}
    archive={() => removeFromReader("archive")}
    unread={() => void act(bulk, { kind: "read", value: false })}
    read={() => void act(bulk, { kind: "read", value: true })}
    starred={allStarred}
    toggleStar={() => void act(bulk, { kind: "star", value: !allStarred })}
    trash={() => removeFromReader("trash")}
    spam={() => removeFromReader("spam")}
    restore={bulk.every(m => !!snoozedThreads[`${m.account_id}/${m.thread_id}`]) ? () => void restoreInbox([...bulk]) : undefined}
    snooze={() => setSnooze([...bulk])} move={() => setMove([...bulk])}
    category={bulk.every(message => message.category === bulk[0].category) ? bulk[0].category || "" : ""}
    categories={groups.filter(g => g !== "pending" && g !== "other")}
    setCategory={value => void act(bulk, { kind: "category", value })}
  /> : null;
  const coverage = mail.data?.pages.at(-1);
  const inboxCoverage = !q && nav === "INBOX" ? coverage : undefined;
  const refreshFailed = !!mail.error || !!inboxCoverage?.coverage.some(c => c.state === "error");
  const updating = manualRefreshing;
  const refreshStatus = refreshFailed
    ? "Couldn’t update mail. Try refreshing again."
    : updating ? (!q && nav === "INBOX" ? "Updating inbox…" : "Updating mail…")
    : inboxCoverage?.coverage.some(c => c.state === "partial") ? "Inbox refreshed. Provider coverage is limited; some mail may not be exposed."
    : inboxCoverage?.coverage.some(c => c.state !== "complete") ? "Inbox refresh is pending." : "Refresh mail";
  const signedInUser = useRef<string | undefined>(undefined);
  if (!signedInUser.current && session.data?.user_id) signedInUser.current = session.data.user_id;
  const identityChanged = !!signedInUser.current && !!session.data?.user_id && signedInUser.current !== session.data.user_id;
  useEffect(() => { if (identityChanged) window.location.replace("/"); }, [identityChanged]);
  if (identityChanged) return <Empty title="Opening Dakia…" />;
  const sessionExpired = session.error instanceof ApiFailure && session.error.status === 401;
  const connectionProblem = !online || !!session.error;
  const connectionNotice = connectionProblem && !sessionExpired && (
    <div role="status" className="connection-state session-connection-state">
      <span>{online
        ? "Can’t reach Dakia. Check your internet connection or VPN."
        : "You’re offline. Connect to the internet to load or change mail."}</span>
      {online && <button title="Retry connection" disabled={session.isFetching} onClick={() => void session.refetch()}>
        {session.isFetching ? "Retrying…" : "Retry connection"}
      </button>}
    </div>
  );
  // A failed session read is not evidence of logout. Retain the mounted
  // workspace (including reader/composer state) unless authentication is denied.
  if (!session.data && !sessionExpired && connectionProblem)
    return (
      <div className="login connection-recovery">
        <img src="/dakia-icon-512.png" alt="Dakia" width="72" height="72" />
        <h1>{online ? "Can’t reach Dakia" : "You’re offline"}</h1>
        <p role="status">Check your internet connection or VPN, then try again.</p>
        <button title="Retry connection" className="primary" disabled={session.isFetching || !online} onClick={() => void session.refetch()}>
          {session.isFetching ? "Retrying…" : "Retry connection"}
        </button>
      </div>
    );
  if (session.isPending) return <Empty title="Opening Dakia…" />;
  if (sessionExpired)
    return (
      <div className="login">
        <img src="/dakia-icon-512.png" alt="Dakia" width="72" height="72" />
        <h1>Your mail, in one place.</h1>
        <a title="Sign in" className="primary" href="/auth/login" onClick={rememberNotificationLink}>
          Sign in
        </a>
        <InstallApp />
      </div>
    );
  return (
    <KeyboardHints.Provider value={keyboardPreferences.hints && keyboardPreferences.shortcuts}>
    <div
      className={`app ${selected ? "reading" : ""} ${sidebar ? "nav-open" : ""} ${workspace ? "workspace-view" : ""}`}

    >
      <aside className="sidebar">
        <div className="brand">
          <img src="/dakia-icon-192.png" alt="" width="32" height="32" />
          Dakia
          <button title="Close navigation"
            aria-label="Close navigation"
            className="mobile"
            onClick={() => setSidebar(false)}
          >
            <X size={18} />
          </button>
        </div>
        {(!q || sidebar) && searchForm}

        <nav aria-label="Mailboxes">
          {navs.map(({ id, label, icon: Icon }) => (
            <button title={label}
              aria-label={id === "Snoozed" ? "Snoozed" : undefined}
              aria-describedby={id === "Snoozed" ? "snoozed-count" : undefined}
              key={id}
              className={!workspace && nav === id && !q ? "selected" : ""}
              aria-current={!workspace && nav === id && !q ? "page" : undefined}
              onClick={() => navigate(id)}
            >
              <Icon size={17} />
              {label}
              {id === "Snoozed" && <><span className="count" aria-hidden="true">{snoozeState.isSuccess ? snoozedCount : "…"}</span><span className="sr-only" id="snoozed-count">{snoozeState.isSuccess ? `${snoozedCount} snoozed emails` : "Snoozed count unavailable"}</span></>}
              {id === "INBOX" && rows.some((m) => m.unread) && (
                <span className="count">
                  {rows.filter((m) => m.unread).length}
                </span>
              )}
            </button>
          ))}
        </nav>
        <div className="section-label">
          Accounts
          <button title="Connect account" aria-label="Connect account" onClick={() => setSetup(true)}>
            <Plus size={15} />
          </button>
        </div>
        <nav aria-label="Accounts">
          <button title="All accounts"
            className={account === "all" ? "selected" : ""}
            onClick={() => {
              changeAccount("all");
            }}
          >
            All accounts
          </button>
          {(accounts.data || []).map((a) => (
            <button title={a.display_name}
              key={a.id}
              className={account === a.id ? "selected" : ""}
              onClick={() => {
                changeAccount(a.id);
              }}
            >
              <Mail size={16} />
              <span className="account-name">
                {a.display_name}
                <small>{a.email}</small>
              </span>
              <AccountSyncIndicator account={a} online={online} />
              {a.state !== "ready" && (
                <span className="account-state">{a.state}</span>
              )}
            </button>
          ))}
        </nav>
        <button title="Refresh mail" className="mobile sidebar-refresh" onClick={() => { void refreshMail(); setSidebar(false); }}><RefreshCw size={16} />Refresh mail</button>
        <div className="profile">
          <button title="Manage workspace" className="workspace-link" aria-label="Manage workspace" aria-current={workspace ? "page" : undefined} onClick={() => { if (!workspace) navigation.push({ ...route, workspace: true }); setSidebar(false); }}><span className="avatar">D</span><span>Private workspace</span></button>
          <button title={dark ? "Light theme" : "Dark theme"}
            aria-label={dark ? "Light theme" : "Dark theme"}
            onClick={() => setColorScheme(dark ? "light" : "dark")}
          >
            {dark ? <Sun size={17} /> : <Moon size={17} />}
          </button>
          <button title="Sign out"
            aria-label="Sign out"
            onClick={async () => {
              try {
                await localMailCache.clear();
                await api.logout();
                client.clear();
                window.location.assign("/auth/login");
              } catch (e) {
                setError(e);
              }
            }}
          >
            <LogOut size={16} />
          </button>
        </div>
        <ReleaseNotice userId={session.data?.user_id} onDetails={() => {
          if (!workspace) navigation.push({ ...route, workspace: true });
          setUpdatesFocus(value => value + 1); setSidebar(false);
        }} />
      </aside>
      {sidebar && (
        <button title="Close navigation"
          className="nav-scrim"
          aria-label="Close navigation"
          onClick={() => setSidebar(false)}
        />
      )}
      {workspace && <Workspace updatesFocus={updatesFocus} userId={session.data?.user_id} accounts={accounts.data || []} blockHiddenImages={blockHiddenImages} onHiddenImagesChange={setBlockHiddenImages} keyboardHints={keyboardPreferences.hints} onKeyboardHintsChange={keyboardPreferences.setHints} keyboardShortcuts={keyboardPreferences.shortcuts} onKeyboardShortcutsChange={keyboardPreferences.setShortcuts} onShortcutHelp={() => setShortcutHelp(true)} alwaysLoadImages={alwaysLoadImages} onImagePrivacyChange={setAlwaysLoadImages} onBack={() => { navigation.backTo(previous => !previous.workspace); requestAnimationFrame(() => { if (matchMedia("(max-width: 600px)").matches) document.querySelector<HTMLButtonElement>('[aria-label="Open navigation"]')?.focus(); else document.querySelector<HTMLButtonElement>('[aria-label="Manage workspace"]')?.focus(); }); }} onConnect={() => setSetup(true)} onSettings={setSettingsAccount} onRemove={setRemovingAccount} onReconnect={(account) => { setReconnect(account); setSetup(true); }} notificationStatus={notificationStatus} onNotifications={() => {
        setNotificationStatus("Enabling…");
        void enableNotifications().then(() => setNotificationStatus("Notifications enabled")).catch((error) => setNotificationStatus(error instanceof Error ? error.message : "Could not enable notifications. Try again."));
      }} />}
      <main tabIndex={-1} className={`mailbox ${nav === "INBOX" && !q ? "compact-inbox" : ""} ${checked.size ? "has-selection" : ""}`}>
        <header className="pane-header">
          <button title="Open navigation"
            className="mobile"
            aria-label="Open navigation"
            onClick={() => setSidebar(true)}
          >
            <Menu size={20} />
          </button>
          <h1>{q ? "Search" : nav === "INBOX" ? "Inbox" : nav}</h1>
          <button title="Compose"
            className="primary list-compose"
            aria-label="Compose"
            onClick={startCompose}
          >
            <Pencil size={16} />
          </button>
          <select title="Account filter"
            aria-label="Account filter"
            value={account}
            onChange={(e) => {
              changeAccount(e.target.value);
            }}
          >
            <option value="all">All accounts</option>
            {(accounts.data || []).map((a) => (
              <option key={a.id} value={a.id}>
                {a.display_name}
              </option>
            ))}
          </select>
          <RefreshButton updating={updating} failed={refreshFailed} status={refreshStatus} onRefresh={() => void refreshMail()} />
        </header>
        {q && !sidebar && searchForm}
        <div className="view-toolbar">
          {nav === "INBOX" && !q && <div role="tablist" aria-label="Inbox view" onKeyDown={(event) => {
            if (!plainKey(event.nativeEvent) || event.repeat) return;
            if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
            event.preventDefault();
            const tabs = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]'));
            const index = tabs.indexOf(event.target as HTMLButtonElement);
            const next = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : (index + (event.key === "ArrowRight" ? 1 : tabs.length - 1)) % tabs.length;
            tabs[next]?.focus();
            tabs[next]?.click();
          }}>
            <button title="Smart"
              role="tab"
              aria-selected={view === "smart"}
              className={view === "smart" ? "active" : ""}
              onClick={() => {
                positions.current[`${account}/${nav}/${q}/${view}`] =
                  listRef.current?.scrollTop || 0;
                positions.current[`${account}/${nav}/${q}/smart`] ??=
                  listRef.current?.scrollTop || 0;
                setView("smart");
              }}
            >
              Smart
            </button>
            <button title="List"
              role="tab"
              aria-selected={view === "list"}
              className={view === "list" ? "active" : ""}
              onClick={() => {
                positions.current[`${account}/${nav}/${q}/${view}`] =
                  listRef.current?.scrollTop || 0;
                positions.current[`${account}/${nav}/${q}/list`] ??=
                  listRef.current?.scrollTop || 0;
                setView("list");
              }}
            >
              List
            </button>
          </div>}
          {checked.size > 0 && (
            <button title="Clear selection" className="selection-clear-desktop" onClick={clearSelection}>
              {checked.size} selected · Clear
            </button>
          )}

        </div>
        <div className="pull-status" role="status" style={{ height: pull.refreshing ? 44 : pull.distance }}>
          {(pull.distance > 0 || pull.refreshing) && <span><RefreshCw size={15} />{pull.refreshing ? "Refreshing…" : pull.distance >= 64 ? "Release to refresh" : "Pull to refresh"}</span>}
        </div>
        <ErrorNotice error={accounts.error || mail.error || error || deepLinkError} />

        {!active && actionFailure()}
        {!active && messageActions && <div className="selection-actions" role="group" aria-label="Selected message actions">
          <button title={`Clear selection (${checked.size} selected)`} className="selection-clear-mobile" aria-label={`Clear selection (${checked.size} selected)`} onClick={clearSelection}><span>{checked.size} selected</span><span>Clear</span></button>
          {messageActions}
        </div>}

        {ready
          .filter((a) => a.state !== "ready")
          .map((a) => (
            <div className="connection-state" key={a.id}>
              {a.email}: {a.state}
              <button title="Reconnect"
                onClick={() => {
                  setReconnect(a);
                  setSetup(true);
                }}
              >
                Reconnect
              </button>
            </div>
          ))}
        {connectionNotice}

        {latestSubmission && (
          <div className="submission" role="status">
            {submissionLabel(latestSubmission)}{" "}
            {["scheduled", "claimed"].includes(latestSubmission.state) && (
              <button title="Cancel send"
                onClick={() =>
                  void api
                    .cancelSubmission(latestSubmission)
                    .then((s) => {
                      setSubmission(s);
                      void client.invalidateQueries({
                        queryKey: ["submission"],
                      });
                    })
                    .catch(setError)
                }
              >
                Cancel send
              </button>
            )}
            {latestSubmission.state === "scheduled" && (
              <button title="Edit scheduled"
                onClick={() =>
                  void api
                    .cancelSubmission(latestSubmission)
                    .then(async (cancelled) => {
                      const draft = await api.draft(
                        cancelled.account_id,
                        cancelled.draft_id,
                      );
                      setSubmission(cancelled);
                      setCompose({
                        draft,
                        initialDue: latestSubmission.due_at,
                      });
                      void client.invalidateQueries({
                        queryKey: ["submission"],
                      });
                    })
                    .catch(setError)
                }
              >
                Edit scheduled
              </button>
            )}
            <button title="Dismiss send status"
              aria-label="Dismiss send status"
              onClick={() => setSubmission(null)}
            >
              <X size={14} />
            </button>
          </div>
        )}
        {inboxCoverage?.coverage.some(c => c.state === "error") && (
          <p role="status" className="inbox-sync-status">
            Couldn’t update inbox. Showing saved mail.
          </p>
        )}
        {q && coverage?.coverage.some(c => c.state === "error") && (
          <p role="alert" className="inbox-sync-status">Couldn’t finish checking the provider. Try searching again.</p>
        )}
        {!(accounts.data || []).length && !accounts.isPending ? (
          <Empty title="Welcome to Dakia">
            <p>Connect your first mailbox to get started.</p>
            <button title="Connect account" className="primary" onClick={() => setSetup(true)}>
              Connect account
            </button>
          </Empty>
        ) : nav === "Drafts" ? (
          <DraftList
            accounts={ready}
            onOpen={(draft) => setCompose({ draft })}
          />
        ) : nav === "Scheduled" ? (
          <ScheduledList accounts={ready} onSelect={(s) => setSubmission(s)} />
        ) : mail.isPending ? (
          <Empty title="Loading mail…" />
        ) : !rows.length && !mail.error && !mail.hasNextPage && !paginationStalled ? (
          <div className="message-list" ref={listRef}><Empty title={q ? coverage?.coverage.some(c => c.state !== "complete") ? "No matches in checked mail" : "No matches" : nav === "INBOX" && coverage?.coverage.some(c => c.state !== "complete") ? "Checking for mail…" : "All caught up"}>
            <p>
              {q ? coverage?.coverage.some(c => c.state !== "complete") ? "Search coverage is incomplete. Try searching again when the provider is available." : "Try a different search." : nav === "INBOX" && coverage?.coverage.some(c => c.state !== "complete") ? "Your inbox will update as mail arrives." : "New mail will appear here."}
            </p>
          </Empty></div>
        ) : (
          <div
            ref={listRef}
            className="message-list"
            aria-label="Messages"
            role="region"
            tabIndex={0}
            aria-busy={mail.isFetchingNextPage}
            onScroll={() => {
              infiniteScroll();
              // Mobile hides this pane while reading, which can emit a zero
              // scroll event. Keep the position captured before opening mail.
              if (listRef.current?.clientHeight) positions.current[`${account}/${nav}/${q}/${view}`] =
                listRef.current?.scrollTop || 0;
            }}
          >
            <div
              style={{ height: virtual.getTotalSize(), position: "relative" }}
            >
              {virtual.getVirtualItems().map((v) => {
                const item = items[v.index];
                return (
                  <div
                    key={v.key}
                    data-index={v.index}
                    ref={virtual.measureElement}
                    style={{
                      position: "absolute",
                      top: 0,
                      left: 0,
                      width: "100%",
                      transform: `translateY(${v.start}px)`,
                    }}
                  >
                    {item.kind === "heading" ? (
                      <button title={`${item.label} · ${item.count} loaded messages`}
                        className="group-heading"
                        aria-label={`${item.label} · ${item.count} loaded messages`}
                        aria-expanded={!item.collapsed}
                        onClick={() => updateSection(item.label, old => ({ ...old, collapsed: !item.collapsed }))}
                      >
                        <ChevronDown size={13} aria-hidden="true" style={{ transform: item.collapsed ? "rotate(-90deg)" : undefined }} />
                        {item.label}
                        <span>{item.count}</span>
                      </button>
                    ) : item.kind === "more" ? (
                      <button title={`Load more ${item.label} messages`}
                        className="section-more"
                        data-mail-more={item.label}
                        aria-label={`Load more ${item.label} messages`}
                        onClick={() => loadMore(item.label)}
                      >
                        Load more
                      </button>
                    ) : (
                      <div
                        data-message-id={identity(item.message)}
                        className={`message-row ${selected && identity(item.message) === identity(selected) ? "selected" : ""} ${item.message.unread ? "unread" : ""} ${heldReadSections[identity(item.message)]?.leaving ? "leaving-section" : ""}`}
                      >
                        <label className="message-select" onClick={(e) => {
                          // Label-forwarded checkbox clicks can lose modifier keys.
                          if (e.target instanceof HTMLInputElement) return;
                          e.preventDefault();
                          e.currentTarget.querySelector("input")?.focus();
                          selectMessage(item.message, e.shiftKey, !checked.has(identity(item.message)));
                        }}>
                        <input
                          type="checkbox"
                          aria-label={`Select ${item.message.subject || "(No subject)"}`}
                          title="Shift-click or Shift+Space to select a range"
                          checked={checked.has(identity(item.message))}
                          onChange={(e) => selectMessage(item.message,
                            (e.nativeEvent as MouseEvent).shiftKey === true, e.currentTarget.checked)}
                          onKeyDown={(e) => {
                            if (!plainKey(e.nativeEvent, true)) return;
                            if (e.key === " " && e.shiftKey) {
                              e.preventDefault();
                              if (!e.repeat) selectMessage(item.message, true, !checked.has(identity(item.message)));
                            }
                          }}
                        />
                        </label>
                        <button title={`Open ${item.message.subject || "message"}`}
                          className="message-open"
                          onFocus={() => prefetch.request(item.message)}
                          onMouseEnter={() => prefetch.request(item.message)}
                          onMouseLeave={() => prefetch.clearIntent()}
                          onBlur={() => prefetch.clearIntent()}
                          onClick={(e) => {
                            if (e.shiftKey) selectMessage(item.message, true, true);
                            else void open(item.message);
                          }}
                          onKeyDown={(e) => {
                            if (!plainKey(e.nativeEvent, true)) return;
                            if (e.shiftKey && (e.key === " " || e.key === "Enter")) {
                              e.preventDefault();
                              if (!e.repeat) selectMessage(item.message, true, true);
                            }
                          }}
                        >
                          <span className="sender" title={item.message.sender.name || item.message.sender.email || "Unknown sender"}>
                            <span
                              className={`unread-dot ${item.message.unread ? "" : "is-read"}`}
                              aria-label={item.message.unread ? "Unread" : undefined}
                            />
                            <bdi className="sender-name">{item.message.sender.name || item.message.sender.email || "Unknown sender"}</bdi>
                          </span>
                          <time dateTime={item.message.received_at} title={item.message.received_at}>{time(item.message.received_at)}</time>
                          <span className="message-summary">
                            <span className="subject" title={item.message.subject || "(No subject)"} dir="auto">
                              {item.message.subject || "(No subject)"}
                            </span>
                            {snoozedThreads[`${item.message.account_id}/${item.message.thread_id}`]
                              ? <span className="preview snooze-deadline">Snoozed until {snoozeTime(snoozedThreads[`${item.message.account_id}/${item.message.thread_id}`].until)}</span>
                              : item.message.preview && <span className="preview" dir="auto">{item.message.preview}</span>}
                          </span>
                          <span className="account-label" title={accounts.data?.find((a) => a.id === item.message.account_id)?.display_name || item.message.account_id} dir="auto">
                            {accounts.data?.find((a) => a.id === item.message.account_id)?.display_name || item.message.account_id}
                          </span>
                        </button>
                        <div className="row-quick-actions" role="group" aria-label="Quick message actions">
                          {snoozedThreads[`${item.message.account_id}/${item.message.thread_id}`] && <IconButton label="Move back to Inbox" disabled={pendingTargets[identity(item.message)] > 0} onClick={() => void restoreInbox([item.message])}><FolderInput size={16} /></IconButton>}
                          {([
                            ["archive", "Archive message", "Archive", Archive],
                            ["trash", "Move message to trash", "Move to Trash", Trash2],
                            ["spam", "Move message to spam", "Move to spam", ShieldAlert],
                          ] as const).map(([kind, label, title, Icon]) => (
                            <button
                              key={kind}
                              aria-label={label}
                              title={title}
                              disabled={pendingTargets[identity(item.message)] > 0 || !!cleanupRows[identity(item.message)]}
                              onClick={() => {
                                void act([item.message], { kind });
                                if (selected && identity(selected) === identity(item.message)) closeReader();
                              }}
                            >
                              <Icon size={16} aria-hidden="true" />
                            </button>
                          ))}
                        </div>
                        <button title={
                            item.message.starred
                              ? "Unstar message"
                              : "Star message"
                          }
                          className={`star ${item.message.starred ? "starred" : ""}`}
                          disabled={pendingTargets[identity(item.message)] > 0 || !!cleanupRows[identity(item.message)]}
                          aria-label={
                            item.message.starred
                              ? "Unstar message"
                              : "Star message"
                          }
                          onClick={() =>
                            void act([item.message], {
                              kind: "star",
                              value: !item.message.starred,
                            })
                          }
                        >
                          <Star
                            size={15}
                            fill={
                              item.message.starred ? "currentColor" : "none"
                            }
                          />
                        </button>
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
            {(bottomSection !== null || (mail.hasNextPage && !mail.isFetchNextPageError)) && (
              <button title={view === "smart" && !q && nav === "INBOX" ? "Load more mail" : "Load more"}
                className="load-more"
                data-mail-more={bottomSection || (items.some(item => item.kind === "mail") || !rows.length ? "" : undefined)}
                aria-label={view === "smart" && !q && nav === "INBOX" ? "Load more mail" : "Load more"}
                disabled={bottomSection === null && mail.isFetching}
                onClick={() => loadMore(bottomSection || "")}
              >
                {bottomSection === null && mail.isFetchingNextPage ? "Loading…" : "Load more"}
              </button>
            )}
            <p className="mail-load-status" role="status">
              {mail.isFetchNextPageError ? "Couldn’t load more mail. Your loaded messages are still available." : mail.isFetchingNextPage ? "Loading more mail…" : paginationStalled ? "Paging stopped because the mailbox returned a repeated page. Refresh mail to try again." : bottomSection === null && !mail.hasNextPage && rows.length ? "End of loaded mail" : ""}
            </p>
            {mail.isFetchNextPageError && <button title="Retry loading mail" className="load-more" disabled={mail.isFetching} onClick={() => loadMore("")}>Retry loading mail</button>}
          </div>
        )}
        <div className="keyboard-hints">
          <span><kbd>Tab</kbd> Navigate controls</span><span><kbd>Enter</kbd> Activate</span>
          {keyboardPreferences.hints && keyboardPreferences.shortcuts && <>
            <span><kbd>J</kbd> / <kbd>K</kbd> Next / previous</span><span><kbd>E</kbd> Archive</span><span><kbd>R</kbd> Reply</span>
            <button title="Keyboard shortcuts" onClick={() => setShortcutHelp(true)} aria-label="Keyboard shortcuts"><kbd>?</kbd> All shortcuts</button>
          </>}
        </div>
      </main>
      <section className="reader" aria-label="Reading pane" tabIndex={-1}>
        <header className="pane-header">
          <button title="Back to inbox"
            className="reader-back"
            aria-label="Back to inbox"
            onClick={closeReader}
          >
            <ArrowLeft size={18} />
          </button>
          <button title="Previous email" aria-label="Previous email" disabled={!active || selectionOrder.indexOf(identity(active)) <= 0} onClick={() => navigateMessage(-1)}><ArrowUp size={16} /><KeyHint keys="K" /></button>
          <button title="Next email" aria-label="Next email" disabled={!active || selectionOrder.indexOf(identity(active)) < 0 || selectionOrder.indexOf(identity(active)) >= selectionOrder.length - 1} onClick={() => navigateMessage(1)}><ArrowDown size={16} /><KeyHint keys="J" /></button>
          <span>
            {active
              ? accounts.data?.find((a) => a.id === active.account_id)?.email
              : "Mail"}
          </span>
          <button title="Compose" className="primary" onClick={startCompose}>
            <Pencil size={15} />
            Compose <KeyHint keys="C" />
          </button>
        </header>
        {connectionNotice}
        {active && actionFailure()}
        {active && checked.size > 0 && <button title="Clear selection" onClick={clearSelection}>{checked.size} selected messages · Clear</button>}

        {active && messageActions}
        {active && snoozedThreads[`${active.account_id}/${active.thread_id}`] && <p className="snooze-notice">Snoozed until {snoozeTime(snoozedThreads[`${active.account_id}/${active.thread_id}`].until)}</p>}
        {active ? (
          <Conversation key={identity(active)} selected={active} accounts={accounts.data || []} cleanupRows={cleanupRows}
            readFailed={!!readFailures[identity(active)] && active.unread}
            dark={dark} alwaysLoadImages={alwaysLoadImages} blockHiddenImages={blockHiddenImages} onReply={reply}
            onDraft={(draft) => setCompose({ draft })}
            onCleanup={targets => { clearSelection(); if (active && targets.some(m => identity(m) === identity(active))) closeReader(); }} />
        ) : (
          <Empty title="A little room to focus">
            <Mail size={30} />
            <p>Select a message to read it here.</p>
          </Empty>
        )}
      </section>

      {settingsAccount && <MailboxSettingsDialog account={settingsAccount} onClose={() => setSettingsAccount(null)} />}
      {removingAccount && <AccountRemoval account={removingAccount} onClose={() => setRemovingAccount(null)} onRemoved={() => { void localMailCache.clear().finally(() => { client.clear(); window.location.assign("/"); }); }} />}
      {setup && (
        <AccountSetup
          account={reconnect}
          onClose={() => {
            setSetup(false);
            setReconnect(null);
          }}
        />
      )}{" "}
      {compose && (
        <Composer
          initial={compose.draft}
          initialDue={compose.initialDue}
          quote={compose.quote}
          forwardAttachments={compose.forwardAttachments}
          accounts={accounts.data || []}
          onClose={() => setCompose(null)}
          onSubmitted={(s) => {
            setSubmission(s);
            void client.invalidateQueries({ queryKey: ["mail"] });
          }}
        />
      )}
      <Toast announcementKey={toast?.id}>{toast?.message}</Toast>
      {snooze && <DateTimeDialog title="Snooze until" confirmLabel="Snooze"
        onChoose={until => { void snoozeMessages([...snooze], until); }}
        onClose={() => setSnooze(null)} />}
      {move && <MoveMessages messages={move} accounts={accounts.data || []}
        onMove={(messages, destinations) => void act(messages, { kind: "move", folder_id: "" }, destinations)}
        onClose={() => setMove(null)} />}
      {shortcutHelp && <Modal title="Keyboard shortcuts" className="keyboard-dialog" onClose={() => setShortcutHelp(false)}>
        <div className="shortcut-help">
          <p className="shortcut-intro">{keyboardPreferences.shortcuts ? "Use these keys when you’re not typing or selecting text." : "Mail shortcuts are off. Enable them in Workspace settings."}</p>
          <div className="shortcut-groups">
            <div>{shortcutGroups.slice(0, 2).map(group => <section key={group.title} aria-label={group.title}>
              <h3>{group.title}</h3><dl className="shortcut-list">{group.shortcuts.map(([keys, label]) => <div key={keys}><dt>{label}</dt><dd><ShortcutKeys keys={keys} /></dd></div>)}</dl>
            </section>)}</div>
            <section aria-label={shortcutGroups[2].title}><h3>{shortcutGroups[2].title}</h3><dl className="shortcut-list">{shortcutGroups[2].shortcuts.map(([keys, label]) => <div key={keys}><dt>{label}</dt><dd><ShortcutKeys keys={keys} /></dd></div>)}</dl></section>
          </div>
          <p className="muted">{menuKeyboardHint}</p>
          <div className="shortcut-basics"><p><ShortcutKeys keys="Tab / Shift+Tab" /> Move between controls and fields</p><p><ShortcutKeys keys="Enter / Space" /> Activate buttons</p></div>
        </div>
      </Modal>}
    </div>
    </KeyboardHints.Provider>
  );
}

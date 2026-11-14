import { useCallback, useEffect, useRef, useState } from "react";
import { useQueries, useQueryClient } from "@tanstack/react-query";
import type { SearchPage, SearchSupplement } from "@dakia/api-types";
import { api } from "./api";

type Token = SearchSupplement & { epoch: number; generation: number };
type Lane = Token & { root: string };
type Root = Token & { previous?: SearchPage[] };
const sameLane = (a: Token, b: Token) => a.account_id === b.account_id && a.continuation === b.continuation && a.epoch === b.epoch && a.generation === b.generation;
const restartCodes = new Set(["provider_reference_changed", "refetch_required", "cursor_expired"]);
const terminalCodes = new Set([...restartCodes, "account_reconnect_required", "imap_authentication_rejected", "smtp_authentication_rejected", "provider_unsupported", "invalid_search", "invalid_mail_request", "folder_mapping_required"]);
function failureCode(error: unknown) {
  return typeof error === "object" && error !== null && "code" in error ? String(error.code) : "";
}
function needsRestart(data: SearchPage | undefined, error: unknown) {
  return restartCodes.has(failureCode(error)) || !!data?.coverage.some(c => c.state === "error" && restartCodes.has(c.error || ""));
}
function failed(data: SearchPage | undefined, error: unknown) {
  return !!error || !!data?.coverage.some(c => c.state === "error");
}
function retryInterval(query: { state: { data?: SearchPage; error: unknown; dataUpdateCount: number; fetchFailureCount: number } }): number | false {
  const state = query.state;
  const transport = state.error as { status?: number } | null;
  const transient = transport
    ? !terminalCodes.has(failureCode(transport)) && (transport.status === 0 || Number(transport.status) >= 500)
    : state.data?.coverage.some(c => c.state === "error" && !terminalCodes.has(c.error || ""));
  return transient ? Math.min(30000, 5000 * 2 ** Math.min(3, Math.max(0, state.dataUpdateCount + state.fetchFailureCount - 1))) : false;
}

// Each account has an independent provider lane. A root remains pinned while
// work is in flight. New accounts can join without resetting existing pages.
export function useSearchSupplements(q: string, ids: string[], category: string | undefined, incoming: SearchSupplement[], primaryReady = true, cacheScope = category) {
  const client = useQueryClient();
  const identity = JSON.stringify([q, ids, category, cacheScope]);
  const scopeLifetime = useRef({ identity, token: {}, revision: 0 });
  if (scopeLifetime.current.identity !== identity) scopeLifetime.current = { identity, token: {}, revision: scopeLifetime.current.revision + 1 };
  const lifetime = scopeLifetime.current.token;
  const scopeRevision = scopeLifetime.current.revision;
  const generations=useQueries({queries:ids.map(account=>({queryKey:["search-mail-generation",account],queryFn:async()=>0,enabled:false,initialData:0,staleTime:Infinity}))});
  const generationOf=(account:string)=>generations[ids.indexOf(account)]?.data||0;
  const adopt=(token:SearchSupplement,epoch=0,previous?:SearchPage[]):Root=>({...token,epoch,generation:client.getQueryData<number>(["search-mail-generation",token.account_id])||0,previous});
  const [refreshVersion, setRefreshVersion] = useState({ scope: "", value: 0 });
  const version = refreshVersion.scope === identity ? refreshVersion.value : 0;
  const [additional, setAdditional] = useState<{ scope: string; tokens: Lane[] }>({ scope: "", tokens: [] });
  const extra = additional.scope === identity ? additional.tokens : [];
  const key = (token: Token) => ["search-supplement", q, ids, category, token.account_id, token.continuation, version, token.epoch, token.generation, scopeRevision];
  const pinned = useRef<{identity:string;roots:Root[]}>({ identity:"", roots:[] });
  const belongs=(token:Lane,root:Root)=>token.account_id===root.account_id && token.root===root.continuation && token.epoch===root.epoch && token.generation===root.generation;
  if (pinned.current.identity !== identity) pinned.current = { identity, roots: primaryReady ? incoming.map(token=>adopt(token)) : [] };
  else if(primaryReady) {
    const retained = pinned.current.roots.flatMap(original => {
      const replacement = incoming.find(next => next.account_id === original.account_id);
      if (!replacement) return [];
      const states = [original, ...extra.filter(token => belongs(token,original))].map(token => client.getQueryState<SearchPage>(key(token)));
      const pages=states.flatMap(state=>state?.data?[state.data]:[]);
      const settled=states.every(state=>!!state && state.fetchStatus==="idle" && state.status!=="pending");
      const restart=states.some(state=>needsRestart(state?.data,state?.error));
      const progressed=pages.some(page=>page.rows.length>0 || (!page.pending && !page.continuation && page.coverage.every(c=>c.state==="complete")));
      const root=original.previous && progressed ? {...original,previous:undefined} : original;
      if(restart && replacement.continuation!==root.continuation) return [adopt(replacement,root.epoch+1)];
      if(root.generation!==generationOf(root.account_id) && settled) {
        return [adopt(replacement,root.epoch+1,root.previous||pages.filter(page=>page.rows.length>0))];
      }
      return [root];
    });
    const added = incoming.filter(root => !retained.some(old => old.account_id === root.account_id)).map(token=>adopt(token));
    pinned.current = { identity, roots: [...retained, ...added] };
  }
  const roots = pinned.current.roots;
  const tokens: Lane[] = [...roots.map(root => ({ ...root, root: root.continuation })), ...extra.filter(token => roots.some(root => belongs(token,root)))];
  const rootsKey=JSON.stringify(roots.map(root=>[root.account_id,root.continuation,root.epoch,root.generation]));
  useEffect(()=>{
    setAdditional(old=>{
      if(old.scope!==identity) return old;
      const retained=old.tokens.filter(token=>roots.some(root=>belongs(token,root)));
      return retained.length===old.tokens.length ? old : {...old,tokens:retained};
    });
  },[identity,rootsKey]);
  const queries = useQueries({ queries: tokens.map(token => ({
    queryKey: key(token),
    queryFn: ({ signal }: { signal: AbortSignal }) => api.search(q, ids, token.continuation, category, signal, false, false, true),
    staleTime: 30000,
    retry: false,
    refetchOnWindowFocus: false,
    refetchInterval: retryInterval,
  })) });
  const pages = roots.flatMap(root=>{
    const current=queries.flatMap((query,index)=>belongs(tokens[index],root) && query.data ? [query.data] : []);
    const progressed=current.some(page=>page.rows.length>0 || (!page.pending && !page.continuation && page.coverage.every(c=>c.state==="complete")));
    return !progressed && root.previous ? [...root.previous,...current] : current;
  });
  const following = queries.flatMap((query, index) => {
    const continuation=query.data?.continuation;
    const parent=tokens[index];
    const next=continuation ? {account_id:parent.account_id,continuation,root:parent.root,epoch:parent.epoch,generation:parent.generation} : undefined;
    return next && !tokens.some(token=>sameLane(token,next)) ? [next] : [];
  });
  const emptyKey = JSON.stringify(following.filter(token => {
    const parent = queries.find((query,index) => query.data?.continuation === token.continuation && tokens[index].account_id===token.account_id && tokens[index].epoch===token.epoch && tokens[index].generation===token.generation);
    return parent?.data && !parent.data.rows.length;
  }));
  useEffect(() => {
    const next: Lane[] = JSON.parse(emptyKey);
    if (!next.length) return;
    setAdditional(old => ({ scope: identity, tokens: [...(old.scope === identity ? old.tokens : []), ...next.filter(token => !(old.scope === identity && old.tokens.some(t => sameLane(t,token))))] }));
  }, [identity, emptyKey]);
  const rootPredicate = useCallback((query: { queryKey: readonly unknown[] }) => {
    const k = query.queryKey;
    return k[0] === "search-page" && k[1] === q && JSON.stringify(k[2]) === JSON.stringify(ids) && k[3] === category
      || k[0] === "mail" && k[3] === q && JSON.stringify(k[5]) === JSON.stringify(ids) && k[4] === cacheScope;
  }, [identity, client]);
  const restart = queries.some(query => needsRestart(query.data, query.error));
  useEffect(() => {
    if (!restart) return;
    const timer = setTimeout(() => { void client.invalidateQueries({ predicate: rootPredicate }, { cancelRefetch: false }); }, 5000);
    return () => clearTimeout(timer);
  }, [client, rootPredicate, restart]);
  const retry = useCallback(() => {
    const active = client.getQueryCache().findAll({ type: "active", predicate: query => {
      const k = query.queryKey;
      return k[0] === "search-supplement" && k[1] === q && JSON.stringify(k[2]) === JSON.stringify(ids) && k[3] === category;
    } });
    for (const query of active) {
      const data = query.state.data as SearchPage | undefined;
      if (needsRestart(data, query.state.error)) void client.invalidateQueries({ predicate: rootPredicate }, { cancelRefetch: false });
      else if (failed(data, query.state.error)) void client.refetchQueries({ queryKey: query.queryKey, exact: true });
    }
  }, [client, identity, rootPredicate]);
  const refresh = useCallback((fresh: SearchSupplement[]) => {
    // A primary refresh may finish after the user has submitted another query.
    if (pinned.current.identity !== identity || scopeLifetime.current.token !== lifetime) return;
    void client.cancelQueries({ predicate: query => {
      const k = query.queryKey;
      return k[0] === "search-supplement" && k[1] === q && JSON.stringify(k[2]) === JSON.stringify(ids) && k[3] === category;
    } });
    const previous=pinned.current.roots;
    pinned.current = { identity, roots: fresh.map(token=>adopt(token,(previous.find(root=>root.account_id===token.account_id)?.epoch??-1)+1)) };
    setAdditional({ scope: identity, tokens: [] });
    // A new key prevents completed heads, obsolete continuations and late
    // responses from being reused, even when the root token itself is unchanged.
    setRefreshVersion(old => ({ scope: identity, value: (old.scope === identity ? old.value : 0) + 1 }));
  }, [client, identity, lifetime]);
  const stalled = queries.some((query, index) => {
    const token = query.data?.continuation;
    const earlier = token ? tokens.findIndex(t => sameLane(t,{...tokens[index],continuation:token})) : -1;
    return earlier >= 0 && earlier <= index;
  });
  return {
    pages,
    isFetching: queries.some(query => query.isFetching),
    error: queries.find(query => query.error)?.error,
    hasError: queries.some(query => failed(query.data, query.error)),
    hasNextPage: following.length > 0,
    stalled,
    loadMore: () => setAdditional(old => ({ scope: identity, tokens: [...(old.scope === identity ? old.tokens : []), ...following.filter(token => !(old.scope === identity && old.tokens.some(t => sameLane(t,token))))] })),
    retry,
    refresh,
  };
}

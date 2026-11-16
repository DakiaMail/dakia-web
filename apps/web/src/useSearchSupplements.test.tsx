import { createElement } from "react";
import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider, useQuery } from "@tanstack/react-query";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { SearchPage } from "@dakia/api-types";
import { api } from "./api";
import { messages } from "../browser/fixtures";
import { useSearchSupplements } from "./useSearchSupplements";
vi.mock("./api", () => ({ api: { search: vi.fn() } }));
const roots=[{account_id:"work",continuation:"work-start"},{account_id:"personal",continuation:"personal-start"}];
function page(account: string, continuation: string | null=null):SearchPage {
  return {rows:[],continuation,coverage:[{account_id:account,folder_ids:["INBOX"],state:"complete",error:null}],classification:{classified:0,total:0},pending:!!continuation,counts_exact:false};
}
function mount() {
  const client=new QueryClient({defaultOptions:{queries:{retry:false,gcTime:0}}});
  const wrapper=({children}:{children:React.ReactNode})=>createElement(QueryClientProvider,{client},children);
  return {client,wrapper};
}
afterEach(()=>{vi.useRealTimers();vi.resetAllMocks();});
describe("independent provider search",()=>{
  it("starts accounts together and exposes one response while another is pending",async()=>{
    let finish!:(page:SearchPage)=>void;
    vi.mocked(api.search).mockImplementation(async(_q,_ids,token)=>token==="work-start"?new Promise(resolve=>{finish=resolve;}):page("personal"));
    const {wrapper,client}=mount();
    const hook=renderHook(()=>useSearchSupplements("parcel",["work","personal"],undefined,roots),{wrapper});
    await waitFor(()=>expect(api.search).toHaveBeenCalledTimes(2));
    await waitFor(()=>expect(hook.result.current.pages).toHaveLength(1));
    expect(hook.result.current.pages[0].coverage[0].account_id).toBe("personal");
    expect(hook.result.current.isFetching).toBe(true);
    await act(async()=>finish(page("work")));
    await waitFor(()=>expect(hook.result.current.pages).toHaveLength(2));
    hook.unmount();client.clear();
  });
  it("advances empty provider folders and stops repeated opaque continuations",async()=>{
    vi.mocked(api.search).mockImplementation(async(_q,_ids,token)=>page("work",token==="work-start"?"next":"next"));
    const {wrapper,client}=mount();
    const hook=renderHook(()=>useSearchSupplements("parcel",["work"],undefined,[roots[0]]),{wrapper});
    await waitFor(()=>expect(api.search).toHaveBeenCalledTimes(2));
    await waitFor(()=>expect(hook.result.current.stalled).toBe(true));
    expect(hook.result.current.hasNextPage).toBe(false);
    hook.unmount();client.clear();
  });
  it("cancels obsolete searches without replacing the current query",async()=>{
    const aborted:string[]=[];
    vi.mocked(api.search).mockImplementation(async(q,_ids,_token,_category,signal)=>{
      if(q==="new")return page("work");
      return new Promise((_resolve,reject)=>signal?.addEventListener("abort",()=>{aborted.push(q);reject(new DOMException("Aborted","AbortError"));}));
    });
    const {wrapper,client}=mount();
    const hook=renderHook(({q})=>useSearchSupplements(q,["work"],undefined,[roots[0]]),{wrapper,initialProps:{q:"old"}});
    await waitFor(()=>expect(api.search).toHaveBeenCalledTimes(1));
    hook.rerender({q:"new"});
    await waitFor(()=>expect(aborted).toEqual(["old"]));
    await waitFor(()=>expect(hook.result.current.pages).toHaveLength(1));
    hook.unmount();client.clear();
  });
  it("keeps provider work alive through index refreshes and stops it when caught up",async()=>{
    const aborted:string[]=[];
    vi.mocked(api.search).mockImplementation(async(_q,_ids,token,_category,signal)=>new Promise((_resolve,reject)=>signal?.addEventListener("abort",()=>{aborted.push(token!);reject(new DOMException("Aborted","AbortError"));})));
    const {wrapper,client}=mount();
    const hook=renderHook(({roots})=>useSearchSupplements("parcel",["work"],undefined,roots),{wrapper,initialProps:{roots:[{account_id:"work",continuation:"original"}]}});
    await waitFor(()=>expect(api.search).toHaveBeenCalledTimes(1));
    hook.rerender({roots:[{account_id:"work",continuation:"refreshed-index"}]});
    expect(api.search).toHaveBeenCalledTimes(1);expect(aborted).toEqual([]);
    hook.rerender({roots:[]});
    await waitFor(()=>expect(aborted).toEqual(["original"]));
    hook.unmount();client.clear();
  });

  it("retries temporary account failures without restarting healthy work",async()=>{
    vi.useFakeTimers();
    let attempts=0;
    vi.mocked(api.search).mockImplementation(async(_q,_ids,token)=>{
      if(token!=="work-start")return page("personal");
      attempts++;
      const result=page("work");
      if(attempts===1)result.coverage[0]={...result.coverage[0],state:"error",error:"provider_unavailable"};
      return result;
    });
    const {wrapper,client}=mount();
    const hook=renderHook(()=>useSearchSupplements("parcel",["work","personal"],undefined,roots),{wrapper});
    await act(async()=>{await vi.advanceTimersByTimeAsync(1);});
    expect(hook.result.current.hasError).toBe(true);
    await act(async()=>{await vi.advanceTimersByTimeAsync(5000);});
    expect(hook.result.current.hasError).toBe(false);
    expect(attempts).toBe(2);
    expect(vi.mocked(api.search).mock.calls.filter(call=>call[2]==="personal-start")).toHaveLength(1);
    hook.unmount();client.clear();
  });
  it("recovers from a network failure without restarting healthy accounts",async()=>{
    vi.useFakeTimers();
    let attempts=0;
    vi.mocked(api.search).mockImplementation(async(_q,_ids,token)=>{
      if(token!=="work-start")return page("personal");
      if(++attempts===1)throw new TypeError("Failed to fetch");
      return page("work");
    });
    const {wrapper,client}=mount();
    const hook=renderHook(()=>useSearchSupplements("parcel",["work","personal"],undefined,roots),{wrapper});
    await act(async()=>{await vi.advanceTimersByTimeAsync(1);});
    expect(hook.result.current.hasError).toBe(true);
    await act(async()=>{await vi.advanceTimersByTimeAsync(5000);});
    expect(attempts).toBe(2);
    expect(hook.result.current.hasError).toBe(false);
    expect(hook.result.current.pages).toHaveLength(2);
    expect(vi.mocked(api.search).mock.calls.filter(call=>call[2]==="personal-start")).toHaveLength(1);
    hook.unmount();client.clear();
  });
  it.each([
    new DOMException("Aborted","AbortError"),
    Object.assign(new Error("Sign in"),{status:401}),
    Object.assign(new Error("Reconnect"),{status:503,code:"account_reconnect_required"}),
  ])("does not automatically retry cancellation or terminal errors: %s",async(error)=>{
    vi.useFakeTimers();
    vi.mocked(api.search).mockRejectedValue(error);
    const {wrapper,client}=mount();
    const hook=renderHook(()=>useSearchSupplements("parcel",["work"],undefined,[roots[0]]),{wrapper});
    await act(async()=>{await vi.advanceTimersByTimeAsync(60000);});
    expect(api.search).toHaveBeenCalledTimes(1);
    hook.unmount();client.clear();
  });
  it("replaces expired provider roots and lets new accounts join independently",async()=>{
    vi.mocked(api.search).mockImplementation(async(_q,_ids,token)=>{
      const result=page(token==="personal-start"?"personal":"work");
      if(token==="expired")result.coverage[0]={...result.coverage[0],state:"error",error:"cursor_expired"};
      return result;
    });
    const {wrapper,client}=mount();
    const hook=renderHook(({incoming})=>useSearchSupplements("parcel",["work","personal"],undefined,incoming),{wrapper,initialProps:{incoming:[{account_id:"work",continuation:"expired"}]}});
    await waitFor(()=>expect(hook.result.current.hasError).toBe(true));
    hook.rerender({incoming:[{account_id:"work",continuation:"fresh"},roots[1]]});
    await waitFor(()=>expect(hook.result.current.pages).toHaveLength(2));
    await waitFor(()=>expect(hook.result.current.hasError).toBe(false));
    expect(vi.mocked(api.search).mock.calls.map(call=>call[2])).toEqual(["expired","fresh","personal-start"]);
    hook.unmount();client.clear();
  });
  it("manually retries a failed account without refetching completed accounts",async()=>{
    let attempts=0;
    vi.mocked(api.search).mockImplementation(async(_q,_ids,token)=>{
      if(token!=="work-start")return page("personal");
      if(++attempts===1)throw Object.assign(new Error("Unavailable"),{status:503});
      return page("work");
    });
    const {wrapper,client}=mount();
    const hook=renderHook(()=>useSearchSupplements("parcel",["work","personal"],undefined,roots),{wrapper});
    await waitFor(()=>expect(hook.result.current.hasError).toBe(true));
    act(()=>hook.result.current.retry());
    await waitFor(()=>expect(hook.result.current.hasError).toBe(false));
    expect(attempts).toBe(2);
    expect(vi.mocked(api.search).mock.calls.filter(call=>call[2]==="personal-start")).toHaveLength(1);
    hook.unmount();client.clear();
  });

  it("explicitly refreshes completed heads and discards the old pagination chain",async()=>{
    const attempts=new Map<string,number>();
    vi.mocked(api.search).mockImplementation(async(_q,_ids,token)=>{
      const n=(attempts.get(token!)||0)+1;attempts.set(token!,n);
      const result=page("work",token==="work-start"&&n===1?"obsolete-next":null);
      result.rows=[{...messages[0],id:token==="obsolete-next"?"old-tail":n===1?"old-head":"fresh-head"}];
      return result;
    });
    const {wrapper,client}=mount();
    const hook=renderHook(()=>useSearchSupplements("parcel",["work"],undefined,[roots[0]]),{wrapper});
    await waitFor(()=>expect(hook.result.current.pages).toHaveLength(1));
    act(()=>hook.result.current.loadMore());
    await waitFor(()=>expect(hook.result.current.pages).toHaveLength(2));
    act(()=>hook.result.current.refresh([roots[0]]));
    await waitFor(()=>expect(hook.result.current.pages[0]?.rows[0].id).toBe("fresh-head"));
    expect(hook.result.current.pages).toHaveLength(1);
    expect(attempts.get("work-start")).toBe(2);
    expect(attempts.get("obsolete-next")).toBe(1);
    hook.unmount();client.clear();
  });
  it("cancels obsolete provider work when explicit refresh supplies fresh roots",async()=>{
    let oldSignal:AbortSignal|undefined;
    let finish!:(value:SearchPage)=>void;
    vi.mocked(api.search).mockImplementation(async(_q,_ids,token,_category,signal)=>{
      if(token==="work-start"){oldSignal=signal;return new Promise(resolve=>{finish=resolve;});}
      return page("work");
    });
    const {wrapper,client}=mount();
    const hook=renderHook(()=>useSearchSupplements("parcel",["work"],undefined,[roots[0]]),{wrapper});
    await waitFor(()=>expect(api.search).toHaveBeenCalledTimes(1));
    act(()=>hook.result.current.refresh([{account_id:"work",continuation:"fresh-root"}]));
    await waitFor(()=>expect(hook.result.current.pages).toHaveLength(1));
    expect(oldSignal?.aborted).toBe(true);
    await act(async()=>finish({...page("work"),pending:true}));
    expect(hook.result.current.pages[0].pending).toBe(false);
    expect(vi.mocked(api.search).mock.calls.map(call=>call[2])).toEqual(["work-start","fresh-root"]);
    hook.unmount();client.clear();
  });
  it("ignores a late explicit refresh after leaving and returning to the same query",async()=>{
    vi.mocked(api.search).mockResolvedValue(page("work"));
    const {wrapper,client}=mount();
    const hook=renderHook(({q})=>useSearchSupplements(q,["work"],undefined,[roots[0]]),{wrapper,initialProps:{q:"parcel"}});
    await waitFor(()=>expect(hook.result.current.pages).toHaveLength(1));
    const obsoleteRefresh=hook.result.current.refresh;
    hook.rerender({q:"garden"});
    await waitFor(()=>expect(api.search).toHaveBeenCalledTimes(2));
    hook.rerender({q:"parcel"});
    await waitFor(()=>expect(hook.result.current.pages).toHaveLength(1));
    const calls=vi.mocked(api.search).mock.calls.length;
    act(()=>obsoleteRefresh([{account_id:"work",continuation:"obsolete-root"}]));
    expect(api.search).toHaveBeenCalledTimes(calls);
    expect(vi.mocked(api.search).mock.calls.some(call=>call[2]==="obsolete-root")).toBe(false);
    hook.unmount();client.clear();
  });

  it("stops automatic retries when a temporary failure becomes an authentication error",async()=>{
    vi.useFakeTimers();
    let attempts=0;
    vi.mocked(api.search).mockImplementation(async()=>{
      attempts++;
      if(attempts>1)throw Object.assign(new Error("Reconnect"),{status:401,code:"account_reconnect_required"});
      const result=page("work");
      result.coverage[0]={...result.coverage[0],state:"error",error:"provider_unavailable"};
      return result;
    });
    const {wrapper,client}=mount();
    const hook=renderHook(()=>useSearchSupplements("parcel",["work"],undefined,[roots[0]]),{wrapper});
    await act(async()=>{await vi.advanceTimersByTimeAsync(1);});
    expect(attempts).toBe(1);
    await act(async()=>{await vi.advanceTimersByTimeAsync(5000);});
    expect(attempts).toBe(2);
    await act(async()=>{await vi.advanceTimersByTimeAsync(60000);});
    expect(attempts).toBe(2);
    expect(hook.result.current.hasError).toBe(true);
    hook.unmount();client.clear();
  });

  it("obtains a fresh root after an expired continuation without user input",async()=>{
    vi.useFakeTimers();
    let discoveries=0;
    vi.mocked(api.search).mockImplementation(async(_q,_ids,token)=>{
      const result=page("work");
      if(token==="expired")result.coverage[0]={...result.coverage[0],state:"error",error:"refetch_required"};
      return result;
    });
    const {wrapper,client}=mount();
    const hook=renderHook(()=>{
      const discovery=useQuery({queryKey:["search-page","parcel",["work"],undefined],queryFn:async()=>({supplement:[{account_id:"work",continuation:++discoveries===1?"expired":"fresh"}]}),staleTime:Infinity});
      return useSearchSupplements("parcel",["work"],undefined,discovery.data?.supplement||[]);
    },{wrapper});
    await act(async()=>{await vi.advanceTimersByTimeAsync(20);});
    expect(hook.result.current.hasError).toBe(true);
    await act(async()=>{await vi.advanceTimersByTimeAsync(5020);});
    expect(discoveries).toBe(2);
    expect(vi.mocked(api.search).mock.calls.map(call=>call[2])).toEqual(["expired","fresh"]);
    expect(hook.result.current.hasError).toBe(false);
    hook.unmount();client.clear();
  });

});

describe("provider refresh after mail changes",()=>{
  it("refreshes only the changed completed account and replaces its pagination chain",async()=>{
    let finish!:(value:SearchPage)=>void;
    vi.mocked(api.search).mockImplementation(async(_q,_ids,token)=>{
      if(token==="fresh-work") return new Promise(resolve=>{finish=resolve;});
      const data=page(token==="personal-start"?"personal":"work",token==="work-start"?"old-tail":null);
      data.rows=[{...messages[0],id:token!,account_id:token==="personal-start"?"personal":"work"}];
      return data;
    });
    const {wrapper,client}=mount();
    const hook=renderHook(({incoming})=>useSearchSupplements("parcel",["work","personal"],undefined,incoming),{wrapper,initialProps:{incoming:roots}});
    await waitFor(()=>expect(hook.result.current.pages).toHaveLength(2));
    act(()=>hook.result.current.loadMore());
    await waitFor(()=>expect(hook.result.current.pages).toHaveLength(3));
    hook.rerender({incoming:[{account_id:"work",continuation:"fresh-work"},roots[1]]});
    expect(api.search).toHaveBeenCalledTimes(3); // Index refresh alone keeps the completed root.
    act(()=>client.setQueryData(["search-mail-generation","work"],1));
    await waitFor(()=>expect(api.search).toHaveBeenCalledTimes(4));
    expect(hook.result.current.pages.flatMap(p=>p.rows.map(m=>m.id))).toContain("old-tail");
    const fresh=page("work");fresh.rows=[{...messages[0],id:"new-arrival"}];
    await act(async()=>finish(fresh));
    await waitFor(()=>expect(hook.result.current.pages.flatMap(p=>p.rows.map(m=>m.id))).toEqual(["new-arrival","personal-start"]));
    expect(vi.mocked(api.search).mock.calls.filter(call=>call[2]==="personal-start")).toHaveLength(1);
    expect(vi.mocked(api.search).mock.calls.filter(call=>call[2]==="old-tail")).toHaveLength(1);
    expect(hook.result.current.hasNextPage).toBe(false);
    hook.unmount();client.clear();
  });

  it("keeps in-flight work and catches up once with the latest primary root",async()=>{
    let finish!:(value:SearchPage)=>void;let signal:AbortSignal|undefined;
    vi.mocked(api.search).mockImplementation(async(_q,_ids,token,_category,abort)=>{
      if(token==="work-start"){signal=abort;return new Promise(resolve=>{finish=resolve;});}
      const result=page("work");result.rows=[{...messages[0],id:token!}];return result;
    });
    const {wrapper,client}=mount();
    const hook=renderHook(({incoming})=>useSearchSupplements("parcel",["work"],undefined,incoming),{wrapper,initialProps:{incoming:[roots[0]]}});
    await waitFor(()=>expect(api.search).toHaveBeenCalledTimes(1));
    act(()=>client.setQueryData(["search-mail-generation","work"],1));
    hook.rerender({incoming:[{account_id:"work",continuation:"intermediate"}]});
    act(()=>client.setQueryData(["search-mail-generation","work"],2));
    hook.rerender({incoming:[{account_id:"work",continuation:"latest"}]});
    expect(signal?.aborted).toBe(false);expect(api.search).toHaveBeenCalledTimes(1);
    await act(async()=>finish(page("work")));
    await waitFor(()=>expect(hook.result.current.pages[0]?.rows[0]?.id).toBe("latest"));
    expect(vi.mocked(api.search).mock.calls.map(call=>call[2])).toEqual(["work-start","latest"]);
    hook.unmount();client.clear();
  });

  it("waits for the first fresh primary response instead of querying cached roots",async()=>{
    vi.mocked(api.search).mockResolvedValue(page("work"));
    const {wrapper,client}=mount();
    const hook=renderHook(({ready,incoming})=>useSearchSupplements("parcel",["work"],undefined,incoming,ready),{wrapper,initialProps:{ready:false,incoming:[roots[0]]}});
    await act(async()=>{});expect(api.search).not.toHaveBeenCalled();
    hook.rerender({ready:false,incoming:[{account_id:"work",continuation:"fresh"}]});
    expect(api.search).not.toHaveBeenCalled();
    hook.rerender({ready:true,incoming:[{account_id:"work",continuation:"fresh"}]});
    await waitFor(()=>expect(vi.mocked(api.search).mock.calls.map(call=>call[2])).toEqual(["fresh"]));
    hook.unmount();client.clear();
  });

  it("does not reuse completed provider heads after leaving and returning to a query",async()=>{
    vi.mocked(api.search).mockImplementation(async(q)=>{const result=page("work");result.rows=[{...messages[0],id:q}];return result;});
    const {wrapper,client}=mount();
    const hook=renderHook(({q})=>useSearchSupplements(q,["work"],undefined,[roots[0]]),{wrapper,initialProps:{q:"parcel"}});
    await waitFor(()=>expect(hook.result.current.pages[0]?.rows[0]?.id).toBe("parcel"));
    hook.rerender({q:"invoice"});
    await waitFor(()=>expect(hook.result.current.pages[0]?.rows[0]?.id).toBe("invoice"));
    hook.rerender({q:"parcel"});
    await waitFor(()=>expect(vi.mocked(api.search).mock.calls.filter(call=>call[0]==="parcel")).toHaveLength(2));
    hook.unmount();client.clear();
  });
});

it("keeps mailbox cache metadata out of provider category filters", async () => {
  vi.mocked(api.search).mockResolvedValue(page("work"));
  const { wrapper, client } = mount();
  const hook = renderHook(() => useSearchSupplements("hotel in Lisbon", ["work"], undefined, [roots[0]], true, JSON.stringify({ revision: "personal-first" })), { wrapper });
  await waitFor(() => expect(api.search).toHaveBeenCalledTimes(1));
  expect(vi.mocked(api.search).mock.calls[0][3]).toBeUndefined();
  hook.unmount(); client.clear();
});

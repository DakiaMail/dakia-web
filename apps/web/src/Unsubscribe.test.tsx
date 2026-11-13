// @vitest-environment jsdom
import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Unsubscribe } from "./Unsubscribe";
import { messages } from "../browser/fixtures";

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });
for (const action of [
  { kind: "mailto", to: "listunsubscribe-synthetic@email.example.org", subject: "Leave+now", body: "Stop\r\nplease" },
  { kind: "one_click", url: "https://example.org/leave" },
]) it(`one reader click sends ${action.kind} without review or composer, never on read`, async () => {
  const writes: unknown[] = [];
  let finish!: () => void;
  const fetch = vi.fn(async (_url: string, init?: RequestInit) => {
    if (init?.method === "POST") {
      writes.push(JSON.parse(init.body as string));
      await new Promise<void>(resolve => { finish = resolve; });
      return new Response(null, { status: 204 });
    }
    return new Response(JSON.stringify(action));
  });
  vi.stubGlobal("fetch", fetch);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const draft = vi.fn();
  render(<QueryClientProvider client={client}><Unsubscribe message={messages[0]} onDraft={draft} /></QueryClientProvider>);
  const button = await screen.findByRole("button", { name: "Unsubscribe" });
  expect(writes).toHaveLength(0);
  fireEvent.click(button); fireEvent.click(button);
  await waitFor(() => expect(writes).toHaveLength(1));
  expect(screen.queryByText(/Review email|Confirm unsubscribe|Nothing is sent automatically/)).toBeNull();
  expect(screen.queryByRole("dialog")).toBeNull();
  finish();
  await screen.findByRole("heading", { name: "Unsubscribe request sent" });
  expect(draft).not.toHaveBeenCalled();
  expect(writes[0]).toEqual({ expected_action: action });
  client.clear();
});

it("a newsletter row loads unsubscribe metadata only after a click and sends once",async()=>{
 const action={kind:"one_click",url:"https://example.org/leave"};const writes:unknown[]=[];
 const fetch=vi.fn(async(_url:string,init?:RequestInit)=>{if(init?.method==="POST"){writes.push(JSON.parse(init.body as string));return new Response(null,{status:204});}return new Response(JSON.stringify(action));});
 vi.stubGlobal("fetch",fetch);const client=new QueryClient({defaultOptions:{queries:{retry:false}}});
 render(<QueryClientProvider client={client}><Unsubscribe message={messages[0]} lazy/></QueryClientProvider>);
 const button=screen.getByRole("button",{name:"Unsubscribe"});expect(fetch).not.toHaveBeenCalled();
 fireEvent.click(button);fireEvent.click(button);await screen.findByRole("heading",{name:"Unsubscribe request sent"});
 expect(writes).toEqual([{expected_action:action}]);client.clear();
});
it("web unsubscribe remains a domain-labelled link and never sends automatically",async()=>{
 const fetch=vi.fn(async(_url:string,_init?:RequestInit)=>new Response(JSON.stringify({kind:"web",url:"https://example.org/leave"})));
 vi.stubGlobal("fetch",fetch);const client=new QueryClient({defaultOptions:{queries:{retry:false}}});
 render(<QueryClientProvider client={client}><Unsubscribe message={messages[0]} lazy/></QueryClientProvider>);
 fireEvent.click(screen.getByRole("button",{name:"Unsubscribe"}));
 const link=await screen.findByRole("link",{name:"Unsubscribe example.org"});expect(link.getAttribute("href")).toBe("https://example.org/leave");
 expect(fetch.mock.calls.every(args=>!(args[1] as RequestInit|undefined)?.method)).toBe(true);client.clear();
});

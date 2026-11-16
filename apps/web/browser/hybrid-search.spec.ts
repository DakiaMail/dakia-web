import { test, expect } from "@playwright/test";
import { accounts, messages, page as original } from "./fixtures";
import { mkdirSync } from "node:fs";
for (const theme of ["light","dark"]) for (const viewport of [{width:1480,height:820},{width:390,height:568}]) {
  test(`hybrid search keyboard ${theme} ${viewport.width}`,async({page})=>{
    await page.setViewportSize(viewport);
    const requestOrder:string[]=[];
    let refreshed=false;
    let mailArrived=false;
    let releaseArrival!:()=>void;
    const heldArrival=new Promise<void>(resolve=>{releaseArrival=resolve;});
    let releaseWork!:()=>void;
    let releaseLocal!:()=>void;
    const heldLocal=new Promise<void>(resolve=>{releaseLocal=resolve;});
    const heldWork=new Promise<void>(resolve=>{releaseWork=resolve;});
    const ranked={...messages[0],id:"ranked",account_id:"work",subject:"Older exact delivery match",preview:"Original preview",received_at:"2020-01-01T00:00:00Z"};
    const attachment={...messages[2],id:"attachment",account_id:"work",subject:"Attachment shipment detail",received_at:"2019-01-01T00:00:00Z"};
    const provider={...messages[1],id:"provider",account_id:"personal",subject:"Newer provider result",received_at:"2026-11-01T00:00:00Z"};
    const result=(rows:typeof messages,continuation:string|null=null)=>({...original,rows,continuation,pending:!!continuation});
    await page.route("**/api/**",async route=>{
      const url=new URL(route.request().url());const path=url.pathname;
      if(path==="/api/session")return route.fulfill({json:{user_id:"hybrid-fixture",csrf_token:"synthetic"}});
      if(path==="/api/accounts")return route.fulfill({json:accounts});
      if(path.endsWith("/events"))return route.fulfill({contentType:"text/event-stream",body:": ready\n\n"});
      if(path.endsWith("/sync-active"))return route.fulfill({json:false});
      if(path.endsWith("/indexing"))return route.fulfill({json:{account_id:path.split("/")[3],state:"complete",discovery_complete:true,eligible:2,processed:2,backlog:0,gaps:0}});
      if(path==="/api/search"){
        if(url.searchParams.has("suggest"))return route.fulfill({json:result([])});
        const q=url.searchParams.get("q")||"";const token=url.searchParams.get("continuation");
        if(q==="parcel"){
          requestOrder.push(token||"initial");
          if(token==="work-mail-provider"){
            await heldArrival;
            return route.fulfill({json:result([{...ranked,id:"mail-provider",subject:"A parcel update just arrived"}])});
          }
          if(token==="work-provider"){
            if(refreshed)return route.fulfill({json:result([{...ranked,id:"fresh-provider",subject:"Arrived since the previous search"}])});
            await heldWork;return route.fulfill({json:result([ranked])});
          }
          if(token==="personal-provider")return route.fulfill({json:{...result([provider]),search:{ranked:false,matches:[],degradation:[{account_id:"personal",code:"provider_literal_only"}]}}});
          if(token==="local-next"){await heldLocal;return route.fulfill({json:{...result([attachment]),search:{ranked:true,matches:[{account_id:"work",message_id:attachment.id,score:0.02,excerpt:"The delivery address appears in the attachment.",attachment:"report.pdf"}],degradation:[]}}});}
          return route.fulfill({json:{...result([ranked],"local-next"),pending:true,cache_preview:true,index_page:true,search:{ranked:true,matches:[{account_id:"work",message_id:ranked.id,score:0.03,excerpt:"Your parcel will arrive tomorrow.",attachment:null}],degradation:[{account_id:"work",code:"semantic_search_unavailable"}]},supplement:[{account_id:"work",continuation:mailArrived?"work-mail-provider":"work-provider"},{account_id:"personal",continuation:"personal-provider"}]}});
        }
        return route.fulfill({json:original});
      }
      if(path.endsWith("/messages"))return route.fulfill({json:original});
      return route.fulfill({json:[]});
    });
    // A controlled stream keeps this rendered journey deterministic. Each
    // account still receives its own production event handler and debounce.
    await page.addInitScript(()=>{
      class FixtureEvents extends EventTarget {
        onmessage:((event:MessageEvent)=>void)|null=null;
        onerror:(()=>void)|null=null;
        private receive=(event:Event)=>{
          const detail=(event as CustomEvent).detail;
          if(this.url.includes(`/accounts/${detail.account_id}/`)) this.onmessage?.(new MessageEvent("message",{data:JSON.stringify(detail)}));
        };
        constructor(readonly url:string){super();window.addEventListener("fixture-mail-event",this.receive);}
        close(){window.removeEventListener("fixture-mail-event",this.receive);}
      }
      window.EventSource=FixtureEvents as unknown as typeof EventSource;
    });
    await page.addInitScript(value=>localStorage.setItem("dakia-theme",value),theme);
    await page.goto("/");
    const input=page.getByRole("combobox",{name:"Search all mail"});
    await input.focus();await input.dispatchEvent("compositionstart");
    await page.keyboard.type("parcel");await page.keyboard.press("Enter");
    await expect(page.getByRole("heading",{name:"Inbox",exact:true})).toBeVisible();
    expect(requestOrder).not.toContain("initial");
    await input.dispatchEvent("compositionend");await page.keyboard.press("Enter");
    await expect(page.locator('[data-message-id="work/ranked"]')).toBeVisible();
    await expect(input).toBeFocused();
    await expect(page.locator('[data-message-id="personal/provider"]')).toBeVisible();
    expect(requestOrder).toContain("work-provider");expect(requestOrder).toContain("personal-provider");
    await expect(page.getByText("Your parcel will arrive tomorrow.",{exact:true})).toBeVisible();
    await expect(page.getByText("Some related matches and attachment text may be missing.",{exact:true})).toHaveCount(0);
    // Prefetching may warm one local page, but provider work never waits for it
    // to be appended. A slow provider also cannot disable local keyboard paging.
    expect(await page.locator("[data-message-id]").evaluateAll(rows=>rows.map(row=>row.getAttribute("data-message-id")))).toEqual(["work/ranked","personal/provider"]);
    // The local prefetch is still pending, while the independent Personal
    // provider response is already visible. Release the next ranked page.
    releaseLocal();
    await expect(page.locator('[data-message-id="work/attachment"]')).toBeVisible();
    expect(await page.locator("[data-message-id]").evaluateAll(rows=>rows.map(row=>row.getAttribute("data-message-id")))).toEqual(["work/ranked","work/attachment","personal/provider"]);
    releaseWork();
    await expect(page.locator('[data-message-id="work/ranked"]')).toHaveCount(1);
    await expect(input).toBeFocused();
    await page.keyboard.press("Tab");
    await expect(page.getByRole("button",{name:"Clear search text",exact:true})).toBeFocused();
    await page.keyboard.press("Tab");
    await expect(page.getByRole("button",{name:"Search filters",exact:true})).toBeFocused();
    await page.getByRole("region",{name:"Messages",exact:true}).focus();
    await page.keyboard.press("Tab");
    const checkbox=page.getByRole("checkbox",{name:"Select Older exact delivery match",exact:true});
    await expect(checkbox).toBeFocused();
    await page.keyboard.press("Space");await expect(checkbox).toBeChecked();
    await page.keyboard.press("Space");await expect(checkbox).not.toBeChecked();
    await page.keyboard.press("Shift+Tab");
    await page.keyboard.press("/");
    await expect(input).toBeFocused();
    refreshed=true;
    await page.keyboard.press("Enter");
    await expect(page.locator('[data-message-id="work/fresh-provider"]')).toBeVisible();
    await expect(input).toBeFocused();
    expect(requestOrder.filter(token=>token==="work-provider")).toHaveLength(2);
    expect(requestOrder.filter(token=>token==="personal-provider")).toHaveLength(2);

    mailArrived=true;
    await page.evaluate(()=>window.dispatchEvent(new CustomEvent("fixture-mail-event",{detail:{account_id:"work",cursor:1,kind:"message_changed",target_id:"new-mail"}})));
    await expect.poll(()=>requestOrder.filter(token=>token==="work-mail-provider").length).toBe(1);
    // Keep the completed provider rows while the new head is in flight.
    await expect(page.locator('[data-message-id="work/fresh-provider"]')).toBeVisible();
    await expect(input).toBeFocused();
    releaseArrival();
    await expect(page.locator('[data-message-id="work/mail-provider"]')).toBeVisible();
    await expect(page.locator('[data-message-id="work/fresh-provider"]')).toHaveCount(0);
    await expect(page.locator('[data-message-id="personal/provider"]')).toBeVisible();
    expect(requestOrder.filter(token=>token==="personal-provider")).toHaveLength(2);
    expect(requestOrder.filter(token=>token==="work-provider")).toHaveLength(2);
    await expect(input).toBeFocused();

    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    mkdirSync("artifacts/hybrid",{recursive:true});
    await page.screenshot({path:`artifacts/hybrid/${theme}-${viewport.width}.png`});
    await page.reload();
    const restoredInput=page.getByRole("combobox",{name:"Search all mail"});
    await restoredInput.focus();await page.keyboard.type("parcel");await page.keyboard.press("Enter");
    await expect(page.getByRole("heading",{name:"Search",exact:true})).toBeVisible();
    await expect(page.locator('[data-message-id="work/ranked"]')).toBeVisible();
  });
}

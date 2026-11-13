import { test, expect, type Page } from "@playwright/test";
import type { Message, Operation } from "@dakia/api-types";
import { accounts, content, page as mail } from "./fixtures";
import { classification, matchesClassification } from "../src/classification";
import { optimistic } from "../src/api";
const make=(id:string,automated=false):Message=>({...mail.rows[0],id,provider_id:id,thread_id:id,subject:id,unread:false,classification:{...classification(mail.rows[0]),status:"ready",kind:automated?"automated":"personal",types:automated?["flight_booking","receipt"]:[],policy_revision:"personal-first-1"}});
async function fixture(page:Page) {
 let rows=[make("Personal read"),make("Personal new"),...Array.from({length:65},(_,i)=>make(`Older ${i}`)),make("Flight",true)];rows[1].unread=true;
 const filters:string[]=[];
 await page.route("**/api/**",async route=>{
  const url=new URL(route.request().url()),path=url.pathname;
  const reply=(v:unknown)=>route.fulfill({contentType:"application/json",body:JSON.stringify(v)});
  if(path==="/api/session")return reply({user_id:"personal-fixture",csrf_token:"fixture"});
  if(path==="/api/accounts")return reply(accounts);
  if(path.endsWith("/events"))return route.fulfill({contentType:"text/event-stream",body:": ready\n\n"});
  if(path.endsWith("/operations")){const op:Operation=route.request().postDataJSON();rows=rows.map(m=>m.account_id===op.account_id&&m.id===op.target_id?{...optimistic(m,op.change),revision:m.revision+1}:m);return reply({...op,status:"applied",revision:2});}
  if(path==="/api/search"){
   const f={inbox_kind:url.searchParams.get("inbox_kind")||undefined,email_type:url.searchParams.get("email_type")||undefined,attention:url.searchParams.get("attention")==="true",transactions:url.searchParams.get("transactions")==="true"};filters.push(url.search);
   let matches=rows.filter(m=>matchesClassification(m,f as Parameters<typeof matchesClassification>[1]));
   const offset=Number(url.searchParams.get("continuation")||0);return reply({...mail,rows:matches.slice(offset,offset+50),continuation:matches.length>offset+50?String(offset+50):null});
  }
  const m=rows.find(m=>path.includes(`/messages/${encodeURIComponent(m.id)}`))||rows[0];
  if(path.endsWith("/conversation"))return reply({rows:[m],continuation:null,complete:true});
  if(path.includes("/messages/"))return reply({...content,message:m});
  return reply([]);
 });
 await page.goto("/");await expect(page.locator(".message-row").first()).toBeVisible();return filters;
}
for(const width of [390,1480])test(`Personal includes read mail and Automated filters older overlapping types at ${width}`,async({page})=>{
 await page.setViewportSize({width,height:900});const filters=await fixture(page);
 await expect(page.getByRole("checkbox",{name:"Select Personal read",exact:true})).toBeVisible();
 await page.getByTitle("Open Personal new",{exact:true}).click();await page.keyboard.press("Escape");
 await expect(page.getByRole("checkbox",{name:"Select Personal new",exact:true})).toBeVisible();
 await page.getByRole("tab",{name:"Automated",exact:true}).click();
 await page.getByRole("button",{name:"Flights",exact:true}).click();await expect(page.locator(".message-row")).toHaveCount(1);
 expect(filters.some(f=>f.includes("inbox_kind=automated")&&f.includes("email_type=flight_booking"))).toBe(true);
 await page.getByRole("button",{name:"Transactions",exact:true}).click();await expect(page.getByRole("checkbox",{name:"Select Flight",exact:true})).toBeVisible();
 await page.reload();await expect(page.getByRole("tab",{name:"Automated",exact:true})).toHaveAttribute("aria-selected","true");
 await expect(page.locator(".group-heading")).toHaveCount(0);
});

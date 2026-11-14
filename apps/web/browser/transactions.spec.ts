import {test,expect} from '@playwright/test';
import type {Message,Operation} from '@dakia/api-types';
import {accounts,messages,page as mail} from './fixtures';
import {classification,matchesClassification,type MailFilter} from '../src/classification';
for(const width of [390,1480])test(`Transactions combines both types across pages with category icons ${width}`,async({page},info)=>{
 const rows:Message[]=Array.from({length:55},(_,i)=>({...messages[0],id:`transaction-${i}`,subject:`Transaction ${i}`,thread_id:`transaction-${i}`,unread:false,classification:{...classification(messages[0]),kind:'automated',status:'ready',types:i===0?['receipt','invoice']:i%2?['invoice']:['receipt']}}));
 rows.push({...rows[0],id:'order-only',subject:'Order only',classification:{...rows[0].classification!,types:['order']}});
 const operations:Operation[]=[],queries:URL[]=[];
 await page.setViewportSize({width,height:980});
 await page.route('**/api/**',async route=>{
  const url=new URL(route.request().url()),path=url.pathname;const respond=(json:unknown)=>route.fulfill({json});
  if(path==='/api/session')return respond({user_id:'transactions-fixture',csrf_token:'fixture'});
  if(path==='/api/accounts')return respond([accounts[0]]);
  if(path.endsWith('/events'))return route.fulfill({contentType:'text/event-stream',body:': ready\n\n'});
  if(path.endsWith('/operations')){const op={...route.request().postDataJSON(),status:'applied',revision:2};operations.push(op);return respond(op);}
  if(path==='/api/search'||path.endsWith('/messages')){
   queries.push(url);const filter:MailFilter={inbox_kind:url.searchParams.get('inbox_kind') as MailFilter['inbox_kind']||undefined,transactions:url.searchParams.get('transactions')==='true',attention:url.searchParams.get('attention')==='true'};
   const selected=rows.filter(m=>matchesClassification(m,filter)),offset=url.searchParams.has('continuation')?50:0;
   return respond({...mail,rows:selected.slice(offset,offset+50),continuation:selected.length>offset+50?'tail':null,pending:selected.length>offset+50});
  }
  return respond([]);
 });
 await page.goto('/');await page.getByRole('tab',{name:'Automated',exact:true}).click();
 const categories=page.getByRole('group',{name:'Filter automated email',exact:true});
 await categories.getByRole('button',{name:'Transactions',exact:true}).click();
 await expect(categories.getByRole('button',{name:'Receipts',exact:true})).toHaveCount(0);await expect(categories.getByRole('button',{name:'Invoices',exact:true})).toHaveCount(0);
 expect(await categories.locator('button').count()).toBe(10);expect(await categories.locator('button svg[aria-hidden="true"]').count()).toBe(10);
 await page.getByRole('checkbox',{name:'Select Transaction 0',exact:true}).check();await page.getByRole('checkbox',{name:'Select all',exact:true}).check();
 await page.getByRole('button',{name:'Select all messages matching this filter',exact:true}).click();await expect(page.getByRole('status').filter({hasText:'All 55 matching messages are selected.'})).toBeVisible();
 await page.getByRole('button',{name:'Mark unread',exact:true}).click();await expect.poll(()=>operations.length).toBe(55);
 expect(new Set(operations.map(o=>o.target_id)).size).toBe(55);expect(operations.some(o=>o.target_id==='order-only')).toBe(false);
 expect(queries.some(q=>q.searchParams.get('transactions')==='true'&&q.searchParams.has('continuation'))).toBe(true);
 expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
 await page.screenshot({path:info.outputPath(`transactions-${width}.png`),animations:'disabled'});
});

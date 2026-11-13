import {test,expect,type Page} from '@playwright/test';
import type {Message,Operation} from '@dakia/api-types';
import {accounts,messages,page as mail,content} from './fixtures';
async function fixture(page:Page){
 const rows:Message[]=Array.from({length:53},(_,i)=>({...messages[0],id:`attention-${i}`,thread_id:`attention-${i}`,subject:`Attention ${i}`,unread:false,classification:{kind:i%2?'personal':'automated',status:'ready',types:['event_invitation'],details:[],attention:[{label:'Event cancelled',expires_at:null}],attention_override:null,source:'automatic',policy_revision:'personal-first-2'}}));
 const code:Message={...rows[0],id:'code',subject:'Your verification code is 937449',classification:{...rows[0].classification!,types:['verification_code'],details:[{email_type:'verification_code',label:'Code',value:'937449',action:'copy'}],attention:[{label:'Verification code',expires_at:'2000-01-01T00:00:00Z'}]}};
 const newsletter:Message={...code,id:'newsletter',subject:'Weekly update',classification:{...code.classification!,types:['newsletter'],details:[],attention:[]}};
 const receipt:Message={...code,id:'receipt',subject:'Purchase receipt',classification:{...code.classification!,types:['receipt'],details:[{email_type:'receipt',label:'Total',value:'12,50 €',action:null}],attention:[]}};
 const all=[...rows,code,newsletter,receipt],operations:Operation[]=[];let unsubscribeReads=0,unsubscribePosts=0;
 await page.addInitScript(()=>{Object.defineProperty(navigator,'clipboard',{value:{writeText:async(value:string)=>{(window as any).copied=value;}}});});
 await page.route('**/api/**',async route=>{
  const url=new URL(route.request().url()),p=url.pathname;const respond=(json:unknown)=>route.fulfill({json});
  if(p==='/api/session')return respond({user_id:'details-review',csrf_token:'fixture'});
  if(p==='/api/accounts')return respond([accounts[0]]);
  if(p.endsWith('/events'))return route.fulfill({contentType:'text/event-stream',body:': ready\n\n'});
  if(p.endsWith('/unsubscribe')){if(route.request().method()==='POST'){unsubscribePosts++;return route.fulfill({status:204});}unsubscribeReads++;return respond({kind:'one_click',url:'https://example.org/unsubscribe'});}
  if(p.endsWith('/operations')){const op={...route.request().postDataJSON(),status:'applied',revision:2};operations.push(op);if(op.change.kind==='attention')all.find(m=>m.id===op.target_id)!.classification!.attention_override=op.change.value;return respond(op);}
  if(p==='/api/search'||p.endsWith('/messages')){
   const attention=url.searchParams.has('attention');if(attention){expect(url.searchParams.get('q')).toBe('folder:INBOX');expect(url.searchParams.has('inbox_kind')).toBe(false);}
   let found=all.filter(m=>(!attention||(m.id.startsWith('attention-')&&m.classification!.attention_override!==false))&&(!url.searchParams.has('inbox_kind')||m.classification!.kind===url.searchParams.get('inbox_kind'))&&(!url.searchParams.has('transactions')||m.classification!.types.some(t=>t==='receipt'||t==='invoice'))&&(!url.searchParams.has('email_type')||m.classification!.types.includes(url.searchParams.get('email_type') as any)));
   const tail=url.searchParams.has('continuation');return respond({...mail,rows:tail?found.slice(50):found.slice(0,50),continuation:!tail&&found.length>50?'tail':null,pending:!tail&&found.length>50});
  }
  if(p.endsWith('/conversation'))return respond({rows:[code],continuation:null,complete:true});
  if(p.includes('/messages/'))return respond({...content,message:code});
  return respond([]);
 });await page.goto('/');
 return {operations,unsubscribeReads:()=>unsubscribeReads,unsubscribePosts:()=>unsubscribePosts};
}
for(const width of [390,1480])test(`attention page and row shortcuts ${width}`,async({page},info)=>{
 await page.setViewportSize({width,height:980});const state=await fixture(page);
 const strip=page.getByRole('region',{name:'Needs attention',exact:true});await expect(strip.locator('.attention-item')).toHaveCount(3);
 await strip.getByRole('button',{name:'Show more',exact:true}).click();
 await expect(page.getByRole('heading',{name:'Needs attention',exact:true})).toBeVisible();
 await expect(page.locator('.attention-strip')).toHaveCount(0);await expect(page.getByRole('tablist')).toHaveCount(0);
 await page.getByRole('checkbox',{name:'Select Attention 0',exact:true}).check();await page.getByRole('checkbox',{name:'Select all',exact:true}).check();
 await page.getByRole('button',{name:'Select all messages matching this filter',exact:true}).click();await expect(page.getByRole('status').filter({hasText:'All 53 matching messages are selected.'})).toBeVisible();
 await page.getByRole('button',{name:'Mark unread',exact:true}).click();await expect.poll(()=>state.operations.length).toBe(53);
 if(width<700)await page.getByRole('button',{name:'Open navigation',exact:true}).click();
 await page.getByRole('navigation',{name:'Mailboxes',exact:true}).getByRole('button',{name:/Unified inbox/}).click();
 await page.getByRole('tab',{name:'Automated',exact:true}).click();await page.getByRole('button',{name:'Codes',exact:true}).click();
 await expect(page.locator('.detected-types')).toHaveCount(0);await page.getByRole('button',{name:'Code: 937449',exact:true}).click();
 expect(await page.evaluate(()=>(window as any).copied)).toBe('937449');await expect(page.locator('.conversation-title')).toHaveCount(0);
 await page.getByRole('button',{name:'Transactions',exact:true}).click();await expect(page.getByText('Total: 12,50 €',{exact:true})).toBeVisible();
 await page.getByRole('button',{name:'Newsletters',exact:true}).click();await expect(page.getByRole('button',{name:'Unsubscribe',exact:true})).toBeVisible();expect(state.unsubscribeReads()).toBe(0);
 await page.screenshot({path:info.outputPath(`newsletter-row-${width}.png`),animations:'disabled'});
 await page.getByRole('button',{name:'Unsubscribe',exact:true}).click();await expect(page.getByRole('heading',{name:'Unsubscribe request sent',exact:true})).toBeVisible();expect(state.unsubscribePosts()).toBe(1);
});

test('attention ages out after 30 days without hiding ordinary mail or its code',async({page})=>{
 await page.clock.install({time:new Date('2026-11-13T12:00:00Z')});await fixture(page);
 await page.getByRole('region',{name:'Needs attention',exact:true}).getByRole('button',{name:'Show more',exact:true}).click();
 await expect(page.locator('.message-row').first()).toBeVisible();
 await page.clock.setSystemTime(new Date('2026-12-14T12:00:00Z'));await page.clock.runFor(15001);
 await expect(page.locator('.message-row')).toHaveCount(0);
 await page.getByRole('navigation',{name:'Mailboxes',exact:true}).getByRole('button',{name:/Unified inbox/}).click();
 await expect(page.locator('.attention-strip')).toHaveCount(0);
 await page.getByRole('tab',{name:'Automated',exact:true}).click();await page.getByRole('button',{name:'Codes',exact:true}).click();
 await expect(page.getByRole('button',{name:'Code: 937449',exact:true})).toBeVisible();
});

for(const [width,dark,reduced] of [[390,true,true],[1480,true,false],[1480,false,true]] as const)test(`shared copy button keyboard and motion ${width} ${dark} ${reduced}`,async({page},info)=>{
 await page.setViewportSize({width,height:980});await page.emulateMedia({reducedMotion:reduced?'reduce':'no-preference'});
 if(dark)await page.addInitScript(()=>localStorage.setItem('dakia-theme','dark'));
 await fixture(page);await page.getByRole('tab',{name:'Automated',exact:true}).click();await page.getByRole('button',{name:'Codes',exact:true}).click();
 await page.evaluate(()=>document.fonts.ready);
 const button=page.getByRole('button',{name:'Code: 937449',exact:true});const before=await button.boundingBox();await button.focus();await page.keyboard.press('Enter');
 await expect(button).toHaveAttribute('data-copy-state','copied');await expect(button).toBeFocused();await expect(button).toHaveAttribute('title','Copied');
 expect(await page.evaluate(()=>(window as any).copied)).toBe('937449');await expect(page.locator('.conversation-title')).toHaveCount(0);
 const after=await button.boundingBox();expect(after!.width).toBe(before!.width);expect(after!.height).toBe(before!.height);
 const duration=await button.locator('[data-icon="copied"]').evaluate(el=>getComputedStyle(el).transitionDuration);
 expect(duration.split(',').every(value=>parseFloat(value)===0)).toBe(reduced);
 const row=button.locator('xpath=ancestor::*[contains(@class,"message-row")]');const rowBox=await row.boundingBox();expect(after!.y+after!.height).toBeLessThanOrEqual(rowBox!.y+rowBox!.height+1);
 await page.screenshot({path:info.outputPath(`copied-${width}-${dark}-${reduced}.png`),animations:'disabled'});
 await expect(button).toHaveAttribute('data-copy-state','idle');await expect(button).toBeFocused();
});

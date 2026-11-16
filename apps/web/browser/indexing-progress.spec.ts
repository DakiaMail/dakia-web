import { test, expect } from "@playwright/test";
import { accounts, page as mail } from "./fixtures";
import { mkdirSync } from "node:fs";

for (const theme of ["light", "dark"]) for (const viewport of [{width:1480,height:820},{width:390,height:568}]) {
  test(`indexing progress ${theme} ${viewport.width}`, async ({page}) => {
    test.setTimeout(45000);
    await page.setViewportSize(viewport);
    await page.clock.install();
    const reducedMotion=theme==="dark" && viewport.width<600;
    if(reducedMotion) await page.emulateMedia({reducedMotion:"reduce"});
    let state="discovering";
    let finishing=false;
    await page.route("**/api/**", route=> {
      const path=new URL(route.request().url()).pathname;
      if(path==="/api/session")return route.fulfill({json:{user_id:"indexing-fixture",csrf_token:"synthetic"}});
      if(path==="/api/accounts")return route.fulfill({json:accounts.map(account=>account.id==="work"?{...account,state:"reconnect"}:account)});
      if(path.endsWith("/events"))return route.fulfill({contentType:"text/event-stream",body:": ready\n\n"});
      if(path.endsWith("/indexing"))return route.fulfill({json:{account_id:path.split("/")[3],state,discovery_complete:state!=="discovering",eligible:100,processed:state==="complete"||finishing?100:37,backlog:state==="complete"?0:finishing?1:63,gaps:state==="complete"?2:0}});
      if(path.endsWith("/sync-active"))return route.fulfill({json:false});
      if(path==="/api/search" || path.endsWith("/messages"))return route.fulfill({json:mail});
      return route.fulfill({json:[]});
    });
    await page.addInitScript(value=>localStorage.setItem("dakia-theme",value),theme);
    await page.goto("/");
    if(viewport.width<600) { await page.getByRole("button",{name:"Open navigation"}).focus();await page.keyboard.press("Enter"); }
    const work=page.getByRole("navigation",{name:"Accounts"}).getByRole("button",{name:/Work/});
    await work.focus();
    const bar=page.getByRole("progressbar",{name:"Updating Work"});
    await expect(bar).toBeVisible();
    await expect(bar).not.toHaveAttribute("aria-valuenow");
    if(reducedMotion) expect(await bar.locator("span").evaluate(element=>getComputedStyle(element).animationName)).toBe("none");
    await expect(work).toBeFocused();
    await expect(page.getByRole("tooltip")).toHaveCount(0);
    await expect(work).not.toHaveAttribute("title");
    await expect(page.getByText(/emails indexed|unreadable content/)).toHaveCount(0);
    state="indexing";
    await page.clock.fastForward(5100);
    await expect(bar).toHaveAttribute("aria-valuenow","37",{timeout:7000});
    await expect(work).toBeFocused();
    await expect(bar).toHaveAttribute("aria-valuetext", "Updating mail");
    mkdirSync("artifacts/indexing",{recursive:true});
    await page.mouse.move(viewport.width-1,0);
    await page.keyboard.press("Tab");
    await expect(page.getByRole("navigation",{name:"Accounts"}).getByRole("button",{name:/Personal/})).toBeFocused();
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    mkdirSync("artifacts/indexing",{recursive:true});
    await page.screenshot({path:`artifacts/indexing/${theme}-${viewport.width}.png`});
    state="paused";
    await page.clock.fastForward(5100);
    await expect(bar).toHaveAttribute("aria-valuetext","Updating paused",{timeout:7000});
    await page.reload();
    if(viewport.width<600) { await page.getByRole("button",{name:"Open navigation"}).focus();await page.keyboard.press("Enter"); }
    await expect(bar).toHaveAttribute("aria-valuetext","Updating paused");
    state="error";
    await page.clock.fastForward(5100);
    await expect(bar).toHaveAttribute("aria-valuetext","Updating paused",{timeout:7000});
    state="indexing";
    await page.clock.fastForward(5100);
    finishing=true;
    await page.clock.fastForward(5100);
    await expect(bar).toHaveAttribute("aria-valuetext","Updating mail",{timeout:7000});
    await expect(bar).toHaveAttribute("aria-valuenow","100");
    await page.screenshot({path:`artifacts/indexing/finishing-${theme}-${viewport.width}.png`});
    await work.focus();
    const before=(await work.locator("..").boundingBox())!.height;
    state="complete";
    await page.clock.fastForward(5100);
    await expect(bar).toHaveCount(0,{timeout:7000});
    await expect(work).toBeFocused();
    expect((await work.locator("..").boundingBox())!.height).toBeLessThan(before);
    await expect(page.getByRole("tooltip")).toHaveCount(0);
    await expect(page.getByText(/emails indexed|unreadable content/)).toHaveCount(0);
  });
}

// Real Rust HTTP, durable corrections, MIME detection, and SQLite; only mail transport is synthetic.
import { test, expect, type Page } from "@playwright/test";
const row = (page: Page, n: number) => page.locator(".message-row").filter({ has: page.locator(".subject", { hasText: new RegExp(`^Synthetic ${n}$`) }) });
async function open(page: Page, n: number) { await row(page,n).locator(".message-open").focus(); await page.keyboard.press("Enter"); await expect(page.locator(".conversation-title h2")).toHaveText(`Synthetic ${n}`); }
async function menu(page: Page, submenu: string, item: string, checked = false) {
  const trigger = page.getByRole("button",{name:"More actions",exact:true}); await trigger.focus(); await page.keyboard.press("Enter");
  await page.getByRole("menuitem",{name:submenu,exact:true}).focus(); await page.keyboard.press("ArrowRight");
  await page.getByRole(checked ? "menuitemcheckbox" : item === "Personal" || item === "Automated" ? "menuitemradio" : "menuitem",{name:item,exact:true}).focus(); await page.keyboard.press("Enter");
  await expect(trigger).toBeFocused();
}
test.beforeEach(async({context})=>{await context.addCookies([{name:"__Host-dakia",value:"release-browser-fixture",url:"https://127.0.0.1:4327/",secure:true,sameSite:"Lax"}]);});
test("personal-first inbox, independent attention, and durable keyboard corrections",async({page},info)=>{
  test.setTimeout(120000);
  await page.goto("/");
  await expect(page.getByRole("tab",{name:"Personal",exact:true})).toHaveAttribute("aria-selected","true");
  await expect(row(page,0)).toBeVisible(); await expect(row(page,2)).toHaveCount(0);
  const attention=page.getByRole("region",{name:"Needs attention"});
  await expect(attention.getByText("Synthetic 3",{exact:true})).toBeVisible(); await expect(attention.getByText("Synthetic 4",{exact:true})).toBeVisible();
  await open(page,0);
  await page.getByRole("button",{name:"More actions",exact:true}).focus();await page.keyboard.press("Enter");
  await page.getByRole("menuitem",{name:"Needs attention",exact:true}).focus();await page.keyboard.press("Enter");
  await page.keyboard.press("Escape");await expect(row(page,0)).toBeVisible();await expect(attention.getByText("Synthetic 0",{exact:true})).toBeVisible();
  await attention.getByRole("button",{name:"Dismiss attention for Synthetic 0",exact:true}).focus();await page.keyboard.press("Enter");
  await expect(attention.getByText("Synthetic 0",{exact:true})).toHaveCount(0);

  await page.getByRole("tab",{name:"Personal",exact:true}).focus(); await page.keyboard.press("ArrowRight");
  await expect(page.getByRole("tab",{name:"Automated",exact:true})).toHaveAttribute("aria-selected","true");
  await page.getByRole("button",{name:"Receipts",exact:true}).focus(); await page.keyboard.press("Enter");
  await expect(row(page,2)).toBeVisible(); await expect(page.locator(".message-row")).toHaveCount(1);
  await page.getByRole("button",{name:"Orders",exact:true}).click(); await expect(row(page,2)).toBeVisible();
  await page.getByRole("button",{name:"Flights",exact:true}).click(); await expect(row(page,3)).toBeVisible();
  await expect(row(page,3).getByText("Reference: ABC123",{exact:true})).toBeVisible();
  await page.getByRole("button",{name:"All automated",exact:true}).click(); await expect(row(page,7)).toBeVisible();
  const typeLabel = await row(page,2).locator('.detected-types').boundingBox();
  await page.mouse.click(typeLabel!.x + typeLabel!.width / 2, typeLabel!.y + typeLabel!.height / 2);
  await expect(page.locator('.conversation-title h2')).toHaveText('Synthetic 2');
  await page.keyboard.press('Escape');
  await open(page,4);await page.keyboard.press("Escape");await expect(row(page,4)).toBeVisible();await expect(attention.getByText("Synthetic 4",{exact:true})).toBeVisible();

  for (const [width,height] of [[1480,980],[390,740],[390,480]]) for (const theme of ["light","dark"]) {
    await page.setViewportSize({width:1480,height:980});
    const themeButton=page.getByRole("button",{name:theme === "dark" ? "Dark theme" : "Light theme",exact:true});
    if(await themeButton.isVisible()) await themeButton.click();
    await expect(page.locator("html")).toHaveAttribute("data-theme",theme);
    await page.setViewportSize({width,height});
    await page.screenshot({path:info.outputPath(`inbox-${width}-${height}-${theme}.png`),animations:"disabled"});
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await open(page,2);
    for (const name of ["Personal or Automated","Detected types"]) {
      const trigger=page.getByRole("button",{name:"More actions",exact:true});await trigger.focus();await page.keyboard.press("Enter");
      await page.getByRole("menuitem",{name,exact:true}).focus();await page.keyboard.press("ArrowRight");
      const panel=page.getByRole("menu",{name,exact:true});await expect(panel).toBeVisible();
      const bounds=await panel.boundingBox();expect(bounds!.x).toBeGreaterThanOrEqual(0);expect(bounds!.y).toBeGreaterThanOrEqual(0);expect(bounds!.y+bounds!.height).toBeLessThanOrEqual(height);
      await page.screenshot({path:info.outputPath(`${name=== "Detected types"?"types":"classification"}-${width}-${height}-${theme}.png`),animations:"disabled"});
      await page.keyboard.press("End");await page.keyboard.press("Home");await page.keyboard.press("Escape");await page.keyboard.press("Escape");await expect(trigger).toBeFocused();
    }
    await page.keyboard.press("Escape");

  }
  await page.setViewportSize({width:1480,height:980});
  await attention.getByRole("button",{name:"Dismiss attention for Synthetic 3",exact:true}).focus(); await page.keyboard.press("Enter");
  await expect(attention.getByText("Synthetic 3",{exact:true})).toHaveCount(0); await expect(row(page,3)).toBeVisible();
  await open(page,3); await menu(page,"Personal or Automated","Personal");
  await expect(row(page,3)).toHaveCount(0); await expect(page.locator(".conversation-title h2")).toHaveText("Synthetic 3");
  await page.keyboard.press("Escape");
  await page.getByRole("tab",{name:"Personal",exact:true}).click(); await expect(row(page,3)).toBeVisible();
  await open(page,0); await menu(page,"Detected types","Flights",true); await page.keyboard.press("Escape");
  await page.reload(); await expect(row(page,0).getByText("Flights",{exact:true})).toBeVisible(); await expect(row(page,0).locator(".mail-details button")).toHaveCount(0);
  await expect(row(page,3)).toBeVisible(); await expect(attention.getByText("Synthetic 3",{exact:true})).toHaveCount(0);
  await open(page,0); await menu(page,"Personal or Automated","Always Automated for this sender"); await page.keyboard.press("Escape");
  await expect(row(page,0)).toHaveCount(0); await expect(row(page,3)).toBeVisible();
  await page.getByRole("tab",{name:"Automated",exact:true}).click(); await expect(row(page,0)).toBeVisible();
  await open(page,0); await menu(page,"Personal or Automated","Forget sender preference"); await page.keyboard.press("Escape");
  await page.getByRole("tab",{name:"Personal",exact:true}).click(); await expect(row(page,0)).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
});

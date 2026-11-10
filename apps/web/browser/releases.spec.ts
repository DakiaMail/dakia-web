import { test, expect, type Page } from '@playwright/test';
import { accounts, page as mail } from './fixtures';
async function fixture(page: Page, owner = true) {
  let version = 'v0.2.0';
  await page.route('**/api/**', route => {
    const path = new URL(route.request().url()).pathname;
    let value: unknown = [];
    if (path === '/api/session') value = { user_id: 'release-owner', csrf_token: 'synthetic' };
    else if (path === '/api/local-access') value = { can_invite: owner };
    else if (path === '/api/releases') value = { installed_version: 'v0.1.0', status: 'checked', checked_at: 1, available: { version, revision: 'a'.repeat(40), image: 'synthetic', notes_url: `https://github.com/DakiaMail/dakia-web/releases/tag/${version}` } };
    else if (path === '/api/accounts') value = accounts;
    else if (path.endsWith('/messages') || path === '/api/search') value = mail;
    else if (path.endsWith('/events')) return route.fulfill({ contentType: 'text/event-stream', body: ': ready\n\n' });
    return route.fulfill({ contentType: 'application/json', body: JSON.stringify(value) });
  });
  await page.goto('/');
  return (next: string) => { version = next; };
}
async function tabTo(page: Page, name: string) {
  for (let n = 0; n < 80; n++) {
    if (await page.getByRole('button', { name, exact: true }).evaluate(el => el === document.activeElement)) return;
    await page.keyboard.press('Tab');
  }
  throw Error(`Could not Tab to ${name}`);
}
test('owner can inspect and dismiss with only the keyboard; persistence and newer releases', async ({ page }) => {
  const change = await fixture(page);
  await expect(page.getByRole('complementary', { name: 'Dakia release' })).toBeVisible();
  await tabTo(page, 'Details'); await page.keyboard.press('Enter');
  await expect(page.getByRole('heading', { name: 'Updates', exact: true })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('link', { name: 'Release notes' })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('link', { name: 'Upgrade instructions' })).toBeFocused();
  await tabTo(page, 'Dismiss release v0.2.0'); await page.keyboard.press('Space');
  await expect(page.getByRole('button', { name: 'Manage workspace' })).toBeFocused();
  await expect(page.getByRole('complementary', { name: 'Dakia release' })).toHaveCount(0);
  await page.reload();
  await expect(page.getByRole('button', { name: 'Manage workspace' })).toBeVisible();
  await expect(page.getByRole('complementary', { name: 'Dakia release' })).toHaveCount(0);
  change('v0.3.0'); await page.reload();
  await expect(page.getByRole('button', { name: 'Dismiss release v0.3.0' })).toBeVisible();
});
for (const theme of ['light', 'dark'] as const) {
  for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 600 }, { width: 1100, height: 440 }]) {
    test(`notice and Updates layout ${theme} ${viewport.width}x${viewport.height}`, async ({ page }) => {
      await page.setViewportSize(viewport);
      await fixture(page);
      // Use the same persisted theme setting as the app.
      if (theme === 'dark') {
        if (viewport.width < 600) await page.getByRole('button', { name: 'Open navigation' }).click();
        await page.getByRole('button', { name: 'Dark theme', exact: true }).click();
      } else if (viewport.width < 600) await page.getByRole('button', { name: 'Open navigation' }).click();
      const notice = page.getByRole('complementary', { name: 'Dakia release' });
      await expect(notice).toBeVisible();
      await notice.scrollIntoViewIfNeeded();
      await page.screenshot({ path: `test-results/release-notice-${theme}-${viewport.width}.png` });
      await page.getByRole('button', { name: 'Details', exact: true }).click();
      await expect(page.getByRole('heading', { name: 'Updates', exact: true })).toBeFocused();
      const section = page.locator('section[aria-labelledby="workspace-updates"]');
      await expect(section).toBeVisible();
      expect(await section.evaluate(el => el.scrollWidth <= el.clientWidth)).toBe(true);
      if (viewport.width < 600) await expect.poll(() => page.locator('.sidebar').evaluate(el => el.getBoundingClientRect().right)).toBeLessThanOrEqual(0);
      await page.screenshot({ path: `test-results/release-settings-${theme}-${viewport.width}.png` });
    });
  }
}
test('invited users have no release controls', async ({ page }) => {
  await fixture(page, false);
  await page.getByRole('button', { name: 'Manage workspace' }).click();
  await expect(page.getByRole('heading', { name: 'Workspace', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Updates', exact: true })).toHaveCount(0);
  await expect(page.getByRole('complementary', { name: 'Dakia release' })).toHaveCount(0);
});
test('mobile release controls stay outside the keyboard order when navigation is closed', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 600 });
  await fixture(page);
  await expect(page.getByRole('button', { name: 'Details', exact: true })).toHaveCount(0);
  await tabTo(page, 'Open navigation'); await page.keyboard.press('Enter');
  await expect(page.getByRole('button', { name: 'Details', exact: true })).toBeVisible();
  await tabTo(page, 'Details'); await page.keyboard.press('Enter');
  await expect(page.getByRole('heading', { name: 'Updates', exact: true })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(page.getByRole('link', { name: 'Release notes' })).toBeFocused();
});

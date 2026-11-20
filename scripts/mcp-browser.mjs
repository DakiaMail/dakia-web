// Real built-app keyboard verification with isolated synthetic cookie only.
// Start python3 scripts/mcp-disposable.py --serve first.
// node scripts/mcp-browser.mjs target/mcp-disposable-<id>/browser-state.json
import { createHash } from 'node:crypto';
import { chromium, expect } from '@playwright/test';
import { readFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
const state = JSON.parse(readFileSync(process.argv[2], 'utf8'));
const out = resolve(dirname(process.argv[2]), 'browser'); mkdirSync(out, { recursive: true });
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || '/usr/bin/chromium', args: ['--no-sandbox'] });
const context = await browser.newContext({ ignoreHTTPSErrors: true, viewport: { width: 1440, height: 1000 } });
await context.addCookies([{ name: '__Host-dakia', value: state.cookie, url: state.origin, secure: true, httpOnly: true, sameSite: 'Lax' }]);
const page = await context.newPage(); const shots = []; const passes = [];
async function tabTo(selector) {
  for (let i = 0; i < 150; i++) {
    if (await page.locator(selector).evaluateAll(elements => elements.includes(document.activeElement))) return;
    await page.keyboard.press('Tab');
  }
  throw new Error('Cannot reach with Tab: '+selector);
}
async function activate(selector) { await tabTo(selector); await page.keyboard.press('Enter'); }
async function shot(name) {
  await page.waitForFunction(() => document.getAnimations().every(a => a.playState !== 'running' || a.effect?.getTiming().iterations === Infinity));
  await page.screenshot({ path: `${out}/${name}.png` }); shots.push(`${out}/${name}.png`);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
}
try {
  const existing=await (await context.request.get(state.origin+'/api/agent-connections')).json();
  for (const c of existing.filter(c => ['Keyboard fixture','Keyboard renamed','Browser OAuth fixture'].includes(c.name))) await context.request.delete(state.origin+'/api/agent-connections/'+c.id, { headers: { Origin: state.origin, 'X-CSRF-Token': 'fixture-csrf' } });
  await page.goto(state.origin); await activate('[aria-label="Manage workspace"]');
  await expect(page.getByRole('heading', { name: 'Agent connections', exact: true })).toBeVisible();
  await activate('.agent-connections button:has-text("Create connection")');
  await expect(page.locator('#agent-name')).toBeFocused();
  await page.keyboard.type('Keyboard fixture');
  await shot('create-desktop-light');
  await page.keyboard.press('Enter');
  await expect(page.getByRole('heading', { name: 'Connection created', exact: true })).toBeVisible();
  await expect(page.locator('#agent-client')).toBeFocused();
  await page.setViewportSize({ width: 390, height: 540 }); await shot('setup-claude-narrow-short-light');
  await page.keyboard.press('ArrowDown'); await page.keyboard.press('Tab');
  await expect(page.getByText('Open ChatGPT on the web.', { exact: false })).toBeVisible();
  await shot('setup-chatgpt-narrow-short-light');
  await tabTo('#agent-client'); await page.keyboard.press('ArrowDown'); await page.keyboard.press('Tab');
  await shot('setup-generic-narrow-short-light');
  await activate('[role="dialog"] button:has-text("Done")');
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await activate('[aria-label="Dark theme"]');
  await activate('[aria-label="Edit Keyboard fixture"]');
  await expect(page.locator('#agent-name')).toBeFocused();
  await page.keyboard.press('ControlOrMeta+A'); await page.keyboard.type('Keyboard renamed');
  await tabTo('[aria-label="Allow sending"]'); await page.keyboard.press('Space');
  await tabTo('[aria-label="Allow deleting"]'); await page.keyboard.press('Space');
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.evaluate(() => document.documentElement.classList.add('dark'));
  await shot('edit-desktop-dark');
  await activate('[role="dialog"] button:has-text("Save changes")');
  await expect(page.locator('[aria-label="Edit Keyboard renamed"]')).toBeFocused();
  await page.reload(); await activate('[aria-label="Manage workspace"]');
  const card=page.locator('.agent-card').filter({ has: page.getByRole('heading', { name: 'Keyboard renamed', exact: true }) });
  await expect(card).toContainText('Sending on'); await expect(card).toContainText('Deleting on');
  passes.push('Keyboard create, all three setup clients, independent permissions, rename, focus restoration and persistence after reload');
  await page.getByRole('heading', { name: 'Agent connections', exact: true }).scrollIntoViewIfNeeded();
  await shot('connections-desktop-dark');
  await activate('[aria-label="Set up Keyboard renamed"]');
  await expect(page.getByText('Save your token', { exact: true })).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(page.locator('[aria-label="Set up Keyboard renamed"]')).toBeFocused();
  await page.setViewportSize({ width: 390, height: 540 });
  await activate('[aria-label="Revoke Keyboard renamed"]'); await shot('revoke-narrow-short-dark');
  await activate('[role="dialog"] button:has-text("Keep connection")');
  await expect(page.locator('[aria-label="Revoke Keyboard renamed"]')).toBeFocused();
  await activate('[aria-label="Revoke Keyboard renamed"]');
  await activate('[role="dialog"] button:has-text("Revoke connection")');
  await expect(page.locator('[aria-label="Edit Keyboard renamed"]')).toHaveCount(0);
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.reload(); await activate('[aria-label="Manage workspace"]'); await expect(page.locator('[aria-label="Edit Keyboard renamed"]')).toHaveCount(0);
  passes.push('Token not redisplayed; Escape/cancel focus restoration; keyboard revoke persists after reload');
  await page.setViewportSize({ width: 1440, height: 1000 });
  await activate('[aria-label="Light theme"]');
  await page.getByRole('heading', { name: 'Agent connections', exact: true }).scrollIntoViewIfNeeded(); await shot('connections-desktop-light');
  // OAuth consent rendering uses a separate synthetic connection, not an owner login.
  const created=await context.request.post(state.origin+'/api/agent-connections', { headers: { Origin: state.origin, 'X-CSRF-Token': 'fixture-csrf' }, data: { name: 'Browser OAuth fixture' } });
  const connection=await created.json(); expect(connection.connection.id).toBeTruthy();
  const registered=await context.request.post(state.origin+'/oauth/register', { data: { client_name: 'Synthetic MCP client', redirect_uris: ['https://client.example.invalid/callback'], token_endpoint_auth_method: 'none' } });
  const client=await registered.json();
  const parameters=new URLSearchParams({ client_id: client.client_id, redirect_uri: 'https://client.example.invalid/callback', response_type: 'code', code_challenge_method: 'S256', code_challenge: createHash('sha256').update('synthetic-verifier-'.repeat(4)).digest('base64url'), resource: state.origin+'/mcp', scope: 'mail', state: 'synthetic' });
  await page.goto(state.origin+'/oauth/authorize?'+parameters);
  await expect(page.locator('#connection')).toBeFocused(); await shot('oauth-consent-desktop-light');
  await page.setViewportSize({ width: 390, height: 540 }); await page.emulateMedia({ colorScheme: 'dark' }); await shot('oauth-consent-narrow-short-dark');
  await tabTo('button[value="deny"]');
  const response=page.waitForResponse(r => r.url().endsWith('/oauth/authorize') && r.request().method()==='POST');
  await page.keyboard.press('Enter'); const received=await response; console.log('OAuth form result', received.status(), await received.text(), 'origin', (await received.request().allHeaders()).origin); expect(received.status()).toBe(303);
  passes.push('OAuth consent keyboard initial focus, light/dark and narrow/short rendering, keyboard cancellation');
  console.log(JSON.stringify({ passes, screenshots: shots }, null, 2));
  writeFileSync(`${out}/results.json`, JSON.stringify({ passes, screenshots: shots }, null, 2));
} finally { await browser.close(); }

// Runs inside a disposable published image, including with networking disabled.
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { readFile, writeFile, readdir } from 'node:fs/promises';

const origin = 'https://mail.test';
const base = 'http://127.0.0.1:3025';
const sessionPath = '/data/image-smoke-session.json';
let cookie;
async function request(path, init = {}) {
  return fetch(`${base}${path}`, {
    ...init,
    headers: { Origin: origin, ...(cookie ? { Cookie: cookie } : {}), ...init.headers },
    redirect: 'manual',
  });
}
assert.equal((await request('/api/accounts/gmail')).status, 401);
assert.equal((await request('/api/accounts/microsoft')).status, 401);
assert.equal((await request('/api/releases')).status, 401);
if (process.argv.includes('--resume')) {
  ({ cookie } = JSON.parse(await readFile(sessionPath, 'utf8')));
} else {
  const code = (await readFile('/secrets/setup-token', 'utf8')).trim();
  const response = await request('/auth/login', {
    method: 'POST',
    body: new URLSearchParams({ code, username: 'image-smoke', password: randomBytes(32).toString('base64url') }),
  });
  assert.equal(response.status, 303);
  cookie = response.headers.get('set-cookie')?.split(';')[0];
  assert.ok(cookie?.startsWith('__Host-dakia='));
  await writeFile(sessionPath, JSON.stringify({ cookie }), { mode: 0o600 });
}
let response = await request('/api/session');
assert.equal(response.status, 200);
const { csrf_token: csrf } = await response.json();
const releaseStatus = await (await request('/api/releases')).json();
assert.equal(releaseStatus.status, 'disabled');
assert.equal(releaseStatus.available, null);
const expectedVersion = process.env.DAKIA_EXPECTED_RELEASE_VERSION;
assert.equal(releaseStatus.installed_version, expectedVersion?.startsWith('v') ? expectedVersion : null);
assert.equal((await request('/api/accounts/gmail')).status, 200);
assert.deepEqual(await (await request('/api/accounts/gmail')).json(), { enabled: true });
const body = JSON.stringify({ email: 'fixture@example.test', display_name: 'Image fixture' });
assert.equal((await request('/api/accounts/gmail', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body })).status, 403);
response = await request('/api/accounts/gmail', {
  method: 'POST', headers: { 'Content-Type': 'application/json', 'X-CSRF-Token': csrf }, body,
});
assert.equal(response.status, 200);
const { url } = await response.json();
const auth = new URL(url);
assert.equal(auth.origin, 'https://accounts.google.com');
assert.equal(auth.searchParams.get('client_id'), 'image-fixture.apps.googleusercontent.com');
assert.equal(auth.searchParams.get('redirect_uri'), `${origin}/api/accounts/gmail/callback`);
assert.equal(auth.searchParams.get('code_challenge_method'), 'S256');
assert.equal(auth.searchParams.get('access_type'), 'offline');
assert.equal(auth.searchParams.get('scope'), 'openid email https://mail.google.com/');
assert.ok(auth.searchParams.get('state').length >= 43);
const callback = `/api/accounts/gmail/callback?${new URLSearchParams({ state: auth.searchParams.get('state'), error: 'access_denied' })}`;
response = await request(callback);
assert.equal(response.status, 303);
assert.equal(response.headers.get('location'), '/?gmail_error=cancelled');
assert.equal((await request(callback)).headers.get('location'), '/?gmail_error=expired');
assert.deepEqual(await (await request('/api/accounts')).json(), []);
assert.deepEqual(await (await request('/api/accounts/microsoft')).json(), { enabled: true });
assert.equal((await request('/api/accounts/microsoft', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body })).status, 403);
response = await request('/api/accounts/microsoft', {
  method: 'POST', headers: { 'Content-Type': 'application/json', 'X-CSRF-Token': csrf }, body,
});
assert.equal(response.status, 200);
const microsoft = new URL((await response.json()).url);
assert.equal(microsoft.origin, 'https://login.microsoftonline.com');
assert.equal(microsoft.pathname, '/common/oauth2/v2.0/authorize');
assert.equal(microsoft.searchParams.get('client_id'), 'microsoft-image-fixture');
assert.equal(microsoft.searchParams.get('redirect_uri'), `${origin}/api/accounts/microsoft/callback`);
assert.equal(microsoft.searchParams.get('code_challenge_method'), 'S256');
assert.ok(microsoft.searchParams.get('scope').includes('https://graph.microsoft.com/Mail.ReadWrite'));
assert.ok(microsoft.searchParams.get('scope').includes('https://graph.microsoft.com/Mail.Send'));
assert.ok(microsoft.searchParams.get('scope').includes('offline_access'));
const microsoftCallback = `/api/accounts/microsoft/callback?${new URLSearchParams({ state: microsoft.searchParams.get('state'), error: 'access_denied' })}`;
assert.equal((await request(microsoftCallback)).headers.get('location'), '/?microsoft_error=cancelled');
assert.equal((await request(microsoftCallback)).headers.get('location'), '/?microsoft_error=expired');
assert.deepEqual(await (await request('/api/accounts')).json(), []);
const assets = (await readdir('/app/web/assets')).filter(name => name.endsWith('.js'));
const sources = await Promise.all(assets.map(name => readFile(`/app/web/assets/${name}`, 'utf8')));
assert.ok(sources.some(source => source.includes('Continue with Google')), 'Gmail sign-in UI must be packaged');
assert.ok(sources.some(source => source.includes('Continue with Microsoft')), 'Microsoft sign-in UI must be packaged');
console.log(`Packaged Gmail and Microsoft OAuth checks passed${process.argv.includes('--resume') ? ' after restart' : ''}.`);

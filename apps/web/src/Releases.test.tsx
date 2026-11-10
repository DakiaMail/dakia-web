// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ReleaseNotice, ReleaseSettings, type ReleaseStatus } from './Releases';
import { AppUpdateProvider } from './AppUpdate';
let owner = true;
let status: ReleaseStatus;
let client: QueryClient;
let browserVersion = 'installed';
beforeEach(() => {
  localStorage.clear(); owner = true; browserVersion = 'installed';
  status = { installed_version: 'v0.1.0', status: 'checked', checked_at: 1, available: { version: 'v0.2.0', revision: 'a'.repeat(40), image: 'digest', notes_url: 'https://github.com/DakiaMail/dakia-web/releases/tag/v0.2.0' } };
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  vi.stubGlobal('fetch', vi.fn(async (url: string) => new Response(JSON.stringify(
    url === '/api/local-access' ? { can_invite: owner } : url === '/app-version.json' ? { version: browserVersion } : status))));
  Element.prototype.scrollIntoView = vi.fn();
});
afterEach(() => { cleanup(); client.clear(); vi.unstubAllGlobals(); });
const mount = (user = 'owner', browser = false) => render(<QueryClientProvider client={client}><AppUpdateProvider enabled={browser} buildId="installed"><button aria-label="Manage workspace">Workspace</button><ReleaseNotice userId={user} onDetails={() => {}} /><ReleaseSettings userId={user} /></AppUpdateProvider></QueryClientProvider>);
it('persists dismissal for this user and version; newer releases reappear', async () => {
  const view = mount();
  await screen.findByRole('button', { name: 'Dismiss release v0.2.0' });
  fireEvent.click(screen.getByRole('button', { name: 'Dismiss release v0.2.0' }));
  expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Manage workspace' }));
  expect(screen.queryByRole('complementary', { name: 'Dakia release' })).toBeNull();
  expect(screen.getByRole('heading', { name: 'Updates' })).toBeTruthy();
  view.unmount(); mount();
  await screen.findByRole('heading', { name: 'Updates' });
  expect(screen.queryByRole('complementary', { name: 'Dakia release' })).toBeNull();
  status = { ...status, available: { ...status.available!, version: 'v0.3.0' } };
  await client.invalidateQueries({ queryKey: ['releases'] });
  await screen.findByRole('button', { name: 'Dismiss release v0.3.0' });
});
it('does not fetch releases for invited or OIDC users', async () => {
  owner = false; mount('guest');
  await waitFor(() => expect(fetch).toHaveBeenCalledWith('/api/local-access', expect.anything()));
  expect(vi.mocked(fetch).mock.calls.some(([url]) => url === '/api/releases')).toBe(false);
  expect(screen.queryByRole('heading', { name: 'Updates' })).toBeNull();
});
it.each(['disabled', 'unknown', 'unavailable'] as const)('stays quiet when checks are %s', async state => {
  status.status = state; mount();
  await screen.findByRole('heading', { name: 'Updates' });
  await waitFor(() => expect(client.getQueryData(['releases', 'owner'])).toBeTruthy());
  expect(screen.queryByRole('complementary', { name: 'Dakia release' })).toBeNull();
  expect(screen.queryByText('You’re using the latest release.')).toBeNull();
});
it('prioritizes the browser reload notice', async () => {
  browserVersion = 'replacement'; mount('owner', true);
  await screen.findByRole('button', { name: 'Reload' });
  expect(screen.queryByRole('complementary', { name: 'Dakia release' })).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Later' }));
  await screen.findByRole('complementary', { name: 'Dakia release' });
});

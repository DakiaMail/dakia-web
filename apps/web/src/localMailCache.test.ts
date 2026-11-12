import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import { QueryClient } from "@tanstack/react-query";
import { LocalMailCache } from "./localMailCache";
import { accounts, content, page as mail } from "../browser/fixtures";
let cache: LocalMailCache, client: QueryClient;
const folder = ["mail", "all", "INBOX", "", "", accounts.map(a => a.id)];
const body = ["content", content.message.account_id, content.message.id];
const auth = async (c: LocalMailCache, q: QueryClient, owner = "alice", authorized = accounts) => {
  await c.identify(owner, q); await c.activate(owner, authorized, q, c.ticket());
};
beforeEach(async () => {
  vi.stubGlobal("indexedDB", new IDBFactory());
  cache = new LocalMailCache(false); client = new QueryClient(); await auth(cache, client);
});
afterEach(async () => { await cache.clear(); client.clear(); vi.unstubAllGlobals(); });
async function save() {
  client.setQueryData(folder, { pages: [{ ...mail, continuation: "expired-private-token" }], pageParams: [null] });
  client.setQueryData(body, content);
  await cache.flush();
}
test("reopening restores folder headers after authorization and reads bodies lazily", async () => {
  await save();
  const next = new QueryClient(), reopened = new LocalMailCache(false);
  expect(await reopened.body(content.message.account_id, content.message.id)).toBeUndefined();
  expect(next.getQueryData(folder)).toBeUndefined();
  await auth(reopened, next);
  expect(next.getQueryData(body)).toBeUndefined();
  expect(next.getQueryData(folder)).toMatchObject({ pages: [{ rows: mail.rows, continuation: null, counts_exact: false }], pageParams: [null] });
  expect(await reopened.body(content.message.account_id, content.message.id)).toEqual(content);
  next.clear(); await reopened.clear();
});
test("another owner and removed accounts cannot recover stored mail", async () => {
  await save();
  const next = new QueryClient(), reopened = new LocalMailCache(false);
  await auth(reopened, next, "bob", accounts);
  expect(next.getQueryData(folder)).toBeUndefined();
  expect(await reopened.body(content.message.account_id, content.message.id)).toBeUndefined();
  await reopened.clear(); next.clear();
});
test("fresh account authorization prunes removed accounts, including memory", async () => {
  await save(); await cache.activate("alice", [accounts[1]], client, cache.ticket());
  expect(client.getQueryData(body)).toBeUndefined();
  expect(await cache.body(content.message.account_id, content.message.id)).toBeUndefined();
});
test("sign-out fences late account activation and stale writes from another tab", async () => {
  await save();
  const stale = new LocalMailCache(false), second = new QueryClient(); await auth(stale, second);
  const ticket = cache.ticket();
  await cache.clear();
  await cache.activate("alice", accounts, client, ticket);
  second.setQueryData(body, content); await stale.flush();
  const fresh = new LocalMailCache(false), next = new QueryClient(); await auth(fresh, next);
  expect(next.getQueryData(folder)).toBeUndefined();
  expect(await fresh.body(content.message.account_id, content.message.id)).toBeUndefined();
  await fresh.clear(); second.clear(); next.clear();
});
test("known mail actions invalidate saved folders before stale results can be saved", async () => {
  await save();
  await cache.invalidateFolders(accounts.map(a => a.id));
  client.setQueryData(folder, { pages: [mail], pageParams: [null] }); await cache.flush();
  const fresh = new LocalMailCache(false), next = new QueryClient(); await auth(fresh, next);
  expect(next.getQueryData(folder)).toBeUndefined();
  cache.resumeFolders(accounts.map(a => a.id)); await fresh.clear(); next.clear();
});
test("cached body reads do not renew the retention deadline", async () => {
  await save();
  const start = Date.now();
  vi.spyOn(Date, "now").mockReturnValue(start + 6 * 86400000);
  const cached = await cache.body(content.message.account_id, content.message.id);
  client.setQueryData(body, cached); await cache.flush();
  vi.spyOn(Date, "now").mockReturnValue(start + 8 * 86400000);
  expect(await cache.body(content.message.account_id, content.message.id)).toBeUndefined();
  vi.restoreAllMocks();
});
test("expired entries and provider cursors never hydrate", async () => {
  await save(); vi.spyOn(Date, "now").mockReturnValue(Date.now() + 8 * 86400000);
  const fresh = new LocalMailCache(false), next = new QueryClient(); await auth(fresh, next);
  expect(next.getQueryData(folder)).toBeUndefined();
  await fresh.clear(); next.clear(); vi.restoreAllMocks();
});
test("storage denial keeps activation and online reads usable", async () => {
  vi.stubGlobal("indexedDB", { open: () => { throw new Error("denied"); } });
  const denied = new LocalMailCache(false), next = new QueryClient(); await auth(denied, next);
  next.setQueryData(body, content); await denied.flush();
  expect(await denied.body(content.message.account_id, content.message.id)).toBeUndefined();
  await denied.clear(); next.clear();
});
test("does not persist searches, secrets, drafts, or oversized bodies", async () => {
  client.setQueryData(["session"], { csrf_token: "private" });
  client.setQueryData(["drafts", "work"], [{ text: "unsent" }]);
  client.setQueryData(["mail", "all", "INBOX", "private query", "", accounts.map(a => a.id)], { pages: [mail], pageParams: [null] });
  client.setQueryData(body, { ...content, html: "x".repeat(3 * 1024 * 1024) });
  await cache.flush();
  const fresh = new LocalMailCache(false), next = new QueryClient(); await auth(fresh, next);
  expect(next.getQueryCache().getAll()).toHaveLength(0);
  expect(await fresh.body(content.message.account_id, content.message.id)).toBeUndefined();
  await fresh.clear(); next.clear();
});
test("folder count and body byte budgets evict oldest records", async () => {
  for (let i = 0; i < 40; i++) client.setQueryData(["mail", "all", `folder${i}`, "", "", accounts.map(a => a.id)], { pages: [mail], pageParams: [null] });
  await cache.flush();
  const fresh = new LocalMailCache(false), next = new QueryClient(); await auth(fresh, next);
  expect(next.getQueryCache().findAll({ queryKey: ["mail"] })).toHaveLength(32);
  for (let i = 0; i < 40; i++) {
    client.setQueryData(["content", "work", String(i)], { ...content, message: { ...content.message, id: String(i) }, html: "x".repeat(1024 * 1024) });
  }
  await cache.flush();
  const db = await new Promise<IDBDatabase>((resolve, reject) => { const r = indexedDB.open("dakia-local-mail-v2"); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
  const sizes = await new Promise<{ bytes: number }[]>(resolve => { const r = db.transaction("sizes").objectStore("sizes").getAll(); r.onsuccess = () => resolve(r.result); });
  expect(sizes.reduce((sum, entry) => sum + entry.bytes, 0)).toBeLessThanOrEqual(64 * 1024 * 1024);
  expect(sizes.length).toBeLessThan(40); db.close();
  await fresh.clear(); next.clear();
});
test("reconnecting an account with a new identity discards its previous mail", async () => {
  await save();
  await cache.activate("alice", accounts.map(a => ({ ...a, email: `new-${a.email}` })), client, cache.ticket());
  expect(client.getQueryData(body)).toBeUndefined();
  expect(await cache.body(content.message.account_id, content.message.id)).toBeUndefined();
});
test.each(["subject", "classification"])("a malformed saved %s falls back to live mail instead of crashing the reader", async field => {
  await save();
  const db = await new Promise<IDBDatabase>(resolve => { const r = indexedDB.open("dakia-local-mail-v2"); r.onsuccess = () => resolve(r.result); });
  await new Promise<void>(resolve => {
    const tx = db.transaction("entries", "readwrite"), store = tx.objectStore("entries");
    const read = store.get(JSON.stringify(["alice", folder]));
    read.onsuccess = () => {
      const entry = read.result, data = JSON.parse(entry.json);
      data.pages[0].rows[0][field] = { corrupt: true };
      entry.json = JSON.stringify(data); store.put(entry);
    };
    tx.oncomplete = () => resolve();
  }); db.close();
  const fresh = new LocalMailCache(false), next = new QueryClient(); await auth(fresh, next);
  expect(next.getQueryData(folder)).toBeUndefined();
  expect(await fresh.body(content.message.account_id, content.message.id)).toEqual(content);
  await fresh.clear(); next.clear();
});
test("a late session response cannot authorize the cache again after sign-out", async () => {
  await save();
  const ticket = cache.ticket(); await cache.clear();
  expect(await cache.identify("alice", client, ticket)).toBe(false);
  await cache.activate("alice", accounts, client, ticket);
  client.setQueryData(body, content); await cache.flush();
  const fresh = new LocalMailCache(false), next = new QueryClient(); await auth(fresh, next);
  expect(await fresh.body(content.message.account_id, content.message.id)).toBeUndefined();
  await fresh.clear(); next.clear();
});

test("opening the new namespace discards the obsolete category cache", async () => {
  await new Promise<void>(resolve => { const r = indexedDB.open("dakia-local-mail-v1"); r.onsuccess = () => { r.result.close(); resolve(); }; });
  const fresh = new LocalMailCache(false), next = new QueryClient();
  await auth(fresh, next);
  expect((await indexedDB.databases()).some(db => db.name === "dakia-local-mail-v1")).toBe(false);
  await fresh.clear(); next.clear();
});

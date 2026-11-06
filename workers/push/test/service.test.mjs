import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createECDH, randomBytes } from 'node:crypto';
import webpush from 'web-push';
import { PushService, validatePush, payloadFor } from '../service.mjs';
const vapid = { subject: 'mailto:operator@example.test', ...webpush.generateVAPIDKeys() };
const ecdh = createECDH('prime256v1'); ecdh.generateKeys();
const job = () => ({ id: 'notification-1', account_id: 'a', user_id: 'u', message_id: 'm', subscription_id: 's', scope: { account_id: 'a', user_id: 'u' }, subscription: { id: 's', user_id: 'u', endpoint: 'https://fcm.googleapis.com/fcm/send/test', keys: { p256dh: ecdh.getPublicKey().toString('base64url'), auth: randomBytes(16).toString('base64url') } }, deadline: new Date(Date.now() + 60_000).toISOString(), initial_sync: false });

test('official web-push produces encrypted generic payload, bounded TTL and account URL', () => {
  const j = job(); validatePush(j);
  const payload = payloadFor(j); assert.deepEqual(Object.keys(JSON.parse(payload)).sort(), ['tag', 'url']); assert.equal(JSON.parse(payload).url, '/?account=a&message=m');
  const details = webpush.generateRequestDetails(j.subscription, payload, { vapidDetails: vapid, TTL: 300, urgency: 'normal', contentEncoding: 'aes128gcm' });
  assert.equal(details.headers.TTL, 300); assert.equal(details.headers['Content-Encoding'], 'aes128gcm');
  assert.ok(Buffer.isBuffer(details.body)); assert.ok(!details.body.includes(Buffer.from('/?account=a')));
});

test('cross-user/account, old jobs, initial-sync and arbitrary endpoint/paths never reach sender', async () => {
  let calls = 0; const service = new PushService(vapid, async () => { calls++; return { statusCode: 201 }; });
  for (const change of [j => j.scope.user_id = 'v', j => j.scope.account_id = 'b', j => j.subscription.user_id = 'v', j => j.subscription_id = 'other', j => j.deadline = '2000-01-01', j => j.initial_sync = true, j => j.path = '/other/user', j => j.subscription.endpoint = 'https://127.0.0.1/private', j => j.subscription.endpoint = 'https://fcm.googleapis.com.evil.test/push', j => j.subscription.endpoint = 'http://fcm.googleapis.com/push', j => j.subscription.endpoint = 'https://user:password@fcm.googleapis.com/push', j => j.subscription.keys.auth = 'bad']) {
    const j = job(); change(j); await assert.rejects(service.send(j));
  }
  assert.equal(calls, 0);
});

test('sent, expired, rate-limited and transport outcomes are bounded and contain no keys', async () => {
  let options;
  assert.deepEqual(await new PushService(vapid, async (s, p, o) => { options = o; return { statusCode: 201 }; }).send(job()), { id: 'notification-1', status: 'sent' });
  assert.equal(options.TTL, 300); assert.ok(options.timeout <= 10_000);
  for (const [statusCode, expected] of [[404, 'expired'], [410, 'expired'], [429, 'retryable'], [503, 'retryable'], [401, 'failed'], [undefined, 'retryable']]) {
    const service = new PushService(vapid, async () => { throw { statusCode, headers: { 'retry-after': '999999' }, body: 'private-provider-response' }; });
    const result = await service.send(job()); assert.equal(result.status, expected);
    assert.ok(!JSON.stringify(result).includes('private')); if (expected === 'retryable') assert.equal(result.retry_after_seconds, 3600);
  }
});

test('Chromium jmt17 endpoint is supported without allowing arbitrary Google hosts', async () => {
  const j = job();
  j.subscription.endpoint = 'https://jmt17.google.com/fcm/send/synthetic';
  validatePush(j);
  let calls = 0;
  const service = new PushService(vapid, async () => { calls++; return { statusCode: 201 }; });
  assert.equal((await service.send(j)).status, 'sent');
  for (const endpoint of ['https://jmt17.google.com.evil.test/push', 'https://other.google.com/push', 'http://jmt17.google.com/push', 'https://jmt17.google.com:8443/push', 'https://user:password@jmt17.google.com/push', 'https://jmt17.google.com/push#fragment']) {
    j.subscription.endpoint = endpoint;
    await assert.rejects(service.send(j));
  }
  assert.equal(calls, 1);
});

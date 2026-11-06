// Synthetic storage-to-worker fixture. Never invokes a network transport.
import fs from 'node:fs';
import assert from 'node:assert/strict';
import { createECDH, randomBytes } from 'node:crypto';
import webpush from 'web-push';
import { PushService } from '../service.mjs';
const job = JSON.parse(fs.readFileSync(0, 'utf8'));
const ecdh = createECDH('prime256v1');
ecdh.generateKeys();
job.subscription.keys = { p256dh: ecdh.getPublicKey().toString('base64url'), auth: randomBytes(16).toString('base64url') };
const vapid = { subject: 'mailto:fixture@example.test', ...webpush.generateVAPIDKeys() };
let calls = 0;
const service = new PushService(vapid, async (subscription, payload, options) => {
  calls++;
  const encrypted = webpush.generateRequestDetails(subscription, payload, options);
  assert.equal(encrypted.headers['Content-Encoding'], 'aes128gcm');
  assert.ok(!encrypted.body.includes(Buffer.from(job.message_id)));
  return { statusCode: 201 };
});
const result = await service.send(job);
assert.equal(calls, 1);
process.stdout.write(JSON.stringify({ ...result, synthetic_transport_calls: calls, external_requests: 0 }));

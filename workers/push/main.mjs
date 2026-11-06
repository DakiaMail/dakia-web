import { readFile } from 'node:fs/promises';
import { PushService } from './service.mjs';
// Operator-owned key file, not a request field. Never write key material or
// subscriptions to stdout/stderr. Rust persists keys/subscriptions/jobs.
if (!process.env.DAKIA_VAPID_FILE) throw new Error('DAKIA_VAPID_FILE required');
const service = new PushService(JSON.parse(await readFile(process.env.DAKIA_VAPID_FILE, 'utf8')));
const send = value => process.stdout.write(`${JSON.stringify(value)}\n`);
let buffer = ''; process.stdin.setEncoding('utf8');
for await (const chunk of process.stdin) {
  buffer += chunk;
  if (Buffer.byteLength(buffer) > 65_536) { send({ error: 'input_too_large' }); process.exitCode = 1; break; }
  let end;
  while ((end = buffer.indexOf('\n')) >= 0) {
    const line = buffer.slice(0, end); buffer = buffer.slice(end + 1);
    try { send({ result: await service.send(JSON.parse(line)) }); }
    catch { send({ error: 'push_failed' }); }
  }
}

import { readFile, writeFile, link, unlink, lstat } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import webpush from 'web-push';
import { PushService } from './service.mjs';

const file = process.env.DAKIA_VAPID_FILE;
if (!file) throw new Error('DAKIA_VAPID_FILE required');
async function validateExisting() {
  const info = await lstat(file);
  if (!info.isFile() || (info.mode & 0o077)) throw new Error('Private regular VAPID file required');
  new PushService(JSON.parse(await readFile(file, 'utf8')));
}
try {
  try {
    await validateExisting();
    console.log('Existing VAPID preserved');
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
    if (!process.env.DAKIA_VAPID_SUBJECT) throw new Error('DAKIA_VAPID_SUBJECT required');
    const config = { subject: process.env.DAKIA_VAPID_SUBJECT, ...webpush.generateVAPIDKeys() };
    new PushService(config);
    // Publish a complete file atomically, without replacing another startup's keys.
    const temporary = `${file}.${randomUUID()}.tmp`;
    try {
      await writeFile(temporary, JSON.stringify(config), { mode: 0o600, flag: 'wx' });
      try { await link(temporary, file); }
      catch (error) { if (error.code !== 'EEXIST') throw error; }
      await validateExisting();
    } finally { await unlink(temporary).catch(() => {}); }
    console.log('VAPID configured');
  }
} catch {
  // Never include supplied key material or dependency validation details in logs.
  console.error('VAPID configuration failed: check private file permissions and valid keys; DAKIA_VAPID_SUBJECT required (defaults to DAKIA_ORIGIN at startup) for a new file');
  process.exitCode = 1;
}

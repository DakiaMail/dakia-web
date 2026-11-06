import { writeFile } from 'node:fs/promises';
import webpush from 'web-push';
if (!process.env.DAKIA_VAPID_FILE) throw new Error('DAKIA_VAPID_FILE required');
if (!process.env.DAKIA_VAPID_SUBJECT) throw new Error('DAKIA_VAPID_SUBJECT required');
try {
  await writeFile(process.env.DAKIA_VAPID_FILE, JSON.stringify({ subject: process.env.DAKIA_VAPID_SUBJECT, ...webpush.generateVAPIDKeys() }), { mode: 0o600, flag: 'wx' });
  console.log('VAPID configured');
} catch (error) {
  if (error.code !== 'EEXIST') throw new Error('VAPID configuration failed');
  console.log('Existing VAPID preserved');
}

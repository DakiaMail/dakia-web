import webpush from 'web-push';

// Fixed push-service origins only. Subscription endpoints are untrusted and
// must never turn the helper into a general HTTP/private-network client.
export function validatePush(job, now = Date.now()) {
  const { scope, subscription } = job;
  if (!scope || !subscription || typeof job.id !== 'string' || !job.id || typeof job.account_id !== 'string' || !job.account_id || typeof job.message_id !== 'string' || !job.message_id
      || job.account_id !== scope.account_id || job.user_id !== scope.user_id || !job.user_id
      || subscription.user_id !== scope.user_id || subscription.id !== job.subscription_id
      || typeof subscription.id !== 'string' || !subscription.id || !Number.isFinite(Date.parse(job.deadline)) || Date.parse(job.deadline) <= now
      || !Number.isFinite(Date.parse(job.deadline)) || job.initial_sync !== false
      || Object.keys(job).some(k => !['id','account_id','user_id','message_id','subscription_id','scope','subscription','deadline','initial_sync'].includes(k))) throw new Error('invalid_push_job');
  const url = new URL(subscription.endpoint);
  if (url.protocol !== 'https:' || url.port || url.username || url.password || url.hash || !['fcm.googleapis.com', 'jmt17.google.com', 'updates.push.services.mozilla.com', 'web.push.apple.com'].includes(url.hostname)) throw new Error('invalid_push_endpoint');
  if (subscription.endpoint.length > 4096 || !/^[A-Za-z0-9_-]{87}$/.test(subscription.keys?.p256dh) || !/^[A-Za-z0-9_-]{22}$/.test(subscription.keys?.auth)) throw new Error('invalid_push_subscription');
}

export function payloadFor(job) {
  // No mail text, credentials or arbitrary fetch URL crosses the push service.
  return JSON.stringify({ tag: `dakia:${job.id}`, url: `/?account=${encodeURIComponent(job.account_id)}&message=${encodeURIComponent(job.message_id)}` });
}

export class PushService {
  constructor(vapid, sender = webpush.sendNotification.bind(webpush)) {
    // Validate without logging the private key. Keep the configured subject
    // (operator contact) out of jobs and never mutate web-push global state.
    webpush.generateRequestDetails({ endpoint: 'https://fcm.googleapis.com/test' }, null, { vapidDetails: vapid });
    this.vapid = vapid; this.sender = sender;
  }
  async send(job) {
    validatePush(job);
    const subscription = { endpoint: job.subscription.endpoint, keys: job.subscription.keys };
    const options = { vapidDetails: this.vapid, TTL: 300, urgency: 'normal', timeout: Math.min(10_000, Date.parse(job.deadline) - Date.now()), contentEncoding: 'aes128gcm' };
    try {
      const response = await this.sender(subscription, payloadFor(job), options);
      return { id: job.id, status: response.statusCode >= 200 && response.statusCode < 300 ? 'sent' : 'failed' };
    } catch (error) {
      const code = Number(error.statusCode);
      if (code === 404 || code === 410) return { id: job.id, status: 'expired' };
      const retryable = !code || code === 429 || code >= 500;
      const retry = Number(error.headers?.['retry-after']);
      return { id: job.id, status: retryable ? 'retryable' : 'failed', retry_after_seconds: retryable ? (Number.isFinite(retry) && retry >= 0 ? Math.min(retry, 3600) : 60) : null };
    }
  }
}

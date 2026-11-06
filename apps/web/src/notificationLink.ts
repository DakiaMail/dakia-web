// Tab-local opaque target only, never mail text. Used only across explicit sign-in.
const key = "dakia-notification-target";
function target(search: string) {
  const params = new URLSearchParams(search);
  const account = params.get("account"), message = params.get("message");
  return account && message && account.length <= 200 && message.length <= 200
    ? { account, message } : null;
}
export function rememberNotificationLink() {
  try {
    const link = target(location.search);
    if (link) sessionStorage.setItem(key, JSON.stringify({ ...link, expires: Date.now() + 600_000 }));
    else sessionStorage.removeItem(key);
  } catch { /* Storage denial must never prevent sign-in. */ }
}
export function notificationLink() {
  const direct = target(location.search);
  let saved;
  try {
    const raw = sessionStorage.getItem(key);
    sessionStorage.removeItem(key);
    if (raw) saved = JSON.parse(raw);
  } catch {}
  if (direct) return direct;
  return saved && typeof saved.account === "string" && typeof saved.message === "string"
    && saved.account && saved.message && saved.account.length <= 200 && saved.message.length <= 200
    && Number.isFinite(saved.expires) && saved.expires > Date.now()
    ? { account: saved.account as string, message: saved.message as string } : null;
}

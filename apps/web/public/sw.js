/* Cache build-owned static assets only. Mail lives in the authenticated app's
 * bounded IndexedDB cache. Never store API/auth responses, navigations, remote
 * images or notification links in Cache Storage. Waiting updates still require
 * an explicit request from the app. */
self.addEventListener("message", (event) => {
  if (event.data?.type !== "DAKIA_SKIP_WAITING" || !event.source?.url) return;
  try {
    if (new URL(event.source.url).origin !== self.location.origin) return;
    event.waitUntil(self.skipWaiting());
  } catch {}
});
self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(
          keys.filter((key) => key.startsWith("dakia-shell-")).map((key) => caches.delete(key)),
        ),
      )
      .then(async () => {
        const cache = await caches.open(STATIC_CACHE);
        for (const key of await cache.keys()) if (!STATIC_ASSETS.includes(new URL(key.url).pathname)) await cache.delete(key);
      })
      .then(() => self.clients.claim()),
  );
});

// Only app-owned exact-message links. Support already queued legacy payloads.
function mailTarget(value) {
  try {
    const url = new URL(value, self.location.origin);
    if (url.origin !== self.location.origin) return null;
    const legacy = url.pathname.match(/^\/accounts\/([^/]+)\/messages\/([^/]+)$/);
    const account = legacy ? decodeURIComponent(legacy[1]) : url.pathname === "/" && url.searchParams.get("account");
    const message = legacy ? decodeURIComponent(legacy[2]) : url.pathname === "/" && url.searchParams.get("message");
    if (!account || !message || account.length > 200 || message.length > 200) return null;
    return { path: `/?account=${encodeURIComponent(account)}&message=${encodeURIComponent(message)}`,
      summary: `/api/accounts/${encodeURIComponent(account)}/messages/${encodeURIComponent(message)}/notification` };
  } catch { return null; }
}
function plain(value, limit) {
  if (typeof value !== "string") return "";
  return Array.from(value.replace(/<(script|style)\b[^>]*>[\s\S]*?<\/\1\s*>/gi, " ")
    .replace(/<[^>]*>/g, " ")
    .replace(/[\u0000-\u001f\u007f-\u009f\u200b-\u200f\u202a-\u202e\u2066-\u2069]/g, " ")
    .replace(/\s+/g, " ").trim()).slice(0, limit).join("");
}
self.addEventListener("push", (event) => {
  event.waitUntil((async () => {
    let data;
    try { data = event.data?.json(); } catch {}
    const target = mailTarget(data?.url);
    const tag = typeof data?.tag === "string" && data.tag.startsWith("dakia:") && data.tag.length <= 512
      ? data.tag : "dakia-new-mail";
    let title = "New mail";
    let body = "Open Dakia to read your message.";
    if (target) {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 2500);
      try {
        // Session/account authorization still applies. Never download a body,
        // follow a login redirect, persist text or put it through push transport.
        const response = await fetch(target.summary, { credentials: "same-origin", cache: "no-store",
          redirect: "error", signal: controller.signal });
        if (response.ok) {
          const summary = await response.json();
          const sender = plain(summary.sender, 80) || "Unknown sender";
          const subject = plain(summary.subject, 120) || "(No subject)";
          const preview = plain(summary.preview, 160);
          const account = plain(summary.account, 100);
          title = `${sender} · ${subject}`;
          body = [account, preview].filter(Boolean).join("\n") || "Open Dakia to read your message.";
        }
      } catch {} finally { clearTimeout(timer); }
    }
    await self.registration.showNotification(title, {
      body, tag, renotify: false, icon: "/dakia-icon-192.png",
      data: { path: target?.path || "/" },
    });
  })());
});
self.addEventListener("notificationclick", (event) => {
  event.notification.close();
  const target = new URL(mailTarget(event.notification.data?.path)?.path || "/", self.location.origin);
  event.waitUntil(
    self.clients
      .matchAll({ type: "window", includeUncontrolled: true })
      .then(async (clients) => {
        const client = clients.find(
          (client) => new URL(client.url).origin === target.origin,
        );
        if (client) {
          try {
            const navigated = await client.navigate(target.href);
            if (navigated) return navigated.focus();
          } catch {}
        }
        return self.clients.openWindow(target.href);
      }),
  );
});

// Cache only build-owned, content-hashed static assets. Navigations, API/auth,
// notification links and all mail/images remain outside Cache Storage.
const STATIC_CACHE = "dakia-static-v1";
const STATIC_ASSETS = /* DAKIA_STATIC_ASSETS */ [];
self.addEventListener("install", event => {
  event.waitUntil((async () => {
    const cache = await caches.open(STATIC_CACHE);
    await Promise.all(STATIC_ASSETS.map(async path => {
      const response = await fetch(path, { cache: "no-store", credentials: "omit" });
      if (response.ok && !response.headers.get("content-type")?.includes("text/html")) await cache.put(path, response);
    }));
    for (const key of await cache.keys()) if (!STATIC_ASSETS.includes(new URL(key.url).pathname)) await cache.delete(key);
  })().catch(() => {}));
});
self.addEventListener("fetch", event => {
  const url = new URL(event.request.url);
  if (event.request.method !== "GET" || url.origin !== self.location.origin || url.search || !STATIC_ASSETS.includes(url.pathname)) return;
  event.respondWith((async () => {
    const cache = await caches.open(STATIC_CACHE);
    const cached = await cache.match(url.pathname);
    if (cached) return cached;
    const response = await fetch(event.request);
    if (response.ok && !response.headers.get("content-type")?.includes("text/html")) await cache.put(url.pathname, response.clone());
    return response;
  })());
});

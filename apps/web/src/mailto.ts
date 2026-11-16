import { newDraft, parseRecipients } from "./Composer";
import { escapeHtml } from "@dakia/mail-renderer";

function decode(value: string) {
  try { return decodeURIComponent(value); } catch { throw new Error("This email link isn’t valid."); }
}
export function mailtoDraft(value: string, account: string) {
  if (value.length > 16000 || !/^mailto:/i.test(value)) throw new Error("This email link isn’t valid.");
  const [address, ...query] = value.slice(7).split("?");
  const fields = new Map<string, string[]>();
  for (const part of query.join("?").split("&")) {
    if (!part) continue;
    const separator = part.indexOf("=");
    const key = decode(separator < 0 ? part : part.slice(0, separator)).toLowerCase();
    if (!["to", "cc", "bcc", "subject", "body"].includes(key)) continue;
    const text = decode(separator < 0 ? "" : part.slice(separator + 1));
    if (key !== "body" && /[\x00-\x1f\x7f]/.test(text)) throw new Error("This email link contains an invalid header.");
    fields.set(key, [...fields.get(key) || [], text]);
  }
  const to = decode(address);
  if (/[\x00-\x1f\x7f]/.test(to)) throw new Error("This email link contains an invalid recipient.");
  const recipients = (key: string) => parseRecipients([...(key === "to" ? [to] : []), ...fields.get(key) || []].join(","));
  const draft = newDraft(account);
  draft.to = recipients("to"); draft.cc = recipients("cc"); draft.bcc = recipients("bcc");
  if (draft.to.length + draft.cc.length + draft.bcc.length > 100) throw new Error("This email link has too many recipients.");
  draft.subject = fields.get("subject")?.[0] || "";
  if (draft.subject.length > 998) throw new Error("This email link’s subject is too long.");
  draft.text = fields.get("body")?.[0] || "";
  draft.html = escapeHtml(draft.text).replace(/\r\n|\r|\n/g, "<br>");
  return draft;
}
const storageKey = "dakia-compose-link";
export function composeLink(): string | null {
  const url = new URL(location.href);
  let value = url.searchParams.get("mailto");
  if (!value && url.searchParams.get("compose") === "1") value = "mailto:";
  try {
    const saved = JSON.parse(sessionStorage.getItem(storageKey) || "null");
    sessionStorage.removeItem(storageKey);
    if (value === null && saved && typeof saved.value === "string" && saved.value.length <= 16000 && Number.isFinite(saved.expires) && saved.expires > Date.now()) value = saved.value;
  } catch { /* A direct email link still works without tab storage. */ }
  if (value !== null) {
    url.searchParams.delete("mailto"); url.searchParams.delete("compose");
    history.replaceState(history.state, "", url);
  }
  return value;
}
export function rememberComposeLink(value: string | null) {
  try {
    if (value !== null && value.length <= 16000) sessionStorage.setItem(storageKey, JSON.stringify({ value, expires: Date.now() + 600_000 }));
  } catch { /* Sign-in must remain available when browser storage is blocked. */ }
}

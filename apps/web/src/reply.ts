import type { Account, Address, MessageContent } from "@dakia/api-types";
import { newDraft } from "./Composer";
export type ReplyMode = "reply" | "all" | "forward";
const validId = (id: string) => !!id && !/[\r\n]/.test(id) && id.includes("@");
export function replyDraft(mode: ReplyMode, content: MessageContent, accounts: Account[]) {
  const m = content.message;
  const draft = newDraft(m.account_id);
  draft.subject = `${mode === "forward" ? "Fwd" : "Re"}: ${m.subject.replace(/^(?:(?:Re|Fwd?):\s*)+/i, "")}`;
  if (mode === "forward") return draft;
  // Never invent RFC IDs from provider/local IDs. Missing headers still permit
  // an ordinary reply, with only the real available ancestry preserved.
  draft.in_reply_to = content.message_id && validId(content.message_id) ? content.message_id : null;
  draft.references = [...new Set([...(content.references || []).filter(validId), ...(draft.in_reply_to ? [draft.in_reply_to] : [])])];
  const own = new Set(accounts.map(a => a.email.toLowerCase()));
  const fromSelf = own.has(m.sender.email.toLowerCase());
  const used = new Set<string>();
  function recipients(values: Address[]) {
    return values.filter(a => {
      const email = a.email.toLowerCase();
      if (!email || own.has(email) || used.has(email)) return false;
      used.add(email); return true;
    });
  }
  draft.to = recipients(fromSelf ? m.to : content.reply_to?.length ? content.reply_to : [m.sender]);
  if (mode === "all") {
    const others = recipients([...m.to, ...(content.cc || [])]);
    // A message addressed only to us may have everyone else in Cc.
    if (!draft.to.length) draft.to = others;
    else draft.cc = others;
  }
  return draft;
}

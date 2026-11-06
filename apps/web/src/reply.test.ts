import { describe, expect, it } from "vitest";
import { replyDraft } from "./reply";
import { accounts, content } from "../browser/fixtures";

describe("per-message conversation composition", () => {
  it("uses the chosen message RFC ancestry and Reply-To, never local thread IDs", () => {
    const draft = replyDraft("reply", { ...content, message_id: "<selected@example.invalid>", references: ["<root@example.invalid>"], reply_to: [{email:"replies@example.org",name:"Team"}] }, accounts);
    expect(draft.account_id).toBe("work");
    expect(draft.to.map(a=>a.email)).toEqual(["replies@example.org"]);
    expect(draft.in_reply_to).toBe("<selected@example.invalid>");
    expect(draft.references).toEqual(["<root@example.invalid>","<selected@example.invalid>"]);
  });
  it("continues sent mail to original recipients, not ourselves, and deduplicates reply-all", () => {
    const sent = { ...content, message: { ...content.message, sender: {email: accounts[0].email, name: "Alex"}, to: [{email:"friend@example.org",name:null},{email:"FRIEND@example.org",name:null}, {email:"ALEX@example.com",name:null}] }, cc: [{email:"friend@example.org",name:null},{email:"other@example.org",name:null},{email:"alex@example.net",name:null}] };
    const reply = replyDraft("all", sent, accounts);
    expect(reply.to.map(a=>a.email)).toEqual(["friend@example.org"]);
    expect(reply.cc.map(a=>a.email)).toEqual(["other@example.org"]);
    expect(replyDraft("reply", sent, accounts).to.map(a=>a.email)).toEqual(["friend@example.org"]);
  });
  it("does not fabricate absent headers or carry recipients/threading into a forward", () => {
    const missing = {...content, message_id:null, references:[]};
    expect(replyDraft("reply", missing, accounts).in_reply_to).toBeNull();
    expect(replyDraft("reply", missing, accounts).references).toEqual([]);
    const forward = replyDraft("forward", content, accounts);
    expect(forward.to).toEqual([]); expect(forward.cc).toEqual([]);
    expect(forward.in_reply_to).toBeNull(); expect(forward.references).toEqual([]);
  });
});

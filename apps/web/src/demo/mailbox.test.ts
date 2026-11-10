// @vitest-environment node
import { describe, expect, it } from "vitest";
import { DemoMailbox, type Persistence } from "./mailbox";
import { seed, identity, type DemoState } from "./seed";
import type { Draft, MessageContent, Operation, Submission } from "@dakia/api-types";

function fixture(initial: unknown = seed()) {
  let saved = structuredClone(initial), now = Date.now();
  const persistence: Persistence = { read: async () => structuredClone(saved), write: async s => { saved = structuredClone(s); } };
  const box = new DemoMailbox(persistence, () => now);
  const call = async (path: string, method = "GET", data?: unknown) => {
    const response = await box.fetch(new Request(`https://demo.dakiamail.com/api${path}`, { method, ...(data === undefined ? {} : { body: JSON.stringify(data), headers: { "Content-Type": "application/json" } }) }));
    return { status: response.status, data: await response.json() };
  };
  return { box, call, persistence, later: (ms: number) => { now += ms; } };
}

describe("browser-local mailbox", () => {
  it("provides typed session, two accounts, searchable bodies and complete conversations", async () => {
    const { call } = fixture();
    expect((await call("/session")).data.user_id).toMatch(/^sandbox-/);
    expect((await call("/accounts")).data).toHaveLength(2);
    const result = (await call('/search?accounts=work&q=from:%22maya@northline.example%22')).data;
    expect(result.rows.length).toBeGreaterThan(1);
    expect(result.coverage[0].state).toBe("complete");
    const content: MessageContent = (await call("/accounts/work/messages/letter-0")).data;
    expect(content.attachments).toHaveLength(1);
    expect((await call("/accounts/work/messages/letter-0/conversation")).data.rows).toHaveLength(2);
    expect((await call('/search?accounts=work&q=has:attachment%20subject:%22launch%22')).data.rows).toHaveLength(1);
    expect((await call('/search?accounts=work&q=subject:%22launch%22%20-is:unread')).data.rows.length).toBeGreaterThan(0);
  });

  it("persists operations, rejects stale revisions, and deduplicates retries", async () => {
    const { call, persistence } = fixture();
    const op: Operation = { id: "op", account_id: "work", target_id: "letter-0", expected_revision: 1, change: { kind: "archive" }, status: "pending", revision: 0 };
    expect((await call("/accounts/work/operations", "POST", op)).data.status).toBe("applied");
    expect((await call("/accounts/work/operations", "POST", op)).data.revision).toBe(2);
    expect((await call("/accounts/work/operations", "POST", { ...op, id: "stale" })).status).toBe(409);
    const reloaded = new DemoMailbox(persistence);
    expect((await reloaded.snapshot()).contents["work/letter-0"].message.folder_ids).toEqual(["Archive"]);
    expect((await call("/accounts/work/operations/op")).data.status).toBe("applied");
  });

  it("keeps drafts, uploads, sent copies and scheduled sends local, with cancellation and idempotency", async () => {
    const { box, call, later } = fixture();
    const draft: Draft = { ...(await box.snapshot()).drafts["personal/draft-weekend"], id: "new", revision: 0, attachments: [] };
    const uploaded = await box.fetch(new Request("https://demo.dakiamail.com/api/accounts/personal/attachments", { method: "POST", body: "local attachment", headers: { "x-filename": "notes.txt", "content-type": "text/plain" } }));
    draft.attachments.push(await uploaded.json());
    const saved = (await call("/accounts/personal/drafts/new", "PUT", draft)).data;
    expect(saved.revision).toBe(1);
    const payload = { operation_id: "send-once", draft_id: "new", expected_revision: 1, due_at: null, timezone: "Europe/Tallinn" };
    const sub: Submission = (await call("/accounts/personal/submissions", "POST", payload)).data;
    expect(sub.state).toBe("scheduled");
    expect((await call("/accounts/personal/submissions", "POST", payload)).data.id).toBe(sub.id);
    const cancelled = await call(`/accounts/personal/submissions/${sub.id}/cancel`, "POST");
    expect(cancelled.data.state).toBe("cancelled");
    later(20000); await box.advance();
    expect(Object.values((await box.snapshot()).contents).some(c => c.message.id === `sent-${sub.id}`)).toBe(false);
    const resend = (await call("/accounts/personal/submissions", "POST", { ...payload, operation_id: "new-send", due_at: new Date(Date.now() + 120000).toISOString() })).data;
    later(180000); await box.advance();
    expect((await call(`/accounts/personal/submissions/${resend.id}`)).data.state).toBe("accepted");
    expect((await call(`/accounts/personal/submissions/${resend.id}/cancel`, "POST")).status).toBe(409);
    const sent = (await call("/accounts/personal/messages?folder=Sent")).data.rows;
    expect(sent).toHaveLength(1);
    expect((await call(`/accounts/personal/messages/${sent[0].id}`)).data.attachments).toHaveLength(1);
    const attachment = await box.fetch(new Request(`https://demo.dakiamail.com/api/accounts/personal/attachments/${draft.attachments[0].id}`));
    expect(await attachment.text()).toBe("local attachment");
  });

  it("simulates translation, unsubscribe, cleanup, rules and settings", async () => {
    const { call, box } = fixture();
    expect((await call("/accounts/work/messages/letter-4/translation?language=fr", "POST")).data.status).toBe("succeeded");
    expect((await call("/accounts/work/messages/letter-4/translation?language=fr")).data.text).toContain("new designs");
    expect((await call("/accounts/personal/messages/letter-8/unsubscribe", "POST", {})).data.queued).toBe(false);
    expect((await call("/accounts/personal/messages/letter-8/unsubscribe")).data.attempt_status).toBe("request_accepted");
    const c = (await box.snapshot()).contents["personal/letter-8"].message;
    const cleanup = { id: "cleanup", source_id: c.id, search_complete: true, targets: [{ id: c.id, expected_revision: c.revision }] };
    expect((await call("/accounts/personal/sender-cleanups", "POST", cleanup)).data.applied).toBe(1);
    expect((await call("/accounts/personal/sender-cleanups", "POST", cleanup)).data.applied).toBe(1);
    const rule = (await call("/rules", "POST", { preset: "block_sender", scope: "all_mailboxes", condition: { kind: "sender_is", email: c.sender.email }, actions: ["trash", "suppress_notifications"] })).data;
    expect((await call(`/rules/${rule.id}`, "PATCH", { enabled: false })).data.enabled).toBe(false);
    const settings = (await call("/accounts/work/settings")).data;
    expect((await call("/accounts/work/settings", "PUT", { ...settings, archive_folder: "Projects" })).data.archive_folder).toBe("Projects");
  });

  it("fails closed for all external-service routes, unknown paths, and cross-account lookups", async () => {
    const { call } = fixture();
    for (const [path, method] of [["/accounts", "POST"], ["/accounts/work", "DELETE"], ["/accounts/gmail/start", "POST"], ["/accounts/discover", "POST"], ["/invitations", "POST"], ["/logout", "POST"], ["/push/public-key", "GET"], ["/push/subscriptions", "POST"], ["/releases", "GET"], ["/new-endpoint", "POST"], ["/accounts/work/messages/letter-0/image", "POST"]]) {
      expect((await call(path, method, method === "GET" ? undefined : {})).status).toBeGreaterThanOrEqual(400);
    }
    expect((await call("/accounts/personal/messages/letter-0")).status).toBe(404);
    const request = new Request("https://demo.dakiamail.com/api/accounts", { signal: AbortSignal.abort() });
    await expect(fixture().box.fetch(request)).rejects.toThrow();
  });

  it("recovers incompatible storage, preserves compatible state, and works when storage is denied", async () => {
    const bad = fixture({ version: 99 });
    expect((await bad.box.snapshot()).accounts).toHaveLength(2);
    const box = new DemoMailbox({ read: async () => { throw new Error("denied"); }, write: async () => { throw new Error("denied"); } });
    const snapshot = await box.snapshot();
    expect((await box.snapshot()).user).toBe(snapshot.user);
    expect(snapshot.accounts).toHaveLength(2);
    expect((await fixture().box.snapshot()).user).not.toBe(snapshot.user);
  });
});

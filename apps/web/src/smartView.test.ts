import { describe, it, expect } from "vitest";
import type { Message } from "@dakia/api-types";
import { smartItems, type MailItem } from "./smartView";

const message = (id: string, values: Partial<Message> = {}): Message => ({
  id, account_id: "work", provider_id: id, thread_id: id, folder_ids: ["INBOX"],
  sender: { name: "Synthetic", email: "test@example.invalid" }, to: [],
  subject: id, preview: "", received_at: "2026-10-29T08:00:00Z", unread: true,
  starred: false, category: "People", category_status: "ready", revision: 1, ...values,
});
const ids = (items: MailItem[]) => items.flatMap(item => item.kind === "mail" ? [`${item.message.account_id}/${item.message.id}`] : []);
const labels = (items: MailItem[]) => items.flatMap(item => item.kind === "heading" ? [item.label] : []);

describe("Smart View sections", () => {
  it("puts categorized unread first and all remaining mail in Seen, with no empty headings", () => {
    const rows = [message("read", { unread: false }), message("pending", { category_status: "pending" }),
      message("unknown", { category: null }), message("notification", { category: "Notifications" }), message("person")];
    const items = smartItems(rows, {});
    expect(labels(items)).toEqual(["people", "notifications", "seen"]);
    expect(ids(items)).toEqual(["work/person", "work/notification", "work/read", "work/pending", "work/unknown"]);
    expect(smartItems([], {})).toEqual([]);
  });
  it("normalizes dynamic categories without duplicating messages or crossing account identities", () => {
    const a = message("shared", { category: " Updates " });
    const b = message("shared", { account_id: "personal", category: "updates" });
    const items = smartItems([a, a, b], {});
    expect(labels(items)).toEqual(["updates"]);
    expect(ids(items)).toEqual(["work/shared", "personal/shared"]);
  });
  it("caps every section including Seen at eight, then reveals eight more independently", () => {
    const rows = Array.from({ length: 19 }, (_, i) => message(`p${i}`));
    rows.push(...Array.from({ length: 10 }, (_, i) => message(`r${i}`, { unread: false })));
    const initial = smartItems(rows, {});
    expect(ids(initial)).toHaveLength(16);
    expect(initial.filter(i => i.kind === "more")).toEqual([
      { kind: "more", label: "people" }, { kind: "more", label: "seen" },
    ]);
    expect(ids(smartItems(rows, { people: { limit: 16 } }))).toHaveLength(24);
    expect(ids(smartItems(rows, { people: { limit: 24 }, seen: { limit: 16 } }))).toHaveLength(29);
  });
  it("retains collapsed counts and reveal limit across refresh and appended pages", () => {
    const rows = Array.from({ length: 17 }, (_, i) => message(`${i}`));
    const collapsed = smartItems(rows, { people: { collapsed: true, limit: 16 } });
    expect(collapsed).toEqual([{ kind: "heading", label: "people", count: 17, collapsed: true }]);
    expect(ids(smartItems(rows, { people: { collapsed: false, limit: 16 } }))).toHaveLength(16);
  });
  it("moves read/unread and newly classified messages once without retaining empty categories", () => {
    const original = message("one");
    expect(labels(smartItems([original], {}))).toEqual(["people"]);
    const read = { ...original, unread: false };
    expect(labels(smartItems([read], {}))).toEqual(["seen"]);
    expect(ids(smartItems([read], {}))).toEqual(["work/one"]);
    expect(labels(smartItems([{ ...read, unread: true }], {}))).toEqual(["people"]);
    expect(labels(smartItems([{ ...original, category_status: "pending" }], {}))).toEqual(["seen"]);
    expect(labels(smartItems([original], { seen: { collapsed: true } }))).toEqual(["people"]);
  });
});

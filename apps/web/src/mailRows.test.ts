import { describe, expect, it } from "vitest";
import type { Message } from "@dakia/api-types";
import { mailRows } from "./mailRows";
import { matchesClassification } from "./classification";

const message = (id: string, received_at: string, values: Partial<Message> = {}): Message => ({
  id, account_id: "work", provider_id: id, thread_id: id, folder_ids: ["INBOX"],
  sender: { name: "Synthetic", email: "fixture@example.invalid" }, to: [], subject: id, preview: "",
  received_at, unread: false, starred: false, category: "people", category_status: "ready", revision: 1, ...values,
});
const keys = (rows: Message[]) => rows.map(m => `${m.account_id}/${m.id}`);
const personal = (rows: Message[]) => rows.filter(m => matchesClassification(m, { inbox_kind: "personal" }));

describe("loaded mail ordering", () => {
  it("reconciles previews without stale duplicates, dropping provisional-only matches only on complete coverage", () => {
    const stale = message("stale", "2026-10-29T08:00:00Z");
    const old = message("same", "2026-10-29T08:00:00Z");
    const live = { ...old, subject: "Fresh provider result", revision: 2 };
    const preview = { rows: [stale, old], cache_preview: true, coverage: [{ state: "pending" }] };
    for (const state of ["pending", "partial", "error"]) {
      const rows = mailRows([preview, { rows: [live], coverage: [{ state }] }, { rows: [] }], {});
      expect(rows).toHaveLength(2);
      expect(rows.find(m => m.id === "same")).toEqual(live);
    }
    expect(mailRows([preview, { rows: [live], coverage: [{ state: "complete" }] }, { rows: [] }], {})).toEqual([live]);
    expect(mailRows([preview], {})).toEqual([stale, old]);
  });
  it("merges three interleaved account pages by actual instant without read-status sections", () => {
    const work = Array.from({ length: 12 }, (_, i) => message(`w${i}`, `2026-09-${String(30 - i).padStart(2, "0")}T12:00:00Z`));
    const personalRows = Array.from({ length: 12 }, (_, i) => message(`p${i}`, `2026-10-${String(29 - i).padStart(2, "0")}T12:00:00Z`, { account_id: "personal" }));
    const pages = [0, 4, 8].map(start => ({ rows: [...personalRows.slice(start, start + 4), ...work.slice(start, start + 4)] }));
    pages[1].rows.push(personalRows[0]);
    const original = JSON.stringify(pages);
    const merged = mailRows(pages, {});
    expect(keys(merged)).toEqual(keys([...personalRows, ...work]));
    expect(keys(personal(merged))).toEqual(keys(merged));
    expect(personal(merged).map(m => m.id)).toEqual([...personalRows, ...work].map(m => m.id));
    expect(JSON.stringify(pages)).toBe(original);
  });

  it("handles timezone offsets, fractional seconds, stable ties, invalid dates and same IDs in different accounts", () => {
    const a = message("same", "2026-10-29T10:30:00+03:00");
    const b = message("same", "2026-10-29T08:00:00Z", { account_id: "personal" });
    const tie = message("tie", "2026-10-29T10:00:00+02:00");
    const fraction = message("fraction", "2026-10-29T08:00:00.001Z");
    const invalid = message("invalid", "not-a-date");
    expect(keys(mailRows([{ rows: [invalid, a, b] }, { rows: [tie, fraction, a] }], {})))
      .toEqual(keys([fraction, b, tie, a, invalid]));
  });

  it("re-sorts refreshed pages and read transitions without changing classification, identity or pagination", () => {
    const old = message("old", "2026-09-01T08:00:00Z");
    const unread = message("new", "2026-10-29T08:00:00Z", { unread: true });
    const pages = [{ rows: [unread, old], continuation: "opaque-unchanged" }];
    expect(keys(personal(mailRows(pages, {})))).toEqual(["work/new", "work/old"]);
    const overrides = { "work/new": { ...unread, unread: false, revision: 2 } };
    expect(keys(personal(mailRows(pages, overrides)))).toEqual(["work/new", "work/old"]);
    expect(keys(personal(mailRows(pages, { "work/new": unread })))).toEqual(["work/new", "work/old"]);
    const arrival = message("arrival", "2026-10-30T08:00:00Z", { account_id: "personal", category_status: "pending", unread: true });
    const refreshed = [{ rows: [old, arrival] }, { rows: [unread, old] }];
    expect(keys(personal(mailRows(refreshed, overrides)))).toEqual(["personal/arrival", "work/new", "work/old"]);
    expect(pages[0].continuation).toBe("opaque-unchanged");
    expect(unread.unread).toBe(true);
  });
});

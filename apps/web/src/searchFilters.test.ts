import { expect, test } from "vitest";
import { mergeSearch, readSearch, writeSearch } from "./searchFilters";

test("supported filters round-trip quoted values without changing literal text", () => {
  const filters = [{ key: "from" as const, value: 'Alex "A" \\ Morgan' }, { key: "folder" as const, value: "Sent Mail" }];
  expect(readSearch(writeSearch('"launch plan" café 東京', filters))).toEqual({ text: '"launch plan" café 東京', filters });
});
test("last duplicate wins like the server; since normalizes to after", () => {
  expect(readSearch("from:first from:second since:2026-11-01 is:unread launch")).toEqual({ text: "launch", filters: [
    { key: "from", value: "second" }, { key: "after", value: "2026-11-01" }, { key: "is", value: "unread" },
  ] });
});
test("unsupported operators and malformed values stay intact for server validation", () => {
  for (const query of ['has:attachment is:unknown from: "unterminated', '"from:someone', 'from:"unterminated', "literal:word"]) {
    expect(readSearch(query)).toEqual({ text: query, filters: [] });
  }
});

test("a field value cannot inject another search operator", () => {
  const filters = [{ key: "from" as const, value: "sender@example.org is:starred" }];
  expect(writeSearch("launch", filters)).toBe('launch from:"sender@example.org is:starred"');
  expect(readSearch(writeSearch("launch", filters))).toEqual({ text: "launch", filters });
});

test("typed operators replace an existing chip instead of retaining the older value", () => {
  expect(mergeSearch('subject:"New phrase"', [{ key: "subject", value: "Old phrase" }, { key: "is", value: "unread" }])).toBe('is:unread subject:"New phrase"');
});

import { expect, test } from "vitest";
import type { Message } from "@dakia/api-types";
import { threadCountBatches } from "./useThreadCounts";

test("thread count batches deduplicate IDs, isolate accounts and cap request sizes", () => {
  const rows = Array.from({ length: 51 }, (_, i) => ({ account_id: "work", id: String(i) } as Message));
  const batches = threadCountBatches([...rows, rows[0], { ...rows[0], account_id: "personal" }]);
  expect(batches.map(batch => [batch.account, batch.ids.length])).toEqual([["work", 50], ["work", 1], ["personal", 1]]);
  expect(threadCountBatches([])).toEqual([]);
});

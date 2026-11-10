import { expect, it } from "vitest";
import { snoozeTime } from "./snoozeTime";
it("labels local calendar days and preserves a precise time", () => {
  const now = new Date(2026, 10, 10, 10);
  expect(snoozeTime(new Date(2026,10,10,15,30).toISOString(), now)).toMatch(/^today at .*30/);
  expect(snoozeTime(new Date(2026,10,11,9).toISOString(), now)).toMatch(/^tomorrow at /);
  expect(snoozeTime(new Date(2026,10,13,9).toISOString(), now)).toMatch(/^Friday at /);
  expect(snoozeTime(new Date(2027,0,3,9).toISOString(), now)).toContain("2027");
  expect(snoozeTime("invalid", now)).toBe("");
});

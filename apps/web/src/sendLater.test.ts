import { describe, expect, it } from "vitest";
import { localDateTime, sendLaterPresets, validSendTime } from "./sendLater";

describe("send later", () => {
  it("keeps presets future-only and omits elapsed evening choices", () => {
    const morning = new Date(2026, 9, 29, 10, 15);
    const choices = sendLaterPresets(morning);
    expect(choices.map((c) => c.label)).toEqual(["Later today", "This evening", "Tomorrow"]);
    expect(localDateTime(choices[0].date)).toBe("2026-10-29T12:00");
    const late = sendLaterPresets(new Date(2026, 9, 29, 23, 30));
    expect(late.map((c) => c.label)).toEqual(["Tomorrow"]);
    expect(localDateTime(late[0].date)).toBe("2026-10-30T09:00");
  });
  it("handles year boundaries using local calendar days", () => {
    const choices = sendLaterPresets(new Date(2026, 11, 31, 23));
    expect(localDateTime(choices[0].date)).toBe("2027-01-01T09:00");
  });
  it("rejects missing, malformed, impossible and elapsed values", () => {
    const now = new Date(2026, 9, 29, 10).getTime();
    for (const value of ["", "bad", "2026-02-30T09:00", "2026-10-29T09:59", "2026-10-29T10:00"])
      expect(validSendTime(value, now)).toBe(false);
    expect(validSendTime("2026-10-29T10:01", now)).toBe(true);
  });
  it.skipIf(Intl.DateTimeFormat().resolvedOptions().timeZone !== "Europe/Tallinn")("rejects the spring clock gap and keeps tomorrow at nine across DST", () => {
    const now = new Date(2027, 2, 27, 23);
    expect(validSendTime("2027-03-28T03:30", now.getTime())).toBe(false);
    const tomorrow = sendLaterPresets(now)[0].date;
    expect(localDateTime(tomorrow)).toBe("2027-03-28T09:00");
    expect(tomorrow.toISOString()).toBe("2027-03-28T06:00:00.000Z");
  });
});

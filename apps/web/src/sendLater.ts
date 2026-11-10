export { localDateTime, dateTimePresets as sendLaterPresets, validFutureDateTime as validSendTime } from "@dakia/ui";

export function sendTimeLabel(value: string) {
  return new Date(value).toLocaleString(undefined, {
    month: "short", day: "numeric", hour: "numeric", minute: "2-digit",
  });
}

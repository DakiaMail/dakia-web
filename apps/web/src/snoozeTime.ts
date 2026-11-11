/** Calendar-day labels use the reader's local timezone, including DST changes. */
export function snoozeTime(value: string, now = new Date()): string {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return "";
  const sameDay = (a: Date, b: Date) => a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
  const tomorrow = new Date(now); tomorrow.setDate(tomorrow.getDate() + 1);
  const week = new Date(now); week.setDate(week.getDate() + 7);
  const day = sameDay(date, now) ? "today" : sameDay(date, tomorrow) ? "tomorrow"
    : date > now && date < week ? date.toLocaleDateString(undefined, { weekday: "long" })
    : date.toLocaleDateString(undefined, { month: "short", day: "numeric", ...(date.getFullYear() !== now.getFullYear() ? { year: "numeric" } : {}) });
  return `${day} at ${date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}`;
}

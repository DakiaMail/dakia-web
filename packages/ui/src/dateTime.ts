export function localDateTime(date: Date) {
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}T${String(date.getHours()).padStart(2, "0")}:${String(date.getMinutes()).padStart(2, "0")}`;
}

export function validFutureDateTime(value: string, now = Date.now()) {
  const date = new Date(value);
  return Number.isFinite(date.getTime()) && date.getTime() > now && localDateTime(date) === value;
}

export function dateTimePresets(now = new Date()) {
  const later = new Date(now);
  later.setHours(later.getHours() + 2, 0, 0, 0);
  const evening = new Date(now);
  evening.setHours(20, 0, 0, 0);
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  tomorrow.setHours(9, 0, 0, 0);
  return [
    ...(later.getDate() === now.getDate() && later < evening ? [{ label: "Later today", date: later }] : []),
    ...(evening > now ? [{ label: "This evening", date: evening }] : []),
    { label: "Tomorrow", date: tomorrow },
  ];
}


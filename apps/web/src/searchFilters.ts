export const filterFields = [
  { key: "from", label: "From", type: "text" },
  { key: "to", label: "To", type: "text" },
  { key: "subject", label: "Subject", type: "text" },
  { key: "folder", label: "Folder", type: "text" },
  { key: "is", label: "Status", type: "status" },
  { key: "after", label: "Since", type: "date" },
  { key: "before", label: "Before", type: "date" },
] as const;
export type SearchFilter = { key: typeof filterFields[number]["key"]; value: string };
export function readSearch(query: string): { text: string; filters: SearchFilter[] } {
  const filters: SearchFilter[] = [], text: string[] = [];
  if (query.replace(/"(?:[^"\\]|\\.)*"/g, "").includes('"')) return { text: query, filters };
  // Keep raw text intact, including malformed quotes, so server validation still owns the query.
  const terms = query.match(/(?:[^\s"\\]|\\[^\s]|"(?:[^"\\]|\\.)*")+|\S+/g) || [];
  for (const raw of terms) {
    const decoded = raw.replace(/"((?:[^"\\]|\\.)*)"/g, (_, value: string) => value.replace(/\\(.)/g, "$1"));
    const colon = decoded.indexOf(":"), key = decoded.slice(0, colon) === "since" ? "after" : decoded.slice(0, colon), value = decoded.slice(colon + 1);
    const field = filterFields.find(field => field.key === key);
    if (colon < 0 || !field || !value || (field.type === "status" && !["unread", "read", "starred"].includes(value))) { text.push(raw); continue; }
    const existing = filters.findIndex(filter => filter.key === key);
    if (existing >= 0) filters.splice(existing, 1);
    filters.push({ key: field.key, value });
  }
  return { text: text.join(" "), filters };
}
export function writeSearch(text: string, filters: SearchFilter[]): string {
  return [text.trim(), ...filters.map(({ key, value }) => `${key}:${/[\s"\\]/.test(value) ? `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"` : value}`)].filter(Boolean).join(" ");
}
export function filterLabel(filter: SearchFilter): string {
  const field = filterFields.find(field => field.key === filter.key)!;
  return filter.key === "is" ? filter.value[0].toUpperCase() + filter.value.slice(1) : `${field.label} ${filter.value}`;
}
export function mergeSearch(text: string, filters: SearchFilter[]): string {
  const typed = readSearch(text);
  return writeSearch(typed.text, [...filters.filter(filter => !typed.filters.some(item => item.key === filter.key)), ...typed.filters]);
}

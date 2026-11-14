import type { ReactNode } from "react";

export interface FilterChipProps { label: string; onEdit: () => void; onRemove: () => void; removeIcon: ReactNode; }
export function FilterChip({ label, onEdit, onRemove, removeIcon }: FilterChipProps) {
  return <span className="dakia-filter-chip"><button type="button" aria-label={`Edit ${label} filter`} onClick={onEdit}>{label}</button>
    <button type="button" aria-label={`Remove ${label} filter`} onClick={onRemove}>{removeIcon}</button></span>;
}

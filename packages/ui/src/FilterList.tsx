import { useEffect, useRef, useState, type ReactNode } from "react";

export interface FilterListItem { value: string; label: string; icon?: ReactNode; detail?: string; }
export interface FilterListProps {
  items: FilterListItem[];
  onSelect: (value: string) => void;
  label: string;
  searchLabel: string;
  searchIcon: ReactNode;
  trailingIcon: ReactNode;
  closeControl: ReactNode;
}
/** Searchable property picker. Arrows navigate rows; Tab keeps its normal direction. */
export function FilterList({ items, onSelect, label, searchLabel, searchIcon, trailingIcon, closeControl }: FilterListProps) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const frame = requestAnimationFrame(() => input.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, []);
  const filtered = items.filter(item => item.label.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase()));
  function focus(index: number) {
    setActive(index); root.current?.querySelectorAll<HTMLButtonElement>("[data-filter-option]")[index]?.focus();
  }
  return <div ref={root} className="dakia-filter-list">
    <div className="dakia-filter-list-search">{searchIcon}<input ref={input} data-autofocus type="text" aria-label={searchLabel} placeholder="Search filters…" value={query}
      onChange={event => { setQuery(event.target.value); setActive(0); }}
      onKeyDown={event => {
        if (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
        if (filtered.length && event.key === "Enter" && !event.repeat) { event.preventDefault(); event.stopPropagation(); onSelect(filtered[0].value); }
        else if (filtered.length && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
          event.preventDefault(); event.stopPropagation(); focus(event.key === "ArrowDown" ? 0 : filtered.length - 1);
        }
      }} />{closeControl}</div>
    <div role="group" aria-label={label} className="dakia-filter-list-options" onKeyDown={event => {
      if (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
      if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key) || !filtered.length) return;
      event.preventDefault(); event.stopPropagation();
      focus(event.key === "Home" ? 0 : event.key === "End" ? filtered.length - 1 : (active + (event.key === "ArrowDown" ? 1 : filtered.length - 1)) % filtered.length);
    }}>
      {filtered.map((item, index) => <button data-filter-option key={item.value} type="button" tabIndex={index === Math.min(active, filtered.length - 1) ? 0 : -1}
        onFocus={() => setActive(index)} onClick={() => onSelect(item.value)}>
        {item.icon}<span>{item.label}</span>{item.detail && <small>{item.detail}</small>}{trailingIcon}
      </button>)}
      {!filtered.length && <p role="status">No matching filters.</p>}
    </div>
  </div>;
}

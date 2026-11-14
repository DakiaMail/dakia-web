import { forwardRef, useId, useRef, useState, type ReactNode } from "react";

export interface SearchOption {
  id: string;
  label: string;
  description?: string;
  detail?: string;
  group?: string;
  icon?: ReactNode;
  onSelect: () => void;
}
export interface SearchFieldProps {
  value: string;
  onChange: (value: string) => void;
  onSearch: () => void;
  onDismiss?: () => void;
  label: string;
  placeholder?: string;
  searchActionLabel?: string;
  icon: ReactNode;
  clearIcon: ReactNode;
  options?: SearchOption[];
  loading?: boolean;
  status?: string;
  shortcutHint?: ReactNode;
  className?: string;
}

/** Editable combobox: focus stays in the input; only explicit arrow navigation selects a match. */
export const SearchField = forwardRef<HTMLInputElement, SearchFieldProps>(function SearchField({
  value, onChange, onSearch, onDismiss, label, placeholder = label, searchActionLabel, icon, clearIcon, options = [], loading, status, shortcutHint, className = "",
}, ref) {
  const id = useId();
  const input = useRef<HTMLInputElement | null>(null);
  const [opened, setOpened] = useState(false);
  const [selection, setSelection] = useState<{ id: string; value: string }>();
  const active = selection?.value === value ? options.findIndex(option => option.id === selection.id) : -1;
  const expanded = opened && !!value.trim();
  function highlight(label: string) {
    const term = value.trim(), start = label.toLocaleLowerCase().indexOf(term.toLocaleLowerCase());
    return !term || start < 0 ? label : <>{label.slice(0, start)}<mark>{label.slice(start, start + term.length)}</mark>{label.slice(start + term.length)}</>;
  }
  function submit() { setOpened(false); setSelection(undefined); onSearch(); }
  return <form className={`dakia-search-field ${className}`} role="search" aria-label={label}
    onSubmit={event => { event.preventDefault(); submit(); }}
    onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget)) { setOpened(false); setSelection(undefined); } }}>
    <div className="dakia-search-control">
      <button type="submit" aria-label="Search" className="dakia-search-icon">{icon}</button>
      <input ref={node => { input.current = node; if (typeof ref === "function") ref(node); else if (ref) ref.current = node; }}
        role="combobox" type="text" aria-label={label} placeholder={placeholder} autoComplete="off" spellCheck={false}
        aria-autocomplete="list" aria-expanded={expanded} aria-controls={expanded ? `${id}-list` : undefined}
        aria-activedescendant={expanded && active >= 0 ? `${id}-${active}` : undefined}
        value={value} onFocus={() => setOpened(true)} onClick={() => setOpened(true)}
        onChange={event => { onChange(event.target.value); setSelection(undefined); setOpened(true); }}
        onKeyDown={event => {
          if (event.key === "Tab" && !event.ctrlKey && !event.metaKey && !event.altKey && !event.nativeEvent.isComposing) {
            setOpened(false); setSelection(undefined); return;
          }
          if (event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229 || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
          if (event.key === "Escape") {
            event.preventDefault(); event.stopPropagation();
            if (expanded) { setOpened(false); setSelection(undefined); } else if (onDismiss) onDismiss(); else input.current?.blur();
          } else if ((event.key === "ArrowDown" || event.key === "ArrowUp") && value.trim() && options.length) {
            event.preventDefault(); event.stopPropagation(); setOpened(true);
            const next = active < 0 ? event.key === "ArrowDown" ? 0 : options.length - 1 : (active + (event.key === "ArrowDown" ? 1 : options.length - 1)) % options.length;
            setSelection({ id: options[next].id, value });
            requestAnimationFrame(() => document.getElementById(`${id}-${next}`)?.scrollIntoView({ block: "nearest" }));
          } else if (event.key === "Enter") {
            event.preventDefault(); event.stopPropagation();
            if (expanded && active >= 0) { options[active].onSelect(); setOpened(false); setSelection(undefined); } else submit();
          }
        }} />
      {value && <button type="button" aria-label="Clear search text" className="dakia-search-clear" onClick={() => { onChange(""); setSelection(undefined); input.current?.focus(); }}>{clearIcon}</button>}
      {!value && shortcutHint}
    </div>
    {expanded && <div className="dakia-search-popup">
      <div id={`${id}-list`} role="listbox" aria-label="Search suggestions" aria-busy={loading || undefined}>
        <div role="option" aria-selected={false} className="dakia-search-option dakia-search-all" onPointerDown={event => event.preventDefault()} onClick={submit}>
          {icon}<span>{searchActionLabel || `Search for “${value}”`}</span><kbd aria-hidden="true">Enter</kbd>
        </div>
        {options.map((option, index) => <div key={option.id} role="presentation">
          {option.group && option.group !== options[index - 1]?.group && <div className="dakia-search-group" aria-hidden="true">{option.group}</div>}
          <div id={`${id}-${index}`} role="option" aria-selected={active === index} aria-label={[option.label, option.description, option.detail].filter(Boolean).join(", ")}
            className="dakia-search-option" onPointerDown={event => event.preventDefault()} onClick={() => { option.onSelect(); setOpened(false); setSelection(undefined); }}>
            {option.icon}<span className="dakia-search-option-text"><span>{highlight(option.label)}</span>{option.description && <small>{option.description}</small>}</span>
            {option.detail && <small className="dakia-search-detail">{option.detail}</small>}
          </div>
        </div>)}
      </div>
      <div className="dakia-search-footer" role="status">{loading ? "Searching…" : status || `↑ ↓ Navigate · Enter ${active >= 0 ? "Open" : "Search"} · Esc Close`}</div>
    </div>}
  </form>;
});

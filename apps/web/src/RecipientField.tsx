import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { extraApi } from "./extra-api";
import { plainKey } from "./keyboard";
export function RecipientField({
  account,
  label,
  value,
  disabled,
  onChange,
}: {
  account: string;
  label: string;
  value: string;
  disabled: boolean;
  onChange: (v: string) => void;
}) {
  const [search, setSearch] = useState("");
  const [open, setOpen] = useState(false);
  const [highlight, setHighlight] = useState(0);
  const term = value.split(/[,;]/).at(-1)?.trim() || "";
  useEffect(() => {
    const t = setTimeout(() => setSearch(term), 200);
    return () => clearTimeout(t);
  }, [term]);
  const suggestions = useQuery({
    queryKey: ["recipients", account, search],
    queryFn: () => extraApi.recipients(account, search),
    enabled: open && search.length >= 2,
    staleTime: 30000,
    retry: false,
  });
  const options = suggestions.data || [];
  function choose(email: string) {
    const parts = value.split(/[,;]/);
    parts.pop();
    onChange(
      [...parts.map((p) => p.trim()).filter(Boolean), email].join(", ") + ", ",
    );
    setOpen(false);
    setHighlight(0);
  }
  return (
    <span className="recipient-field">
      <input
        aria-label={label}
        value={value}
        disabled={disabled}
        autoComplete="off"
        aria-autocomplete="list"
        aria-expanded={open && options.length > 0}
        aria-controls={`recipients-${label}`}
        aria-activedescendant={
          open && options[highlight]
            ? `recipient-${label}-${highlight}`
            : undefined
        }
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            // The dialog may already have vetoed its own dismissal in capture.
            if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey || e.nativeEvent.isComposing || e.nativeEvent.keyCode === 229) return;
            setOpen(false);
            e.stopPropagation();
            return;
          }
          if (!plainKey(e.nativeEvent)) return;
          if (!open || !options.length) return;
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            setHighlight(
              (i) =>
                (i + (e.key === "ArrowDown" ? 1 : options.length - 1)) %
                options.length,
            );
          }
          if (e.key === "Enter") {
            e.preventDefault();
            choose(options[highlight % options.length].email);
          }
        }}
        onFocus={() => setOpen(true)}
        onBlur={() => setTimeout(() => setOpen(false), 150)}
        onChange={(e) => onChange(e.target.value)}
      />
      {open && options.length > 0 && (
        <span
          id={`recipients-${label}`}
          className="recipient-options"
          role="listbox"
          aria-label={`${label} suggestions`}
        >
          {options.map((a, index) => (
            <button title={`Add ${a.email}`}
              key={a.email}
              type="button"
              role="option"
              id={`recipient-${label}-${index}`}
              aria-selected={highlight === index}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => choose(a.email)}
            >
              {a.name && <b>{a.name}</b>} {a.email}
            </button>
          ))}
        </span>
      )}
    </span>
  );
}

import { useEffect, useRef, useState, type ReactNode } from "react";
import { Button, FilterChip, FilterPopover, IconButton, SearchField, type SearchOption } from "@dakia/ui";
import { ArrowLeft, Mail, Plus, Search, SlidersHorizontal, UserRound, X } from "lucide-react";
import type { Account, Message } from "@dakia/api-types";
import { KeyHint } from "./KeyboardHints";
import { useSearchSuggestions } from "./SearchSuggestions";
import { filterFields, filterLabel, mergeSearch, readSearch, type SearchFilter } from "./searchFilters";

export function MailSearch({ value, committed, ids, accounts, onChange, onSearch, onOpen, onBack, accountControl, actions, backLabel }: {
  value: string; committed: string; ids: string[]; accounts: Account[];
  accountControl: ReactNode; actions: ReactNode; backLabel: string;
  onChange: (value: string) => void; onSearch: (value: string) => void; onOpen: (message: Message) => void; onBack: () => void;
}) {
  const initial = readSearch(value);
  const [text, setText] = useState(initial.text);
  const [filters, setFilters] = useState(initial.filters);
  const emitted = useRef(value);
  const input = useRef<HTMLInputElement>(null);
  const filterButton = useRef<HTMLButtonElement>(null);
  const [editing, setEditing] = useState(false);
  const [editingExisting, setEditingExisting] = useState(false);
  const [candidate, setCandidate] = useState<SearchFilter>({ key: "from", value: "" });
  const [filterSearch, setFilterSearch] = useState("");
  const [suggesting, setSuggesting] = useState(false);
  useEffect(() => {
    if (value === emitted.current) return;
    emitted.current = value;
    const parsed = readSearch(value); setText(parsed.text); setFilters(parsed.filters);
  }, [value]);
  function publish(nextText: string, nextFilters = filters) {
    const query = mergeSearch(nextText, nextFilters);
    emitted.current = query; setText(nextText); setFilters(nextFilters); onChange(query);
    return query;
  }
  function submit(nextText = text, nextFilters = filters) {
    const parsed = readSearch(mergeSearch(nextText, nextFilters));
    onSearch(publish(parsed.text, parsed.filters)); setSuggesting(false);
  }
  const query = mergeSearch(text, filters);
  function applyFilter(filter: SearchFilter) {
    const draft = readSearch(query);
    submit(draft.text, [...draft.filters.filter(item => item.key !== filter.key), filter]);
  }
  const suggestions = useSearchSuggestions(query, ids, suggesting && !!text.trim());
  const senders = [...new Map(suggestions.rows.map(message => [message.sender.email, message.sender])).values()].slice(0, 2);
  const options: SearchOption[] = [
    ...senders.map(sender => ({ id: `sender:${sender.email}`, label: sender.name || sender.email, description: sender.email,
      group: "People in matching mail", icon: <UserRound size={16} />, onSelect: () => {
        applyFilter({ key: "from", value: sender.email }); input.current?.focus();
      } })),
    ...suggestions.rows.slice(0, 5).map(message => ({ id: `message:${message.account_id}/${message.id}`, label: message.subject || "(No subject)",
      description: `${message.sender.name || message.sender.email} · ${accounts.find(account => account.id === message.account_id)?.display_name || ""}`,
      detail: new Date(message.received_at).toLocaleDateString(undefined, { month: "short", day: "numeric" }),
      group: "Messages", icon: <Mail size={16} />, onSelect: () => { setSuggesting(false); onOpen(message); } })),
  ];
  function edit(filter?: SearchFilter) {
    setCandidate(filter || { key: "from", value: "" }); setFilterSearch(""); setEditingExisting(!!filter); setSuggesting(false); setEditing(true);
  }
  const field = filterFields.find(field => field.key === candidate.key)!;
  return <>
    <div className="mail-search-input" onFocus={() => setSuggesting(true)} onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget)) setSuggesting(false); }}>
      <SearchField ref={input} className={`search ${committed ? "results-search" : ""}`} label="Search all mail" placeholder="Search mail" searchActionLabel={`Search all mail for “${text}”`}
        onDismiss={() => document.querySelector<HTMLElement>(".mailbox")?.focus()}
        value={text} onChange={next => { publish(next); setSuggesting(true); }} onSearch={() => submit()}
        icon={<Search size={17} />} clearIcon={<X size={15} />} shortcutHint={<KeyHint keys="/" />}
        options={options} loading={suggestions.loading}
        status={suggestions.error ? "Quick matches unavailable. Press Enter to search all mail." : !suggestions.rows.length ? "Press Enter to search all mail." : undefined} />
    </div>
    {accountControl}
    <FilterPopover opened={editing} onChange={setEditing} label="Search filters" trigger={<Button ref={filterButton} className="mail-search-filter-trigger" variant="tertiary" aria-label="Search filters" aria-expanded={editing} onClick={() => editing ? setEditing(false) : edit()} leftSection={<SlidersHorizontal size={16} />}>
      <span>Filters{filters.length ? ` · ${filters.length}` : ""}</span>
    </Button>}>
      <form className="mail-search-filter-form" onSubmit={event => {
        event.preventDefault();
        if (!candidate.value.trim()) return;
        applyFilter({ ...candidate, value: candidate.value.trim() }); setEditing(false);
      }}>
        <header><strong>Search filters</strong><IconButton label="Close search filters" onClick={() => setEditing(false)}><X size={16} /></IconButton></header>
        <label>Find a filter<input data-autofocus={!editingExisting || undefined} value={filterSearch} placeholder="Sender, date, status…" onChange={event => setFilterSearch(event.target.value)} /></label>
        <div className="mail-search-filter-fields" role="group" aria-label="Filter type">
          {filterFields.filter(field => field.label.toLowerCase().includes(filterSearch.toLowerCase())).map(field => <button key={field.key} type="button" aria-pressed={candidate.key === field.key} onClick={() => setCandidate({ key: field.key, value: field.type === "status" ? "unread" : "" })}>{field.label}</button>)}
          {!filterFields.some(field => field.label.toLowerCase().includes(filterSearch.toLowerCase())) && <p role="status">No matching filters.</p>}
        </div>
        <label>{field.label}{field.type === "status" ? <select data-autofocus={editingExisting || undefined} aria-label="Status" value={candidate.value || "unread"} onChange={event => setCandidate({ ...candidate, value: event.target.value })}>
          <option value="unread">Unread</option><option value="read">Read</option><option value="starred">Starred</option>
        </select> : <input data-autofocus={editingExisting || undefined} aria-label={field.label} type={field.type} required value={candidate.value} maxLength={512} onChange={event => setCandidate({ ...candidate, value: event.target.value })} />}</label>
        <Button type="submit" disabled={!candidate.value.trim()}>Apply filter</Button>
      </form>
    </FilterPopover>
    {actions}
    {(filters.length > 0 || !!committed) && <div className="mail-search-refinements" role="group" aria-label="Applied search filters">
      {committed && <Button variant="tertiary" className="mail-search-back" onClick={onBack} leftSection={<ArrowLeft size={14} />}>{backLabel}</Button>}
      {filters.map(filter => <FilterChip key={filter.key} label={filterLabel(filter)} onEdit={() => edit(filter)} removeIcon={<X size={12} />} onRemove={() => {
        submit(text, filters.filter(item => item.key !== filter.key)); requestAnimationFrame(() => filterButton.current?.focus());
      }} />)}
      {filters.length > 0 && <>
        <Button variant="tertiary" onClick={() => edit()} leftSection={<Plus size={13} />}>Add filter</Button>
        <Button variant="tertiary" onClick={() => { submit(text, []); requestAnimationFrame(() => filterButton.current?.focus()); }}>Clear filters</Button>
      </>}
    </div>}
  </>;
}

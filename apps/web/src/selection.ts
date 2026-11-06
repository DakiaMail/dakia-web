export type MailSelection = {
  scope: string;
  view: string;
  checked: Set<string>;
  anchor: string | null;
};

export function emptySelection(scope: string, view: string): MailSelection {
  return { scope, view, checked: new Set(), anchor: null };
}

// Keep only currently listed mail. A new mailbox/filter clears selection; a
// presentation change keeps checkmarks but starts a fresh range anchor.
export function reconcileSelection(
  state: MailSelection, order: readonly string[], scope: string, view: string,
): MailSelection {
  if (state.scope !== scope) return emptySelection(scope, view);
  const available = new Set(order);
  const checked = new Set([...state.checked].filter(id => available.has(id)));
  const anchor = state.view === view && state.anchor && available.has(state.anchor)
    ? state.anchor : null;
  if (checked.size === state.checked.size && anchor === state.anchor && state.view === view) return state;
  return { scope, view, checked, anchor };
}

export function selectMail(
  state: MailSelection, order: readonly string[], target: string,
  range: boolean, value: boolean,
): MailSelection {
  const end = order.indexOf(target);
  if (end < 0) return state;
  const start = range && state.anchor ? order.indexOf(state.anchor) : -1;
  const affected = start < 0 ? [target] : order.slice(Math.min(start, end), Math.max(start, end) + 1);
  const checked = new Set(state.checked);
  for (const id of affected) value ? checked.add(id) : checked.delete(id);
  return { ...state, checked, anchor: start < 0 ? target : state.anchor };
}

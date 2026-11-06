import { useLayoutEffect, useRef, type RefObject } from "react";

// Reveal finite cached sections near the viewport. Provider I/O has a separate
// budget: one page on entry, then at most one per downward scroll. Layout,
// background refresh and empty pages cannot drain the mailbox. Native buttons
// remain available when a short page cannot scroll further.
export function useMailInfiniteLoading(
  root: RefObject<HTMLDivElement | null>, scope: string, enabled: boolean,
  load: (section: string) => void,
) {
  const state = useRef({ scope, allowed: true, top: 0 });
  if (state.current.scope !== scope) state.current = { scope, allowed: true, top: 0 };

  useLayoutEffect(() => {
    const frame = requestAnimationFrame(() => {
      const element = root.current;
      if (!enabled || !element?.clientHeight) return;
      const bounds = element.getBoundingClientRect();
      const target = Array.from(element.querySelectorAll<HTMLButtonElement>("button[data-mail-more]"))
        .find(button => {
          const rect = button.getBoundingClientRect();
          return !button.disabled && button !== document.activeElement
            && (button.dataset.mailMore !== "" || state.current.allowed)
            && rect.bottom >= bounds.top && rect.top <= bounds.bottom + 120;
        });
      if (!target) return;
      if (target.dataset.mailMore === "") state.current.allowed = false;
      load(target.dataset.mailMore!);
    });
    return () => cancelAnimationFrame(frame);
  });

  // Use React's delegated event, not a native listener removed during the
  // virtualizer's synchronous scroll render before event dispatch finishes.
  return () => {
    const element = root.current;
    if (!element?.clientHeight) return;
    if (element.scrollTop > state.current.top + 1) state.current.allowed = true;
    state.current.top = element.scrollTop;
  };
}

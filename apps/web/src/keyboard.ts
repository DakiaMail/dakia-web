// Component shortcuts must not consume browser/OS chords or IME input.
// Shift is allowed only for explicitly documented shortcuts.
export function plainKey(event: {
  defaultPrevented: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  isComposing?: boolean;
  keyCode?: number;
}, allowShift = false): boolean {
  return !event.defaultPrevented && !event.ctrlKey && !event.metaKey &&
    !event.altKey && (allowShift || !event.shiftKey) &&
    !event.isComposing && event.keyCode !== 229;
}

export function canDismissMail(event: KeyboardEvent): boolean {
  if (event.key !== "Escape" || !plainKey(event) || event.repeat) return false;
  // Native controls, embedded editors and dialogs own their Escape behavior.
  if (!(event.target instanceof Element) || event.target.closest(
    'input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="combobox"],[role="dialog"],[role="alertdialog"],dialog',
  )) return false;
  if (document.querySelector('[role="dialog"],[role="alertdialog"],dialog[open]')) return false;
  return !window.getSelection()?.toString();
}

// Global mail actions leave editing, native widgets, popovers and selected text alone.
export function canUseMailShortcut(event: KeyboardEvent): boolean {
  if (!plainKey(event, true) || event.repeat || !(event.target instanceof Element)) return false;
  if (event.target.closest('input,textarea,select,[contenteditable]:not([contenteditable="false"]),[role="textbox"],[role="combobox"],[role="menu"],[role="listbox"],.message-actions-popover,.message-actions-more')) return false;
  if (document.querySelector('[role="dialog"],[role="alertdialog"],dialog[open]')) return false;
  return !window.getSelection()?.toString();
}

# UI keyboard accessibility

All future UI updates must preserve full keyboard accessibility. Every action and field must be reachable and operable without a pointer, with a clear accessible name, visible focus, logical Tab/Shift+Tab order, and appropriate Enter/Space or arrow-key behavior. Avoid positive tabindex values and keyboard traps. Dialogs and popovers must manage focus and restore it on close.

Keep mail shortcuts consistent with `apps/web/src/keyboard.ts` and the shortcut reference in `apps/web/src/KeyboardHints.tsx`. Honor the user's shortcut and hint preferences. Never intercept browser/OS chords, IME input, editing, selected text or keys owned by dialogs and native widgets. Single-character shortcuts must remain switchable off. Hints supplement button labels and must not replace accessible names.

For UI changes, verify the affected journey using only the keyboard. Add or update focused browser tests for navigation, focus, dialogs and shortcut boundaries when those behaviors change. Update the shortcut reference and README when adding or changing shortcuts.

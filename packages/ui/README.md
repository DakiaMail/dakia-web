# Dakia UI

Reusable React controls built on Mantine, styled through one Dakia theme. Import components from `@dakia/ui`; add library components here rather than introducing Mantine imports throughout the application.

## Setup

Import `@dakia/ui/styles.css` after the application's legacy styles, and mount one `DakiaProvider` at the application root. The provider uses the existing `dakia-theme` preference and synchronizes Mantine and legacy light/dark selectors. Use `useDakiaColorScheme()` to change the preference.

Mantine core and hooks are pinned together at 8.3.18, compatible with the application's React 19.1. Mantine 9 currently requires React 19.2. Upgrade those dependencies as a separate, tested change.

## Central theme

- `src/tokens.css`: semantic colors in both themes, font, control dimensions, focus ring and motion. `--ui-confirmation-duration` is a unitless number of milliseconds.
- `src/theme.ts`: Mantine defaults and shared `primary`, `secondary`, `subtle`, and `danger` variants.
- `src/styles.css`: component structure, responsive labels, focus and reduced motion. Imports only the Mantine component styles used by the library, without its global reset.
- `src/Provider.tsx`: theme provider and color scheme persistence. The current scale bridges Dakia's 13px root font to Mantine's rem units.

Theme existing components here instead of setting colors and radii at call sites. Preserve native semantics, labels and keyboard interactions when adding wrappers. The migrated controls include the translation save button and the snooze/send-later picker.

## Buttons

```tsx
import { Button, ConfirmationButton, IconButton } from '@dakia/ui';

<Button variant="secondary" onClick={cancel}>Cancel</Button>
<Button variant="danger" type="submit">Delete</Button>
<IconButton label="Refresh mail" onClick={refresh}><RefreshIcon /></IconButton>
<ConfirmationButton
  status={save.status}
  disabled={!hasChanges}
  onClick={() => save.mutate(draft)}
>
  Save translation settings
</ConfirmationButton>
```

`Button` and `IconButton` forward their refs. Their default type is `button`; specify `type="submit"` for form submission. `IconButton` requires a `label`, used for its accessible name and native tooltip. Button variants come from the central theme.

`Button` supports Mantine sections, loading, disabled and full-width props. Its `keepFocusable` option uses `aria-disabled` and blocks activation while retaining keyboard focus; use `ConfirmationButton` for asynchronous actions that need a changing label.

`ConfirmationButton` accepts request status (`idle`, `pending`, `success`, `error`), `pendingLabel` (default `Saving…`) and `successLabel` (default `Saved`). It reserves space for all labels, blocks activation while pending, retains focus, announces progress and success, and resets the success label after 1.8 seconds. A new request gets a fresh confirmation. Reset the request status when edits invalidate a success. Errors remain the caller's responsibility and must be presented visibly. Reduced motion disables the text transition.

## Development and verification

Run the web dev server, then open `/ui-gallery.html` to inspect variants, disabled/loading buttons, icon controls and successful/failed confirmations. This separate Vite entry is not included in production builds.

```sh
npm test --workspace @dakia/web -- Buttons.test.tsx TranslationSettings.test.tsx
CHROME_PATH='/path/to/google-chrome' npm exec --workspace @dakia/web -- playwright test --config playwright.ui.config.ts
npm run build
```

The browser tests use disposable API fixtures. They cover keyboard saves/retries, blocked duplicate submits, stable button dimensions, light/dark themes, narrow/short viewports, reading the saved policy after reload, theme persistence, shared token overrides and reduced motion. They do not claim backend storage verification.

## Date and time selector

`DateTimeSelector` combines future-only local-time presets with a custom date and time form. It uses Mantine Modal and TextInput, shared Dakia buttons, and central semantic tokens. Past or invalid values stay in the selector with an inline error. The local timezone is visible.

```tsx
<DateTimeSelector
  opened={open}
  title="Snooze until"
  confirmLabel="Snooze"
  value={localValue}
  onChoose={setLocalValue}
  onClose={() => setOpen(false)}
/>
```

Values use `YYYY-MM-DDTHH:mm` in the device's local timezone. Convert to an ISO timestamp at the application boundary. `dateTimePresets`, `localDateTime`, and `validFutureDateTime` are exported for callers that need the same date rules. No mail APIs, router, or app reload guards are imported by the library.

The dialog initially focuses the first preset. Custom selection focuses Date; Back returns focus to the custom-option button. Tab and Shift+Tab stay inside the dialog and plain Escape closes it. The default portal and focus return work for standalone use. When nesting inside another dialog, set `withinPortal={false}` to remain inside its focus boundary. A host that removes the trigger on selection can set `returnFocus={false}` and restore focus to its next meaningful control. The mail adapter handles this alongside app history and reload guards.

The development gallery includes standalone snooze and send-later examples. Chrome tests cover standalone reuse, nested composer focus, presets, validation, custom selection, theme tokens, narrow/short layouts, and disposable-demo snooze persistence.

## Action feedback

`Toast` provides non-interactive, polite status feedback using Mantine Notification. Keep `<Toast announcementKey={confirmation?.id}>{confirmation?.message}</Toast>` mounted so screen readers can observe new confirmations. The caller controls its lifetime; the mail app clears confirmations after seven seconds. It does not move focus or block pointer actions, and it stays below interactive overlays. Its colors, shape and motion use the central theme. Compact layouts position it above the mail toolbar.

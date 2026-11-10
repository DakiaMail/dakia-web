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

Theme existing components here instead of setting colors and radii at call sites. Preserve native semantics, labels and keyboard interactions when adding wrappers. The initial migration includes the translation save button only.

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

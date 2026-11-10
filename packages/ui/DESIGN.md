# Dakia UI design direction

Every component in `@dakia/ui` must follow Linear's product-interface style. Mantine supplies interaction behavior and accessibility primitives; Dakia's shared tokens and component styles own the appearance. Migrate existing Dakia controls one component at a time.

## Official references

- [A calmer interface for a product in motion](https://linear.app/now/behind-the-latest-design-refresh), March 2026: let the active task dominate, reduce decorative icon treatments, keep navigation compact, and communicate structure with restrained separation. Its account of incremental rollout and shared theme tokens is relevant to this library.
- [How we redesigned the Linear UI](https://linear.app/now/how-we-redesigned-the-linear-ui), March 2024: evaluate hierarchy, appearance and environment together. Check label/icon alignment and both themes in real workflows. Treat complete reference screens as a visual benchmark while refining components.
- [Linear Method: Principles & Practices](https://linear.app/method/introduction): purpose-built workflows, clear language and small, concrete changes. This is product philosophy rather than a component specification.
- [Linear Brand Guidelines](https://linear.app/brand): brand identity guidance, not an application component manual. Retain Dakia's identity and assets.

These sources describe principles and selected design decisions. The research did not locate a complete official public component specification or downloadable token library. Do not present third-party reconstructions as Linear's official design system.

## Live application observations

Reviewed the authenticated Chrome application on November 10, 2026: issue-list toolbar, filter menu, navigation, search view and agent composer. No workspace records were edited.

The observed interface uses subdued navigation, restrained accent color, compact controls, quiet surfaces and consistent icon/label spacing. The filter menu keeps icons, labels, submenu indicators and shortcut hints aligned. Escape dismisses that menu and returns focus to its trigger.

One observed desktop toolbar control, New issue, uses Inter Variable at 12px/500, a 28px visual height and pill geometry. This is evidence for that control in that context, not a universal button token. Dakia's mail commands retain the requested regular 400 weight. Preserve readable text, visible focus and larger touch targets even where the reference uses denser desktop controls.

## Component rules

- Capture the corresponding live Linear component before designing or migrating its Dakia counterpart. Compare complete context and states, rather than relying on a generic purple-and-dark aesthetic.
- Centralize typography, geometry, colors, focus and motion in shared tokens. Avoid app-specific styling that changes one action's emphasis accidentally.
- Buttons have three emphasis levels: primary, secondary and tertiary/ghost. Use primary for the main decision, secondary for supporting decisions, and ghost for routine toolbar actions. Destructive tone is independent of emphasis.
- Keep icons optically aligned, labels readable, shortcut hints subordinate and geometry consistent between siblings. Preserve shortcut and hint preferences.
- Specify default, hover, pressed, focus-visible, disabled and loading states. Keep transitions restrained, honor reduced motion and reserve dimensions through loading or confirmation changes.
- Use neutral surfaces and subtle separators. Do not soften boundaries or text past the point where actions, focus or content become hard to distinguish.
- Verify every migrated component in Chrome in light/dark themes and narrow/short layouts. Exercise keyboard access, popover/dialog focus and restoration, and persistence where relevant. Inspect screenshots as well as assertions.

This direction applies to the custom library and its migrated consumers. It does not authorize a simultaneous redesign of the rest of Dakia.

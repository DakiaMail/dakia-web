import { ActionIcon, Button, createTheme, defaultVariantColorsResolver } from "@mantine/core";

/** Semantic colors live in tokens.css. All library components share these defaults. */
export const dakiaTheme = createTheme({
  primaryColor: "dakia",
  primaryShade: { light: 6, dark: 3 },
  colors: { dakia: ["var(--ui-accent-0)", "var(--ui-accent-1)", "var(--ui-accent-2)", "var(--ui-accent-3)", "var(--ui-accent-4)", "var(--ui-accent-5)", "var(--ui-accent-6)", "var(--ui-accent-7)", "var(--ui-accent-8)", "var(--ui-accent-9)"] },
  fontFamily: "var(--ui-font-family)",
  fontSizes: { sm: "var(--ui-font-size)" },
  // Dakia uses a 13px root; Mantine's rem utilities assume a 16px root.
  scale: 16 / 13,
  defaultRadius: "var(--ui-control-radius)",
  respectReducedMotion: true,
  focusRing: "auto",
  variantColorResolver(input) {
    const common = { border: "1px solid transparent" };
    switch (input.variant) {
      case "primary": return { ...common, background: "var(--ui-button-primary-bg)", hover: "var(--ui-button-primary-hover)", color: "var(--ui-button-primary-text)", hoverColor: "var(--ui-button-primary-text)" };
      case "secondary": return { border: "1px solid var(--ui-button-border)", background: "var(--ui-button-secondary-bg)", hover: "var(--ui-button-secondary-hover)", color: "var(--color-text)", hoverColor: "var(--color-text)" };
      case "tertiary": return { ...common, background: "transparent", hover: "var(--ui-button-ghost-hover)", color: "var(--ui-button-ghost-text)", hoverColor: "var(--color-text)" };
      default: return defaultVariantColorsResolver(input);
    }
  },
  components: {
    Button: Button.extend({ defaultProps: { variant: "primary", size: "sm", loaderProps: { size: 14 } }, classNames: { root: "dakia-button", inner: "dakia-button-inner", label: "dakia-button-label" } }),
    ActionIcon: ActionIcon.extend({ defaultProps: { variant: "tertiary", size: "md", loaderProps: { size: 14 } }, classNames: { root: "dakia-icon-button", loader: "dakia-icon-button-loader" } }),
  },
});

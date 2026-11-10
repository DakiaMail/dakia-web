import { localStorageColorSchemeManager, MantineProvider, useComputedColorScheme, useMantineColorScheme } from "@mantine/core";
import { useLayoutEffect, type ReactNode } from "react";
import { dakiaTheme } from "./theme";

const semanticColors = { "--mantine-color-text": "var(--color-text)", "--mantine-color-body": "var(--color-bg)", "--mantine-color-dimmed": "var(--color-secondary)", "--mantine-color-default-border": "var(--color-border)" };
const colorSchemeManager = localStorageColorSchemeManager({ key: "dakia-theme" });

function LegacyThemeBridge() {
  const scheme = useComputedColorScheme("light", { getInitialValueInEffect: false });
  useLayoutEffect(() => { document.documentElement.dataset.theme = scheme; }, [scheme]);
  return null;
}

export function DakiaProvider({ children }: { children: ReactNode }) {
  return <MantineProvider theme={dakiaTheme} deduplicateCssVariables={false} cssVariablesResolver={() => ({ variables: {}, light: semanticColors, dark: semanticColors })} colorSchemeManager={colorSchemeManager} defaultColorScheme="light">
    <LegacyThemeBridge />{children}
  </MantineProvider>;
}

export function useDakiaColorScheme() {
  const colorScheme = useComputedColorScheme("light", { getInitialValueInEffect: false });
  const { setColorScheme } = useMantineColorScheme();
  return { colorScheme, setColorScheme };
}

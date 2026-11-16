import { Tooltip as MantineTooltip, type TooltipProps as MantineTooltipProps } from "@mantine/core";

export type TooltipProps = Pick<MantineTooltipProps, "children" | "label">;

export function Tooltip({ children, label }: TooltipProps) {
  return <MantineTooltip label={label} position="right" offset={8} openDelay={250} multiline
    events={{ hover: true, focus: true, touch: false }} classNames={{ tooltip: "dakia-tooltip" }}>
    {children}
  </MantineTooltip>;
}

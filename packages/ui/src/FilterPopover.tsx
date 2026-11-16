import { Popover } from "@mantine/core";
import type { ReactElement, ReactNode } from "react";

export interface FilterPopoverProps {
  opened: boolean;
  onChange: (opened: boolean) => void;
  label: string;
  trigger: ReactElement;
  children: ReactNode;
}
/** A positioned filter editor with a focus boundary and focus return to its trigger. */
export function FilterPopover({ opened, onChange, label, trigger, children }: FilterPopoverProps) {
  return <Popover opened={opened} onChange={onChange} position="bottom-end" width={320} withinPortal
    trapFocus returnFocus closeOnEscape={false} offset={6} zIndex={35} transitionProps={{ duration: 0 }}
    middlewares={{ flip: true, shift: { padding: 8 } }} classNames={{ dropdown: "dakia-filter-popover" }}>
    <Popover.Target>{trigger}</Popover.Target>
    <Popover.Dropdown role="dialog" aria-label={label} onKeyDownCapture={event => {
      if (event.key !== "Escape" || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey || event.repeat || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229) return;
      event.preventDefault(); event.stopPropagation(); onChange(false);
    }}>{children}</Popover.Dropdown>
  </Popover>;
}

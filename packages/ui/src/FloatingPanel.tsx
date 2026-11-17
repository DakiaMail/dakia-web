import { Popover, type PopoverProps } from "@mantine/core";
import type { ReactElement, ReactNode } from "react";

export interface FloatingPanelProps {
  opened: boolean;
  onChange: (opened: boolean) => void;
  label: string;
  trigger: ReactElement;
  children: ReactNode;
  width?: number;
  position?: PopoverProps["position"];
}

/** A floating information panel. The primitive owns positioning, focus and dismissal. */
export function FloatingPanel({ opened, onChange, label, trigger, children, width = 420, position = "bottom-start" }: FloatingPanelProps) {
  return <div className="dakia-floating-anchor" onKeyDownCapture={event => {
    // Leave OS chords, repeated keys and composition outside the primitive's bare Escape handler.
    if (opened && event.key === "Escape" && (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey || event.repeat || event.nativeEvent.isComposing || event.nativeEvent.keyCode === 229)) event.stopPropagation();
  }} onKeyDown={event => {
    if (opened && event.key === "Escape") { event.preventDefault(); event.stopPropagation(); }
  }}>
    <Popover opened={opened} onChange={onChange} position={position} width={width} withinPortal
      // Escape returns focus immediately; outside clicks must keep their destination.
      trapFocus returnFocus={false} closeOnEscape offset={6} zIndex={35} transitionProps={{ duration: 0 }}
      middlewares={{ flip: true, shift: { padding: 8 } }} classNames={{ dropdown: "dakia-floating-panel" }}>
      <Popover.Target>{trigger}</Popover.Target>
      <Popover.Dropdown role="dialog" aria-label={label} tabIndex={-1}>{children}</Popover.Dropdown>
    </Popover>
  </div>;
}

import { Menu, type MenuItemProps } from "@mantine/core";
import { cloneElement, createContext, forwardRef, useContext, useEffect, useLayoutEffect, useId, useRef, useState, type KeyboardEvent, type MouseEventHandler, type HTMLAttributes, type ReactElement, type ReactNode } from "react";

interface MenuContextValue { closeAll: () => void; rootId: string; depth: number; }
const MenuContext = createContext<MenuContextValue | null>(null);
const menuClasses = { dropdown: "dakia-menu", item: "dakia-menu-item", itemLabel: "dakia-menu-label", itemSection: "dakia-menu-section", divider: "dakia-menu-divider" };

export interface DropdownMenuProps {
  /** A button that forwards its ref and native button props. */
  trigger: ReactElement<HTMLAttributes<HTMLElement>>;
  label: string;
  children: ReactNode;
  disabled?: boolean;
  /** Submenus open on ArrowRight and close on ArrowLeft or Escape. */
  submenu?: boolean;
}
function unmodified(event: KeyboardEvent) {
  return !event.ctrlKey && !event.metaKey && !event.altKey && !event.shiftKey && !event.nativeEvent.isComposing && event.nativeEvent.keyCode !== 229 && !window.getSelection()?.toString();
}
function items(panel: HTMLElement | null) {
  return Array.from(panel?.querySelectorAll<HTMLElement>("[data-menu-item]:not([data-disabled])") || [])
    .filter(item => item.closest('[role="menu"]') === panel);
}

/** Mantine supplies positioning, roles and item navigation; this adapter owns app-safe keyboard boundaries and focus return. */
export function DropdownMenu({ trigger, label, children, disabled = false, submenu = false }: DropdownMenuProps) {
  const parent = useContext(MenuContext);
  const [opened, setOpened] = useState(false);
  const triggerRef = useRef<HTMLElement>(null);
  const panelRef = useRef<HTMLDivElement>(null);
  const ownId = useId();
  const rootId = parent?.rootId || ownId;
  const depth = (parent?.depth ?? -1) + 1;
  const initial = useRef<"first" | "last" | "pointer">("first");
  const search = useRef({ text: "", at: 0 });
  const hoverTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  function closeCurrent() { triggerRef.current?.focus(); setOpened(false); search.current.text = ""; }
  const closeAll = parent?.closeAll || closeCurrent;
  function open(edge: "first" | "last" | "pointer" = "first") {
    if (disabled) return;
    initial.current = edge;
    if (!opened) search.current = { text: "", at: 0 };
    setOpened(true);
    if (opened && edge !== "pointer") { const rows = items(panelRef.current); (edge === "last" ? rows.at(-1) : rows[0])?.focus({ preventScroll: true }); }
  }
  useLayoutEffect(() => {
    if (!opened) return;
    if (initial.current !== "pointer") {
      const rows = items(panelRef.current);
      (initial.current === "last" ? rows.at(-1) : rows[0])?.focus({ preventScroll: true });
    }
    const onFocus = (event: FocusEvent) => {
      const target = event.target as HTMLElement;
      const owner = target.closest<HTMLElement>('[role="menu"]');
      if (submenu && !triggerRef.current?.contains(target) && owner !== panelRef.current && Number(owner?.dataset.dakiaMenuDepth ?? -1) <= depth) { setOpened(false); return; }
      if (triggerRef.current?.contains(target) || target.closest('[data-dakia-menu-root]')?.getAttribute('data-dakia-menu-root') === rootId) return;
      setOpened(false);
    };
    const onPointer = (event: PointerEvent) => {
      const target = event.target as HTMLElement;
      if (triggerRef.current?.contains(target) || target.closest('[data-dakia-menu-root]')?.getAttribute('data-dakia-menu-root') === rootId) return;
      setOpened(false);
    };
    document.addEventListener("focusin", onFocus);
    if (!parent) document.addEventListener("pointerdown", onPointer);
    return () => { document.removeEventListener("focusin", onFocus); document.removeEventListener("pointerdown", onPointer); };
  }, [opened, rootId]);
  useEffect(() => () => clearTimeout(hoverTimer.current), []);
  function keys(event: KeyboardEvent<HTMLElement>) {
    // Mantine's navigation handles bare keys. Preserve browser chords and IME input before it sees them.
    if (!unmodified(event)) { if (event.key !== "Tab") event.stopPropagation(); return; }
    if (event.target === triggerRef.current && ((!submenu && (event.key === "ArrowDown" || event.key === "ArrowUp")) || (submenu && event.key === "ArrowRight"))) {
      event.preventDefault(); event.stopPropagation(); open(event.key === "ArrowUp" ? "last" : "first"); return;
    }
    if ((event.target as HTMLElement).closest('[role="menu"]') !== panelRef.current) return;
    initial.current = "first";
    if (event.key.length > 1) search.current.text = "";
    if (event.key === "Escape" || (submenu && event.key === "ArrowLeft")) {
      event.preventDefault(); event.stopPropagation(); closeCurrent(); return;
    }
    if (event.key.length === 1 && event.key !== " " && !event.repeat) {
      // Typeahead remains local to the open menu, so letters cannot invoke mail shortcuts.
      const now = Date.now(), previous = now - search.current.at < 700 ? search.current.text : "";
      const letter = event.key.toLocaleLowerCase();
      const text = previous === letter ? letter : previous + letter;
      search.current = { text, at: now };
      const rows = items(panelRef.current), current = rows.indexOf(document.activeElement as HTMLElement);
      const ordered = [...rows.slice(current + 1), ...rows.slice(0, current + 1)];
      const match = ordered.find(row => row.dataset.menuLabel?.toLocaleLowerCase().startsWith(text));
      if (match) { event.preventDefault(); match.focus(); match.scrollIntoView({ block: "nearest" }); }
      event.stopPropagation();
    }
  }
  function tab(event: KeyboardEvent<HTMLElement>) {
    if (event.key === "Tab" && !event.ctrlKey && !event.metaKey && !event.altKey && !event.nativeEvent.isComposing) {
      // Focus the root trigger before the browser performs its normal Tab/Shift+Tab movement.
      closeAll();
      event.stopPropagation();
    }
  }
  const child = cloneElement(trigger, submenu ? {
    onMouseEnter: event => { trigger.props.onMouseEnter?.(event); clearTimeout(hoverTimer.current); hoverTimer.current = setTimeout(() => open("pointer"), 120); },
  } : {});
  useEffect(() => { if (disabled) setOpened(false); }, [disabled]);
  return <MenuContext.Provider value={{ closeAll, rootId, depth }}>
    <div className={submenu ? "dakia-menu-submenu" : "dakia-menu-root"} onKeyDownCapture={keys} onKeyDown={tab}
      onMouseLeave={submenu ? () => { clearTimeout(hoverTimer.current); if (initial.current === "pointer") hoverTimer.current = setTimeout(() => setOpened(false), 180); } : undefined}
      onMouseEnter={submenu ? () => clearTimeout(hoverTimer.current) : undefined}>
      <Menu opened={opened} onChange={value => { if (!disabled) { initial.current = "first"; if (value) search.current = { text: "", at: 0 }; setOpened(value); } }}
        position={submenu ? "right-start" : "bottom-end"} offset={submenu ? 4 : 5} width={submenu ? 200 : 240}
        withinPortal trapFocus={false} returnFocus={false} closeOnEscape={false} closeOnClickOutside={false} loop
        closeOnItemClick={false} withInitialFocusPlaceholder={false} zIndex={35}
        transitionProps={{ duration: 0 }} middlewares={{ flip: true, shift: { padding: 8, crossAxis: true } }} classNames={menuClasses}>
        <Menu.Target ref={triggerRef}>{child}</Menu.Target>
        <Menu.Dropdown ref={panelRef} aria-label={label} aria-labelledby={`${ownId}-label`} data-dakia-menu-root={rootId} data-dakia-menu-depth={depth} onKeyDownCapture={keys} onKeyDown={tab}>
          <span id={`${ownId}-label`} className="dakia-sr-only">{label}</span>
          {children}
        </Menu.Dropdown>
      </Menu>
    </div>
  </MenuContext.Provider>;
}

export interface DropdownMenuItemProps extends Omit<MenuItemProps, "children" | "leftSection" | "rightSection"> {
  label: string;
  icon?: ReactNode;
  shortcutHint?: ReactNode;
  checked?: boolean;
  /** Submenu triggers retain their parent menu. */
  closeOnSelect?: boolean;
  onClick?: MouseEventHandler<HTMLButtonElement>;
  "aria-label"?: string;
}
export const DropdownMenuItem = forwardRef<HTMLButtonElement, DropdownMenuItemProps>(function DropdownMenuItem(
  { label, icon, shortcutHint, checked, closeOnSelect = true, onClick, ...props }, ref,
) {
  const context = useContext(MenuContext);
  return <Menu.Item {...props} ref={ref} data-menu-label={label} closeMenuOnClick={false}
    renderRoot={rootProps => <button {...rootProps} role={checked === undefined ? "menuitem" : "menuitemradio"} aria-checked={checked} />}
    leftSection={<span className="dakia-menu-icon" aria-hidden="true">{checked ? "✓" : icon}</span>}
    rightSection={shortcutHint && <span className="dakia-menu-hint" aria-hidden="true">{shortcutHint}</span>}
    onClick={event => { if (closeOnSelect) context?.closeAll(); onClick?.(event); }}>
    {label}
  </Menu.Item>;
});

export function DropdownSubmenu({ label, icon, disabled, children }: { label: string; icon?: ReactNode; disabled?: boolean; children: ReactNode }) {
  return <DropdownMenu label={label} submenu disabled={disabled} trigger={<DropdownMenuItem label={label} icon={icon} disabled={disabled} closeOnSelect={false} shortcutHint={<span aria-hidden="true">›</span>} />}>
    {children}
  </DropdownMenu>;
}
export const DropdownMenuDivider = Menu.Divider;

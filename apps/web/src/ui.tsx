import * as Dialog from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { useRef, type ReactNode } from "react";
import { plainKey } from "./keyboard";
import { useReloadGuard } from "./AppUpdate";
import { useBackLayer } from "./appHistory";
export function Modal({
  title,
  children,
  onClose,
  wide = false,
  className = "",
  initialFocus,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void | boolean | Promise<void | boolean>;
  wide?: boolean;
  className?: string;
  initialFocus?: string;
}) {
  const content = useRef<HTMLDivElement>(null);
  const returnFocus = useRef(document.activeElement as HTMLElement | null);
  useReloadGuard();
  useBackLayer(onClose);
  return (
    <Dialog.Root
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
    >
      <Dialog.Portal>
        <Dialog.Overlay className="overlay" />
        <Dialog.Content
          ref={content}
          className={`dialog ${wide ? "wide" : ""} ${className}`}
          onOpenAutoFocus={event => {
            const target = initialFocus ? content.current?.querySelector<HTMLElement>(initialFocus) : null;
            if (target && !target.matches(":disabled")) { event.preventDefault(); target.focus(); }
          }}
          onCloseAutoFocus={event => {
            event.preventDefault();
            requestAnimationFrame(() => {
              const original = returnFocus.current;
              const bounds = original?.getBoundingClientRect();
              if (original && original !== document.body && original.isConnected && bounds && bounds.width > 0 && bounds.height > 0 && bounds.right > 0 && bounds.left < innerWidth && bounds.bottom > 0 && bounds.top < innerHeight && !original.matches(":disabled,[inert]")) original.focus();
              else document.querySelector<HTMLElement>('.workspace-view .workspace-content > h1, .app.reading .reader, .mailbox .message-list[tabindex], .mailbox')?.focus();
            });
          }}
          onInteractOutside={(e) => e.preventDefault()}
          onEscapeKeyDown={(event) => {
            // Radix listens in document capture, before recipient input handlers.
            // Nested time pickers and recipient suggestions dismiss before their enclosing draft.
            if ((event.target instanceof Element && event.target.closest("[data-dakia-date-time]")) || !plainKey(event) || event.repeat ||
              (event.target instanceof Element && event.target.matches(
                '[aria-autocomplete="list"][aria-expanded="true"]',
              ))) event.preventDefault();
          }}
        >
          <header>
            <Dialog.Title>{title}</Dialog.Title>
            <Dialog.Close asChild>
              <button title="Close" aria-label="Close">
                <X size={18} />
              </button>
            </Dialog.Close>
          </header>
          <Dialog.Description className="sr-only">{title}</Dialog.Description>
          {children}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
export function ErrorNotice({ error }: { error: unknown }) {
  return error ? (
    <p role="alert" className="error">
      {error instanceof Error ? error.message : String(error)}
    </p>
  ) : null;
}
export function Empty({
  title,
  children,
}: {
  title: string;
  children?: ReactNode;
}) {
  return (
    <div className="empty">
      <h2>{title}</h2>
      {children}
    </div>
  );
}

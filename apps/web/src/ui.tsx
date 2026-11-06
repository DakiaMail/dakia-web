import * as Dialog from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import type { ReactNode } from "react";
import { plainKey } from "./keyboard";
import { useReloadGuard } from "./AppUpdate";
import { useBackLayer } from "./appHistory";
export function Modal({
  title,
  children,
  onClose,
  wide = false,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void | boolean | Promise<void | boolean>;
  wide?: boolean;
}) {
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
          className={`dialog ${wide ? "wide" : ""}`}
          onInteractOutside={(e) => e.preventDefault()}
          onEscapeKeyDown={(event) => {
            // Radix listens in document capture, before recipient input handlers.
            // Let an open suggestion list dismiss first, not its enclosing draft.
            if (!plainKey(event) || event.repeat ||
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

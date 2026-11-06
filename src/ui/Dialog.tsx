import * as Radix from "@radix-ui/react-dialog";
import { type ReactNode, useRef } from "react";
import { Kbd } from "./Kbd";

interface Props {
  open: boolean;
  onClose: () => void;
  /** The dialog's name, in its header. */
  title: string;
  /** The `Esc` hint at the header's end. */
  closeHint: string;
  children: ReactNode;
}

/**
 * A modal dialog as the design draws the palette: a scrim over the window and a 640 px flyout
 * 96 px from the top, centered, with a header row. Esc and a click on the scrim close it; focus
 * stays inside while it is open and goes back where it was after. Its content scrolls when the
 * window is too short.
 */
export function Dialog({ open, onClose, title, closeHint, children }: Props) {
  // Radix gives the focus back to a trigger, and these dialogs open without one.
  const returnTo = useRef<HTMLElement | null>(null);
  return (
    <Radix.Root open={open} onOpenChange={(next) => !next && onClose()}>
      <Radix.Portal>
        {/* Above the list's sticky column headers (z-10), level with menus (z-50). */}
        <Radix.Overlay className="fixed inset-0 z-50 flex items-start justify-center bg-scrim px-4 pt-24 pb-8">
          <Radix.Content
            onOpenAutoFocus={() => {
              const active = document.activeElement;
              returnTo.current = active instanceof HTMLElement ? active : null;
            }}
            onCloseAutoFocus={(event) => {
              const element = returnTo.current;
              returnTo.current = null;
              if (element?.isConnected) {
                event.preventDefault();
                element.focus({ preventScroll: true });
              }
            }}
            aria-describedby={undefined}
            className="flex max-h-full w-palette min-w-0 flex-col overflow-y-auto rounded-flyout border border-flyout-border bg-flyout text-fg shadow-dialog outline-none"
          >
            <div className="sticky top-0 flex h-13 shrink-0 items-center gap-2.5 border-stroke border-b bg-flyout pr-3 pl-4">
              <Radix.Title className="flex-1 truncate font-semibold text-title">
                {title}
              </Radix.Title>
              <Kbd>{closeHint}</Kbd>
            </div>
            {children}
          </Radix.Content>
        </Radix.Overlay>
      </Radix.Portal>
    </Radix.Root>
  );
}

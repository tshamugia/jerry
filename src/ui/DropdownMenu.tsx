import * as Radix from "@radix-ui/react-dropdown-menu";
import type { ReactNode } from "react";

const ITEM =
  "flex h-8 min-w-0 items-center gap-2 rounded-ctrl pr-3 pl-2 outline-none data-disabled:text-disabled data-highlighted:bg-subtle-hover";

/**
 * A menu that drops from a button (`trigger`, one element that takes the trigger's props), drawn
 * as a flyout like `ContextMenu`: arrows move through it, Enter picks, Esc closes. Put
 * `DropdownItem`s in `children`; a long one scrolls.
 */
export function DropdownMenu({
  trigger,
  children,
  label,
  onOpenChange,
}: {
  trigger: ReactNode;
  children: ReactNode;
  /** The menu's name for screen readers. */
  label: string;
  onOpenChange?: (open: boolean) => void;
}) {
  return (
    <Radix.Root modal={false} onOpenChange={onOpenChange}>
      <Radix.Trigger asChild>{trigger}</Radix.Trigger>
      <Radix.Portal>
        <Radix.Content
          aria-label={label}
          align="start"
          sideOffset={4}
          className="z-50 max-h-menu min-w-48 max-w-menu overflow-y-auto rounded-flyout border border-flyout-border bg-flyout p-1 text-fg shadow-flyout"
        >
          {children}
        </Radix.Content>
      </Radix.Portal>
    </Radix.Root>
  );
}

export function DropdownItem({
  children,
  disabled,
  onSelect,
  title,
}: {
  children: ReactNode;
  disabled?: boolean;
  onSelect?: () => void;
  /** A tooltip: the full text of an item cut short. */
  title?: string;
}) {
  return (
    <Radix.Item disabled={disabled} onSelect={onSelect} title={title} className={ITEM}>
      {children}
    </Radix.Item>
  );
}

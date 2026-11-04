import * as Radix from "@radix-ui/react-context-menu";
import type { ReactNode } from "react";
import { CheckIcon } from "./icons";

const ITEM =
  "flex h-8 items-center gap-2 rounded-ctrl pr-3 pl-2 outline-none data-disabled:text-disabled data-highlighted:bg-hover";

/**
 * A right-click menu on `children` (one element, which takes the trigger's props), drawn as a
 * flyout. Put `MenuItem`, `MenuCheckbox` and `MenuSeparator` in `menu`. Shift+F10 and the Menu
 * key open it too when the trigger has the focus.
 */
export function ContextMenu({
  children,
  menu,
  label,
}: {
  children: ReactNode;
  menu: ReactNode;
  /** The menu's name for screen readers. */
  label: string;
}) {
  return (
    <Radix.Root>
      <Radix.Trigger asChild>{children}</Radix.Trigger>
      <Radix.Portal>
        <Radix.Content
          aria-label={label}
          className="z-50 min-w-48 rounded-flyout border border-flyout-border bg-flyout p-1 text-fg shadow-flyout"
        >
          {menu}
        </Radix.Content>
      </Radix.Portal>
    </Radix.Root>
  );
}

export function MenuItem({
  children,
  disabled,
  onSelect,
}: {
  children: ReactNode;
  disabled?: boolean;
  onSelect: () => void;
}) {
  return (
    <Radix.Item disabled={disabled} onSelect={onSelect} className={ITEM}>
      <span className="size-4 shrink-0" />
      {children}
    </Radix.Item>
  );
}

/** A menu item with a check mark while `checked`. */
export function MenuCheckbox({
  children,
  checked,
  disabled,
  onCheckedChange,
}: {
  children: ReactNode;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange: () => void;
}) {
  return (
    <Radix.CheckboxItem
      checked={checked}
      disabled={disabled}
      onCheckedChange={onCheckedChange}
      // Keeps the menu open, so several columns can be switched in a row.
      onSelect={(event) => event.preventDefault()}
      className={ITEM}
    >
      <span className="flex size-4 shrink-0 items-center justify-center">
        <Radix.ItemIndicator>
          <CheckIcon />
        </Radix.ItemIndicator>
      </span>
      {children}
    </Radix.CheckboxItem>
  );
}

export function MenuSeparator() {
  return <Radix.Separator className="my-1 h-px bg-stroke" />;
}

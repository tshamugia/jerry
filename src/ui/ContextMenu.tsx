import * as Radix from "@radix-ui/react-context-menu";
import { createContext, type ReactNode, useContext, useRef } from "react";
import { CheckIcon, ChevronIcon } from "./icons";

const ITEM =
  "group flex h-8 items-center gap-2 rounded-ctrl pr-3 pl-2 outline-none data-disabled:text-disabled data-highlighted:bg-hover";

const CONTENT =
  "z-50 min-w-48 rounded-flyout border border-flyout-border bg-flyout p-1 text-fg shadow-flyout";

/** Runs an item's action once its menu has closed (`MenuItem`'s `afterClose`). */
const AfterClose = createContext<((action: () => void) => void) | undefined>(undefined);

/** Keys held when an item was picked: Shift+Delete deletes for good, as in Explorer. */
export interface MenuModifiers {
  shift: boolean;
}

/**
 * A right-click menu on `children` (one element, which takes the trigger's props), drawn as a
 * flyout. Put `MenuItem`, `MenuCheckbox`, `MenuRadioGroup`, `MenuSub` and `MenuSeparator` in
 * `menu`; it renders only while the menu is open. Shift+F10 and the Menu key open it too when the
 * trigger has the focus; `openContextMenuAt` opens it where the keyboard is.
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
  const pending = useRef<() => void>(undefined);
  return (
    <Radix.Root>
      <Radix.Trigger asChild>{children}</Radix.Trigger>
      <Radix.Portal>
        <Radix.Content
          aria-label={label}
          onCloseAutoFocus={(event) => {
            const action = pending.current;
            pending.current = undefined;
            if (action) {
              // The action puts the focus where it belongs (a field it opens).
              event.preventDefault();
              action();
            }
          }}
          className={CONTENT}
        >
          <AfterClose.Provider
            value={(action) => {
              pending.current = action;
            }}
          >
            {menu}
          </AfterClose.Provider>
        </Radix.Content>
      </Radix.Portal>
    </Radix.Root>
  );
}

/**
 * Opens the context menu of `trigger` with its top left corner at `x`, `y` (viewport px), as a
 * right click there would: for Shift+F10 and the Menu key, at the item the keyboard is on.
 */
export function openContextMenuAt(trigger: Element, x: number, y: number): void {
  trigger.dispatchEvent(
    new MouseEvent("contextmenu", {
      bubbles: true,
      cancelable: true,
      button: 2,
      clientX: x,
      clientY: y,
    }),
  );
}

export function MenuItem({
  children,
  shortcut,
  disabled,
  afterClose = false,
  onSelect,
}: {
  children: ReactNode;
  /** The command's shortcut, as people read it ("Ctrl+Shift+N"), at the right. */
  shortcut?: string;
  disabled?: boolean;
  /**
   * Runs `onSelect` once the menu has closed, and the focus stays where it puts it: for an item
   * that opens a field. While the menu is open it keeps the focus inside, and on closing it gives
   * the focus back to its trigger.
   */
  afterClose?: boolean;
  onSelect: (modifiers: MenuModifiers) => void;
}) {
  const later = useContext(AfterClose);
  // Shift as the item is clicked or Enter pressed on it; Radix's own select event has no keys.
  const shift = useRef(false);
  const held = (event: { shiftKey: boolean }) => {
    shift.current = event.shiftKey;
  };
  const select = () => {
    const modifiers = { shift: shift.current };
    if (afterClose && later) {
      later(() => onSelect(modifiers));
    } else {
      onSelect(modifiers);
    }
  };
  return (
    <Radix.Item
      disabled={disabled}
      onPointerDown={held}
      onPointerUp={held}
      onKeyDown={held}
      onSelect={select}
      aria-keyshortcuts={keyShortcuts(shortcut)}
      className={ITEM}
    >
      <span className="size-4 shrink-0" />
      {children}
      {shortcut && <Shortcut>{shortcut}</Shortcut>}
    </Radix.Item>
  );
}

/** A menu item with a check mark while `checked`. */
export function MenuCheckbox({
  children,
  checked,
  shortcut,
  disabled,
  onCheckedChange,
}: {
  children: ReactNode;
  checked: boolean;
  shortcut?: string;
  disabled?: boolean;
  onCheckedChange: () => void;
}) {
  return (
    <Radix.CheckboxItem
      checked={checked}
      disabled={disabled}
      onCheckedChange={onCheckedChange}
      aria-keyshortcuts={keyShortcuts(shortcut)}
      // Keeps the menu open, so several columns can be switched in a row.
      onSelect={(event) => event.preventDefault()}
      className={ITEM}
    >
      <Check />
      {children}
      {shortcut && <Shortcut>{shortcut}</Shortcut>}
    </Radix.CheckboxItem>
  );
}

/** Items of which one is chosen (`MenuRadio`), such as a sort column. */
export function MenuRadioGroup({
  value,
  onValueChange,
  children,
}: {
  value: string;
  /** None: the choice is fixed for now (the view, Details only until v0.3). */
  onValueChange?: (value: string) => void;
  children: ReactNode;
}) {
  return (
    <Radix.RadioGroup value={value} onValueChange={onValueChange}>
      {children}
    </Radix.RadioGroup>
  );
}

/** One choice of a `MenuRadioGroup`, with a check mark while chosen. */
export function MenuRadio({
  value,
  children,
  shortcut,
  disabled,
}: {
  value: string;
  children: ReactNode;
  shortcut?: string;
  disabled?: boolean;
}) {
  return (
    <Radix.RadioItem
      value={value}
      disabled={disabled}
      aria-keyshortcuts={keyShortcuts(shortcut)}
      className={ITEM}
    >
      <Check />
      {children}
      {shortcut && <Shortcut>{shortcut}</Shortcut>}
    </Radix.RadioItem>
  );
}

/** An item that opens a submenu (`children`) to its side: on hover, → or Enter. */
export function MenuSub({ label, children }: { label: string; children: ReactNode }) {
  return (
    <Radix.Sub>
      <Radix.SubTrigger className={`${ITEM} data-[state=open]:bg-hover`}>
        <span className="size-4 shrink-0" />
        {label}
        <span className="ml-auto pl-6 text-fg-2">
          <ChevronIcon />
        </span>
      </Radix.SubTrigger>
      <Radix.Portal>
        <Radix.SubContent sideOffset={2} alignOffset={-5} className={CONTENT}>
          {children}
        </Radix.SubContent>
      </Radix.Portal>
    </Radix.Sub>
  );
}

export function MenuSeparator() {
  return <Radix.Separator className="my-1 h-px bg-stroke" />;
}

function Check() {
  return (
    <span className="flex size-4 shrink-0 items-center justify-center">
      <Radix.ItemIndicator>
        <CheckIcon />
      </Radix.ItemIndicator>
    </span>
  );
}

/**
 * The shortcut at the item's right. Screen readers get it from the item's `aria-keyshortcuts`
 * (`keyShortcuts`), so the item's name stays its title.
 */
function Shortcut({ children }: { children: string }) {
  return (
    <span
      aria-hidden="true"
      className="ml-auto whitespace-nowrap pl-6 text-fg-2 text-small group-data-disabled:text-disabled"
    >
      {children}
    </span>
  );
}

/** Key names as `aria-keyshortcuts` writes them, where Jerry's differ. */
const ARIA_KEYS: Record<string, string> = {
  Ctrl: "Control",
  Del: "Delete",
  Esc: "Escape",
  Ins: "Insert",
  PgUp: "PageUp",
  PgDn: "PageDown",
  "←": "ArrowLeft",
  "→": "ArrowRight",
  "↑": "ArrowUp",
  "↓": "ArrowDown",
};

/** "Ctrl+Shift+C" as `aria-keyshortcuts` takes it: "Control+Shift+C". */
function keyShortcuts(shortcut: string | undefined): string | undefined {
  return shortcut
    ?.split("+")
    .map((key) => ARIA_KEYS[key] ?? key)
    .join("+");
}

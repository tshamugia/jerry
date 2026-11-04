import { useEffect } from "react";
import { useKeymap } from "../stores/keymap";
import { type KeyTarget, lookup } from "./keymap";
import { isEnabled } from "./registry";

/**
 * Put on the inline rename field, so the rename commands work there and the list's don't.
 * Dialogs are found by their role.
 */
export const RENAME_SCOPE = { "data-command-scope": "rename" } as const;

const RENAME = '[data-command-scope="rename"]';
const DIALOG = '[role="dialog"], [role="alertdialog"]';

/** Inputs that take typed text; checkboxes, radios and buttons don't. */
const TEXT_INPUTS = new Set(["text", "search", "url", "email", "password", "number", "tel"]);

const BUTTONS =
  'button, a[href], summary, [role="button"], [role="link"], [role="tab"], [role="checkbox"], ' +
  '[role="radio"], [role="switch"], [role="menuitem"], input[type="checkbox"], ' +
  'input[type="radio"], input[type="button"], input[type="submit"], input[type="reset"]';

/** Where a key press at `element` lands: its innermost scope and what kind of control has it. */
export function keyTarget(element: EventTarget | null): KeyTarget {
  if (!(element instanceof Element)) {
    return { scope: "window", focus: "other" };
  }
  return { scope: scopeOf(element), focus: focusOf(element) };
}

function scopeOf(element: Element): KeyTarget["scope"] {
  const scope = element.closest(`${RENAME}, ${DIALOG}`);
  if (!scope) {
    return "window";
  }
  return scope.matches(RENAME) ? "rename" : "dialog";
}

function focusOf(element: Element): KeyTarget["focus"] {
  if (isTextField(element)) {
    return "text";
  }
  return element.matches(BUTTONS) ? "button" : "other";
}

function isTextField(element: Element): boolean {
  return (
    element instanceof HTMLTextAreaElement ||
    element instanceof HTMLSelectElement ||
    (element instanceof HTMLInputElement && TEXT_INPUTS.has(element.type)) ||
    (element instanceof HTMLElement && element.isContentEditable)
  );
}

/**
 * Runs the command for a key press, if one is bound where the focus is and it is enabled; then
 * the key does nothing else. Keys a component already handled (`preventDefault`), keys of an IME
 * composition and keys of commands that can't run keep their default.
 */
export function dispatchKey(event: KeyboardEvent): void {
  if (event.defaultPrevented || event.isComposing) {
    return;
  }
  const command = lookup(useKeymap.getState().keymap, event, keyTarget(event.target));
  if (!command || !isEnabled(command)) {
    return;
  }
  event.preventDefault();
  if (!event.repeat || command.repeat) {
    command.run?.();
  }
}

/** Runs commands from the keyboard anywhere in the window. Use once, in the app. */
export function useCommandKeys(): void {
  useEffect(() => {
    window.addEventListener("keydown", dispatchKey);
    return () => window.removeEventListener("keydown", dispatchKey);
  }, []);
}

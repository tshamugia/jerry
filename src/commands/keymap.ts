/**
 * Shortcuts match the physical key (`KeyboardEvent.code`), never the typed character (`key`).
 * On the Georgian layout Ctrl+C arrives as key "ც" with code "KeyC", so a shortcut works the
 * same on every layout. Shortcuts are written as on a US keyboard: "Ctrl+Shift+N", "F5".
 */

export interface Shortcut {
  code: string;
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
}

/** The parts of a keyboard event a shortcut looks at. */
export interface KeyInput {
  code: string;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
  metaKey: boolean;
  getModifierState?: (key: string) => boolean;
}

/** Key names in shortcut text that aren't a letter, a digit or F1…F24. */
const NAMED_CODES: Record<string, string> = {
  Backspace: "Backspace",
  Del: "Delete",
  Delete: "Delete",
  Enter: "Enter",
  Esc: "Escape",
  Space: "Space",
  Tab: "Tab",
  "←": "ArrowLeft",
  "→": "ArrowRight",
  "↑": "ArrowUp",
  "↓": "ArrowDown",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
  "`": "Backquote",
};

/** "Ctrl+Shift+N" → `{ code: "KeyN", ctrl: true, shift: true, alt: false }`. */
export function parseShortcut(text: string): Shortcut {
  const parts = text.split("+");
  const key = parts.pop() ?? "";
  const modifiers = new Set(parts.map((part) => part.toLowerCase()));
  return {
    code: codeOf(key),
    ctrl: modifiers.has("ctrl"),
    shift: modifiers.has("shift"),
    alt: modifiers.has("alt"),
  };
}

function codeOf(key: string): string {
  if (/^[A-Z]$/.test(key)) return `Key${key}`;
  if (/^[0-9]$/.test(key)) return `Digit${key}`;
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(key)) return key;
  const named = NAMED_CODES[key];
  if (named) return named;
  throw new Error(`Unknown key in shortcut: ${key}`);
}

/** Whether `event` is `shortcut`: same physical key, exactly the same modifiers. */
export function matches(event: KeyInput, shortcut: Shortcut): boolean {
  return (
    event.code === shortcut.code &&
    event.ctrlKey === shortcut.ctrl &&
    event.shiftKey === shortcut.shift &&
    event.altKey === shortcut.alt &&
    !event.metaKey
  );
}

/**
 * AltGr. Windows reports it as Ctrl+Alt, and on the Georgian layout AltGr types characters
 * (AltGr+C = ©, AltGr+N = ჼ, AltGr+E = ჱ), so in a text field it must type, not run Ctrl+Alt+….
 */
export function isAltGraph(event: KeyInput): boolean {
  return event.getModifierState?.("AltGraph") ?? false;
}

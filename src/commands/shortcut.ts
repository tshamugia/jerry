/**
 * Shortcuts match the physical key (`KeyboardEvent.code`), never the typed character (`key`).
 * On the Georgian layout Ctrl+C arrives as key "ც" with code "KeyC", so a shortcut works the
 * same on every layout (spike #18). Shortcuts are written as on a US keyboard: "Ctrl+Shift+N",
 * "F5", "Alt+←", "?" (Shift+/).
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
  repeat?: boolean;
  getModifierState?: (key: string) => boolean;
}

/** Key names in shortcut text that aren't a letter, a digit or F1…F24, and their codes. */
const NAMED_KEYS: Record<string, string> = {
  Backspace: "Backspace",
  Del: "Delete",
  Enter: "Enter",
  Esc: "Escape",
  Space: "Space",
  Tab: "Tab",
  Home: "Home",
  End: "End",
  PgUp: "PageUp",
  PgDn: "PageDown",
  Ins: "Insert",
  "←": "ArrowLeft",
  "→": "ArrowRight",
  "↑": "ArrowUp",
  "↓": "ArrowDown",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
  ";": "Semicolon",
  "'": "Quote",
  "[": "BracketLeft",
  "]": "BracketRight",
  "\\": "Backslash",
  "-": "Minus",
  "=": "Equal",
  "`": "Backquote",
};

/** Other spellings of a key name, for shortcuts written by hand. */
const ALIASES: Record<string, string> = {
  Delete: "Del",
  Escape: "Esc",
  Left: "←",
  Right: "→",
  Up: "↑",
  Down: "↓",
  PageUp: "PgUp",
  PageDown: "PgDn",
  Insert: "Ins",
};

const KEY_NAMES: Record<string, string> = Object.fromEntries(
  Object.entries(NAMED_KEYS).map(([name, code]) => [code, name]),
);

const MODIFIERS = new Set(["ctrl", "shift", "alt"]);

/**
 * "Ctrl+Shift+N" → `{ code: "KeyN", ctrl: true, shift: true, alt: false }`. "?" is Shift+/.
 * Returns `undefined` for text that isn't a shortcut ("Ctrl+ც", "Win+E", "Ctrl+").
 */
export function parseShortcut(text: string): Shortcut | undefined {
  if (text === "?") {
    return { code: "Slash", ctrl: false, shift: true, alt: false };
  }
  // The key itself may be "+" only as the last part, which Jerry's keymap doesn't use.
  const parts = text.split("+").map((part) => part.trim());
  const key = parts.pop();
  const modifiers = parts.map((part) => part.toLowerCase());
  if (!key || modifiers.some((part) => !MODIFIERS.has(part))) {
    return undefined;
  }
  if (new Set(modifiers).size !== modifiers.length) {
    return undefined;
  }
  const code = codeOf(key);
  if (!code) {
    return undefined;
  }
  return {
    code,
    ctrl: modifiers.includes("ctrl"),
    shift: modifiers.includes("shift"),
    alt: modifiers.includes("alt"),
  };
}

function codeOf(key: string): string | undefined {
  if (/^[A-Za-z]$/.test(key)) return `Key${key.toUpperCase()}`;
  if (/^[0-9]$/.test(key)) return `Digit${key}`;
  if (isFunctionKey(key)) return key;
  return NAMED_KEYS[ALIASES[key] ?? key];
}

/** The shortcut as people read it: "Ctrl+Shift+N", "Alt+←", "?". */
export function formatShortcut(shortcut: Shortcut): string {
  const { code, ctrl, shift, alt } = shortcut;
  if (code === "Slash" && shift && !ctrl && !alt) {
    return "?";
  }
  const key = code.replace(/^Key|^Digit/, "");
  return [ctrl && "Ctrl", shift && "Shift", alt && "Alt", KEY_NAMES[code] ?? key]
    .filter(Boolean)
    .join("+");
}

/** Whether `event` is `shortcut`: the same physical key, exactly the same modifiers. */
export function matches(event: KeyInput, shortcut: Shortcut): boolean {
  return (
    event.code === shortcut.code &&
    event.ctrlKey === shortcut.ctrl &&
    event.shiftKey === shortcut.shift &&
    event.altKey === shortcut.alt &&
    !event.metaKey
  );
}

export function sameShortcut(a: Shortcut, b: Shortcut): boolean {
  return a.code === b.code && a.ctrl === b.ctrl && a.shift === b.shift && a.alt === b.alt;
}

/**
 * AltGr. Windows reports it as Ctrl+Alt, and on the Georgian layout AltGr types characters
 * (AltGr+C = ©, AltGr+N = ჼ, AltGr+E = ჱ), so in a text field it must type, not run Ctrl+Alt+….
 */
export function isAltGraph(event: KeyInput): boolean {
  return event.getModifierState?.("AltGraph") ?? false;
}

/**
 * Whether a text field keeps this key for itself: anything without Ctrl or Alt that types a
 * character or edits (letters, digits, punctuation, Space, Backspace, Del, arrows, Enter, Tab,
 * Esc). F-keys and every Ctrl or Alt combination are free for shortcuts.
 */
export function isFieldKey(shortcut: Shortcut): boolean {
  return !shortcut.ctrl && !shortcut.alt && !isFunctionKey(shortcut.code);
}

/** Whether a focused button keeps this key for itself: plain Space and Enter press it. */
export function isButtonKey(shortcut: Shortcut): boolean {
  return (
    !shortcut.ctrl &&
    !shortcut.alt &&
    !shortcut.shift &&
    (shortcut.code === "Space" || shortcut.code === "Enter")
  );
}

function isFunctionKey(code: string): boolean {
  return /^F([1-9]|1[0-9]|2[0-4])$/.test(code);
}

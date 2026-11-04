import type { Keybinding } from "../ipc";
import {
  formatShortcut,
  isAltGraph,
  isButtonKey,
  isFieldKey,
  type KeyInput,
  matches,
  parseShortcut,
  type Shortcut,
  sameShortcut,
} from "./shortcut";

/**
 * Where a command's shortcut works:
 * - `global`: everywhere but in a dialog. In a text field only keys the field doesn't use itself
 *   work (Ctrl+F and F1 do, `?` types).
 * - `list`: in the window outside text fields (Ctrl+C, Del, Ctrl+Shift+N), so a field keeps its
 *   own copy, paste and delete.
 * - `rename`: in the inline rename field.
 * - `dialog`: in a dialog (the palette, a confirmation). Nothing else works there.
 *
 * Wherever it is, a focused button keeps Space and Enter, which press it.
 */
export type CommandContext = "global" | "list" | "rename" | "dialog";

/** Where a key press lands. */
export interface KeyTarget {
  /** The innermost scope around the focus: a dialog, the inline rename field, or the window. */
  scope: "window" | "rename" | "dialog";
  /**
   * What has the focus: a text field keeps its typing keys and AltGr, a button (or link, tab,
   * checkbox) keeps Space and Enter.
   */
  focus: "text" | "button" | "other";
}

/** What the keymap needs from a command. */
export interface Bindable {
  id: string;
  /** The default shortcuts with US key names ("Ctrl+Shift+N", "Alt+←", "?"). */
  keys: readonly string[];
  context: CommandContext;
}

export interface Binding<C extends Bindable> {
  command: C;
  shortcut: Shortcut;
  /** The user's shortcut (from `keybindings`), not a default. */
  custom: boolean;
}

/** The shortcuts in force: the user's first, then the defaults they don't shadow. */
export interface Keymap<C extends Bindable> {
  bindings: readonly Binding<C>[];
}

/**
 * The keymap for `commands`, with the shortcuts the user changed (`keybindings`, only changed
 * ones). A changed command has exactly the user's shortcut, or none if it is `null`; the others
 * keep their defaults. A user's shortcut wins over a default it collides with, which then stops
 * working. Rows for unknown commands and text that isn't a shortcut are ignored.
 */
export function buildKeymap<C extends Bindable>(
  commands: readonly C[],
  keybindings: readonly Keybinding[] = [],
): Keymap<C> {
  const changed = new Map<string, Shortcut | null>();
  for (const { command, shortcut } of keybindings) {
    const parsed = shortcut === null ? null : parseShortcut(shortcut);
    if (parsed !== undefined) {
      changed.set(command, parsed);
    }
  }

  const custom: Binding<C>[] = [];
  const defaults: Binding<C>[] = [];
  for (const command of commands) {
    const shortcut = changed.get(command.id);
    if (shortcut !== undefined) {
      if (shortcut) {
        custom.push({ command, shortcut, custom: true });
      }
      continue;
    }
    for (const key of command.keys) {
      const parsed = parseShortcut(key);
      if (parsed) {
        defaults.push({ command, shortcut: parsed, custom: false });
      }
    }
  }
  const shadowed = (binding: Binding<C>) => custom.some((mine) => collide(mine, binding));
  return { bindings: [...custom, ...defaults.filter((binding) => !shadowed(binding))] };
}

/** The command `event` runs at `target`, if any, whether or not it is enabled. */
export function lookup<C extends Bindable>(
  keymap: Keymap<C>,
  event: KeyInput,
  target: KeyTarget,
): C | undefined {
  // AltGr arrives as Ctrl+Alt; in a text field it types a character.
  if (target.focus === "text" && isAltGraph(event)) {
    return undefined;
  }
  return keymap.bindings.find(
    (binding) =>
      matches(event, binding.shortcut) && works(binding.command.context, binding.shortcut, target),
  )?.command;
}

/** A command's shortcuts in force, as people read them ("Ctrl+Shift+N"). */
export function shortcutsOf<C extends Bindable>(keymap: Keymap<C>, id: string): string[] {
  return keymap.bindings
    .filter((binding) => binding.command.id === id)
    .map((binding) => formatShortcut(binding.shortcut));
}

/** Pairs of bindings that one key press could both run. A healthy keymap has none. */
export function conflicts<C extends Bindable>(keymap: Keymap<C>): [Binding<C>, Binding<C>][] {
  const pairs: [Binding<C>, Binding<C>][] = [];
  keymap.bindings.forEach((a, i) => {
    for (const b of keymap.bindings.slice(i + 1)) {
      if (a.command.id !== b.command.id && collide(a, b)) {
        pairs.push([a, b]);
      }
    }
  });
  return pairs;
}

/**
 * The commands that already use `shortcut` where `command` would work too: what the shortcut
 * editor warns about before the user gives `command` that shortcut.
 */
export function conflictsWith<C extends Bindable>(
  keymap: Keymap<C>,
  command: C,
  shortcut: Shortcut,
): C[] {
  const wanted: Binding<C> = { command, shortcut, custom: true };
  return keymap.bindings
    .filter((binding) => binding.command.id !== command.id && collide(binding, wanted))
    .map((binding) => binding.command);
}

/** Every place a key press can land. The rename field is a text field. */
const TARGETS: readonly KeyTarget[] = [
  { scope: "window", focus: "other" },
  { scope: "window", focus: "text" },
  { scope: "window", focus: "button" },
  { scope: "rename", focus: "text" },
  { scope: "dialog", focus: "other" },
  { scope: "dialog", focus: "text" },
  { scope: "dialog", focus: "button" },
];

/** Whether a shortcut of a command in `context` works at `target`. */
function works(context: CommandContext, shortcut: Shortcut, target: KeyTarget): boolean {
  if (target.focus === "button" && isButtonKey(shortcut)) {
    return false;
  }
  switch (context) {
    case "global":
      return target.scope !== "dialog" && !(target.focus === "text" && isFieldKey(shortcut));
    case "list":
      return target.scope === "window" && target.focus !== "text";
    case "rename":
      return target.scope === "rename";
    case "dialog":
      return target.scope === "dialog";
  }
}

/** Whether one key press could run both bindings. */
function collide<C extends Bindable>(a: Binding<C>, b: Binding<C>): boolean {
  return (
    sameShortcut(a.shortcut, b.shortcut) &&
    TARGETS.some(
      (target) =>
        works(a.command.context, a.shortcut, target) &&
        works(b.command.context, b.shortcut, target),
    )
  );
}

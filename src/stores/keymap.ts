import { create } from "zustand";
import { buildKeymap, type Keymap, shortcutsOf } from "../commands/keymap";
import { COMMANDS, type Command, type CommandId } from "../commands/registry";
import { commands } from "../ipc";
import { strings } from "../strings";

interface KeymapState {
  /** The shortcuts in force: the defaults until the user's changes arrive, then with them. */
  keymap: Keymap<Command>;
}

/** The window's keymap: the registry's defaults and the shortcuts the user changed. */
export const useKeymap = create<KeymapState>(() => ({ keymap: buildKeymap(COMMANDS) }));

/**
 * Loads the shortcuts the user changed (`keybindings` in `jerry.db`). Call once per window, at
 * start; until they arrive, and if they can't be read, the defaults work.
 */
export async function loadKeymap(): Promise<void> {
  const result = await commands.keybindingsGet();
  if (result.status === "ok") {
    useKeymap.setState({ keymap: buildKeymap(COMMANDS, result.data) });
  }
}

/** A command's first shortcut in force, as people read it ("Ctrl+Shift+N"). */
export function useShortcut(id: CommandId): string | undefined {
  return useKeymap((state) => shortcutsOf(state.keymap, id)[0]);
}

/** A command's title with its shortcut, for tooltips and screen readers: "Back (Alt+←)". */
export function useCommandLabel(id: CommandId): string {
  return strings.withShortcut(strings.commands[id], useShortcut(id));
}

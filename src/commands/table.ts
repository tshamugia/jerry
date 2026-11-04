import { strings } from "../strings";
import { type Keymap, shortcutsOf } from "./keymap";
import {
  COMMANDS,
  type Command,
  type CommandGroup,
  type CommandId,
  commandTitle,
  isBuilt,
} from "./registry";

export interface ShortcutRow {
  id: CommandId;
  title: string;
  /** The shortcuts in force, as people read them. */
  keys: string[];
}

export interface ShortcutGroup {
  group: CommandGroup;
  title: string;
  rows: ShortcutRow[];
}

/**
 * The shortcut table (F1), straight from the registry: every built command that has a shortcut,
 * by group, in the registry's order. Commands not built yet stay out, so it never offers a key
 * that does nothing.
 */
export function shortcutTable(
  keymap: Keymap<Command>,
  commands: readonly Command[] = COMMANDS,
): ShortcutGroup[] {
  const groups = Object.keys(strings.commandGroups) as CommandGroup[];
  return groups
    .map((group) => ({
      group,
      title: strings.commandGroups[group],
      rows: commands
        .filter((command) => command.group === group && isBuilt(command))
        .map((command) => ({
          id: command.id,
          title: commandTitle(command),
          keys: shortcutsOf(keymap, command.id),
        }))
        .filter((row) => row.keys.length > 0),
    }))
    .filter((group) => group.rows.length > 0);
}

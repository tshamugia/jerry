import { useEffect } from "react";
import { isAltGraph, type KeyInput, matches, parseShortcut, type Shortcut } from "./keymap";

/**
 * Where a command's shortcut works. `list` shortcuts stay quiet while the user types in a text
 * field, so Ctrl+C and Ctrl+V there copy and paste text; `global` ones work everywhere.
 */
export type CommandContext = "global" | "list";

export interface Command {
  id: string;
  title: string;
  /** The default shortcut with US key names ("Ctrl+Shift+N"). */
  shortcut: string;
  context: CommandContext;
  run: () => void;
}

interface Binding {
  command: Command;
  shortcut: Shortcut;
}

/** Looks up the command for a key press. */
export type Keymap = (event: KeyInput, editing: boolean) => Command | undefined;

export function createKeymap(commands: readonly Command[]): Keymap {
  const bindings: Binding[] = commands.map((command) => ({
    command,
    shortcut: parseShortcut(command.shortcut),
  }));
  return (event, editing) => {
    // AltGr arrives as Ctrl+Alt; in a text field it types a character.
    if (editing && isAltGraph(event)) return undefined;
    return bindings.find(
      ({ command, shortcut }) =>
        matches(event, shortcut) && (command.context === "global" || !editing),
    )?.command;
  };
}

/** Whether keys pressed at `target` go into a text field. */
export function isEditing(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    (target.isContentEditable || target.matches("input, textarea, select"))
  );
}

/**
 * Runs the matching command on keydown anywhere in the window. `onRun` sees every command run
 * with the event that ran it (the spike's log).
 */
export function useKeymap(
  commands: readonly Command[],
  onRun?: (command: Command, event: KeyboardEvent) => void,
): void {
  useEffect(() => {
    const keymap = createKeymap(commands);
    const onKeyDown = (event: KeyboardEvent) => {
      const command = keymap(event, isEditing(event.target));
      if (!command) return;
      event.preventDefault();
      command.run();
      onRun?.(command, event);
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [commands, onRun]);
}

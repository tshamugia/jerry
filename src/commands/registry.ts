import { focusFilter } from "../features/filter/focus";
import {
  canGoBack,
  canGoForward,
  canGoUp,
  goBack,
  goForward,
  goUp,
  hasTab,
  openChosen,
  refresh,
} from "../features/navigation/commands";
import {
  clearSelection,
  invertSelection,
  selectAllRows,
  toggleFocusedRow,
} from "../features/selection/commands";
import {
  canCloseTab,
  canReopenTab,
  closeActiveTab,
  newTab,
  nextTab,
  previousTab,
  reopenTab,
  showTab,
} from "../features/tabs/commands";
import { openOverlay } from "../stores/overlay";
import { toggleDensity, toggleHidden, useSettings } from "../stores/settings";
import { strings } from "../strings";
import type { Bindable, CommandContext } from "./keymap";

/** A command's id: the key of its title in `strings.commands`. */
export type CommandId = keyof typeof strings.commands;

/** A command's group in the shortcut table: the key of its title in `strings.commandGroups`. */
export type CommandGroup = keyof typeof strings.commandGroups;

/**
 * Something the user can do. The keyboard, the palette, menus, the toolbar, the shortcut editor
 * and the shortcut table (F1) all take commands from this one registry. Adding a command is one
 * entry in `COMMANDS` plus its title in `strings.commands`; it then shows in the shortcut table
 * by itself.
 */
export interface Command extends Bindable {
  id: CommandId;
  group: CommandGroup;
  /** The default shortcuts with US key names ("Ctrl+Shift+N", "Alt+←", "?"); the user can change them. */
  keys: readonly string[];
  context: CommandContext;
  /** Holding the shortcut down runs it again (Back, Undo); others run once per press. */
  repeat?: boolean;
  /** Whether it can run now. Without it, a command with a handler always can. */
  enabled?: () => boolean;
  /**
   * Runs it. A command without a handler isn't built yet: its shortcut stays reserved, but it is
   * disabled and left out of the shortcut table.
   */
  run?: () => void;
}

const tabs = [1, 2, 3, 4, 5, 6, 7, 8, 9] as const;

const settingsLoaded = () => useSettings.getState().settings !== undefined;

/** Every command, in the PRD's order («კლავიატურა, command palette და context menu»). */
export const COMMANDS: readonly Command[] = [
  // Navigation. Back, Forward and Up work in a text field too: it keeps Backspace.
  { id: "addressBar", group: "navigation", keys: ["Ctrl+L"], context: "global" },
  {
    id: "back",
    group: "navigation",
    keys: ["Alt+←", "Backspace"],
    context: "global",
    repeat: true,
    enabled: canGoBack,
    run: goBack,
  },
  {
    id: "forward",
    group: "navigation",
    keys: ["Alt+→"],
    context: "global",
    repeat: true,
    enabled: canGoForward,
    run: goForward,
  },
  {
    id: "up",
    group: "navigation",
    keys: ["Alt+↑"],
    context: "global",
    repeat: true,
    enabled: canGoUp,
    run: goUp,
  },
  { id: "goToFolder", group: "navigation", keys: ["Ctrl+P"], context: "global" },
  {
    id: "refresh",
    group: "navigation",
    keys: ["Ctrl+R"],
    context: "global",
    enabled: hasTab,
    run: refresh,
  },
  // Enter opens the selection, or the focused row: a folder in the tab, files in their programs.
  {
    id: "open",
    group: "navigation",
    keys: ["Enter"],
    context: "list",
    enabled: hasTab,
    run: () => openChosen(),
  },
  {
    id: "openInNewTab",
    group: "navigation",
    keys: ["Ctrl+Enter"],
    context: "list",
    enabled: hasTab,
    run: () => openChosen(true),
  },

  // Tabs.
  { id: "newTab", group: "tabs", keys: ["Ctrl+T"], context: "global", run: () => newTab() },
  {
    id: "closeTab",
    group: "tabs",
    keys: ["Ctrl+W"],
    context: "global",
    enabled: canCloseTab,
    run: closeActiveTab,
  },
  {
    id: "reopenTab",
    group: "tabs",
    keys: ["Ctrl+Shift+T"],
    context: "global",
    enabled: canReopenTab,
    run: reopenTab,
  },
  {
    id: "nextTab",
    group: "tabs",
    keys: ["Ctrl+Tab"],
    context: "global",
    repeat: true,
    run: nextTab,
  },
  {
    id: "previousTab",
    group: "tabs",
    keys: ["Ctrl+Shift+Tab"],
    context: "global",
    repeat: true,
    run: previousTab,
  },
  ...tabs.map(
    (n): Command => ({
      id: `tab${n}`,
      group: "tabs",
      keys: [`Ctrl+${n}`],
      context: "global",
      run: showTab(n),
    }),
  ),

  // Windows and panes. Copy and move act on the selection, so they belong to the list.
  { id: "newWindow", group: "panes", keys: ["Ctrl+N"], context: "global" },
  { id: "dualPane", group: "panes", keys: ["F3"], context: "global" },
  { id: "switchPane", group: "panes", keys: ["Tab"], context: "list" },
  { id: "copyToOtherPane", group: "panes", keys: ["F5"], context: "list" },
  { id: "moveToOtherPane", group: "panes", keys: ["F6"], context: "list" },
  { id: "copyTo", group: "panes", keys: ["Shift+F5"], context: "list" },
  { id: "moveTo", group: "panes", keys: ["Shift+F6"], context: "list" },
  { id: "comparePanes", group: "panes", keys: ["Ctrl+F9"], context: "global" },

  // View.
  { id: "detailsView", group: "view", keys: ["Ctrl+Shift+1"], context: "global" },
  { id: "listView", group: "view", keys: ["Ctrl+Shift+2"], context: "global" },
  { id: "gridView", group: "view", keys: ["Ctrl+Shift+3"], context: "global" },
  {
    id: "toggleHidden",
    group: "view",
    keys: ["Ctrl+H"],
    context: "global",
    enabled: settingsLoaded,
    run: toggleHidden,
  },
  { id: "toggleSidebar", group: "view", keys: ["Ctrl+B"], context: "global" },
  { id: "togglePreview", group: "view", keys: ["Alt+P"], context: "global" },
  {
    id: "toggleDensity",
    group: "view",
    keys: ["Ctrl+Alt+D"],
    context: "global",
    enabled: settingsLoaded,
    run: toggleDensity,
  },

  // Selection: on the active pane's list, only the rows it shows.
  { id: "selectAll", group: "selection", keys: ["Ctrl+A"], context: "list", run: selectAllRows },
  {
    id: "invertSelection",
    group: "selection",
    keys: ["Ctrl+I"],
    context: "list",
    run: invertSelection,
  },
  {
    id: "selectByPattern",
    group: "selection",
    keys: ["Ctrl+Shift+A"],
    context: "list",
    run: () => openOverlay("selectByPattern"),
  },
  {
    id: "toggleSelection",
    group: "selection",
    keys: ["Ctrl+Space"],
    context: "list",
    run: toggleFocusedRow,
  },
  { id: "clearSelection", group: "selection", keys: ["Esc"], context: "list", run: clearSelection },

  // Files.
  { id: "newFolder", group: "files", keys: ["Ctrl+Shift+N"], context: "list" },
  { id: "newFile", group: "files", keys: ["Ctrl+Alt+N"], context: "list" },
  { id: "rename", group: "files", keys: ["F2"], context: "list" },
  { id: "duplicate", group: "files", keys: ["Ctrl+D"], context: "list" },
  { id: "recycle", group: "files", keys: ["Del"], context: "list" },
  { id: "deletePermanently", group: "files", keys: ["Shift+Del"], context: "list" },
  { id: "properties", group: "files", keys: ["Alt+Enter"], context: "list" },
  { id: "copyPath", group: "files", keys: ["Ctrl+Shift+C"], context: "list" },
  { id: "copyName", group: "files", keys: ["Ctrl+Alt+C"], context: "list" },

  // Inline rename (PRD «ფაილური ოპერაციები»).
  { id: "renameSave", group: "rename", keys: ["Enter"], context: "rename" },
  { id: "renameCancel", group: "rename", keys: ["Esc"], context: "rename" },
  { id: "renameNext", group: "rename", keys: ["Tab"], context: "rename" },

  // Clipboard.
  { id: "copy", group: "clipboard", keys: ["Ctrl+C"], context: "list" },
  { id: "cut", group: "clipboard", keys: ["Ctrl+X"], context: "list" },
  { id: "paste", group: "clipboard", keys: ["Ctrl+V"], context: "list" },

  // Search.
  { id: "filter", group: "search", keys: ["Ctrl+F"], context: "global", run: focusFilter },
  { id: "search", group: "search", keys: ["Ctrl+Shift+F"], context: "global" },
  { id: "openFileLocation", group: "search", keys: ["Ctrl+Shift+E"], context: "list" },

  // Preview.
  { id: "quickLook", group: "preview", keys: ["Space"], context: "list" },

  // History.
  { id: "undo", group: "history", keys: ["Ctrl+Z"], context: "list", repeat: true },
  { id: "redo", group: "history", keys: ["Ctrl+Y"], context: "list", repeat: true },

  // Other.
  { id: "addFavorite", group: "other", keys: ["Ctrl+Shift+D"], context: "global" },
  { id: "palette", group: "other", keys: ["Ctrl+Shift+P"], context: "global" },
  { id: "settings", group: "other", keys: ["Ctrl+,"], context: "global" },
  {
    id: "shortcuts",
    group: "other",
    keys: ["F1", "?"],
    context: "global",
    run: () => openOverlay("shortcuts"),
  },
];

const byId = new Map(COMMANDS.map((command) => [command.id, command]));

export function commandById(id: CommandId): Command | undefined {
  return byId.get(id);
}

/** The command's title, as the palette, menus and the shortcut table show it. */
export function commandTitle(command: Command): string {
  return strings.commands[command.id];
}

/** Whether the command is built: it has a handler. */
export function isBuilt(command: Command): boolean {
  return command.run !== undefined;
}

/** Whether the command can run now. */
export function isEnabled(command: Command): boolean {
  return isBuilt(command) && (command.enabled?.() ?? true);
}

/** Runs the command if it is enabled; returns whether it ran. */
export function runCommand(command: Command): boolean {
  if (!isEnabled(command)) {
    return false;
  }
  command.run?.();
  return true;
}

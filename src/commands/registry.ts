import {
  canCopy,
  canPaste,
  copyEntries,
  copyText,
  pasteEntries,
} from "../features/clipboard/clipboard";
import { canDelete, startDelete } from "../features/delete/flow";
import { focusFilter } from "../features/filter/focus";
import { canEditAddress, editAddress } from "../features/navigation/address";
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
  canOpenInOtherPane,
  canSwitchPane,
  sameFolderInOtherPane,
  switchPane,
  toggleDualPane,
} from "../features/navigation/panes";
import {
  canOpenTerminal,
  canShowInExplorer,
  openInTerminal,
  showInExplorer,
} from "../features/navigation/shell";
import {
  canCreateFolder,
  cancelRename,
  canRename,
  isRenaming,
  newFolder,
  saveRename,
  startRename,
} from "../features/rename/rename";
import {
  canOpenContainingFolder,
  canSearch,
  canStopSearch,
  openContainingFolder,
  openSearch,
  stopSearch,
} from "../features/search/search";
import {
  clearSelection,
  invertSelection,
  selectAllRows,
  toggleFocusedRow,
} from "../features/selection/commands";
import { exitJerry } from "../features/session/instance";
import { addCurrentFolder, canAddFavorite } from "../features/sidebar/favorites";
import { toggleSidebar } from "../features/sidebar/store";
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
  {
    id: "addressBar",
    group: "navigation",
    keys: ["Ctrl+L"],
    context: "global",
    enabled: canEditAddress,
    run: () => editAddress(),
  },
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
  {
    id: "dualPane",
    group: "panes",
    keys: ["F3"],
    context: "global",
    enabled: settingsLoaded,
    run: toggleDualPane,
  },
  // From a file list only: elsewhere Tab moves through the controls.
  {
    id: "switchPane",
    group: "panes",
    keys: ["Tab"],
    context: "list",
    enabled: canSwitchPane,
    run: switchPane,
  },
  // No shortcut in the PRD: the tab menu has it, and the palette (v1.0).
  {
    id: "sameFolderInOtherPane",
    group: "panes",
    keys: [],
    context: "global",
    enabled: canOpenInOtherPane,
    run: sameFolderInOtherPane,
  },
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
  {
    id: "toggleSidebar",
    group: "view",
    keys: ["Ctrl+B"],
    context: "global",
    enabled: settingsLoaded,
    run: toggleSidebar,
  },
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
  {
    id: "newFolder",
    group: "files",
    keys: ["Ctrl+Shift+N"],
    context: "list",
    enabled: canCreateFolder,
    run: () => void newFolder(),
  },
  { id: "newFile", group: "files", keys: ["Ctrl+Alt+N"], context: "list" },
  {
    id: "rename",
    group: "files",
    keys: ["F2"],
    context: "list",
    enabled: canRename,
    run: startRename,
  },
  { id: "duplicate", group: "files", keys: ["Ctrl+D"], context: "list" },
  {
    id: "recycle",
    group: "files",
    keys: ["Del"],
    context: "list",
    enabled: canDelete,
    run: () => void startDelete(false),
  },
  {
    id: "deletePermanently",
    group: "files",
    keys: ["Shift+Del"],
    context: "list",
    enabled: canDelete,
    run: () => void startDelete(true),
  },
  { id: "properties", group: "files", keys: ["Alt+Enter"], context: "list" },
  {
    id: "copyPath",
    group: "files",
    keys: ["Ctrl+Shift+C"],
    context: "list",
    enabled: canCopy,
    run: () => void copyText("path"),
  },
  {
    id: "copyName",
    group: "files",
    keys: ["Ctrl+Alt+C"],
    context: "list",
    enabled: canCopy,
    run: () => void copyText("name"),
  },
  // No shortcuts in the PRD: the context menu has them, and the palette (v1.0).
  {
    id: "openInTerminal",
    group: "files",
    keys: [],
    context: "list",
    enabled: canOpenTerminal,
    run: () => void openInTerminal(),
  },
  {
    id: "showInExplorer",
    group: "files",
    keys: [],
    context: "list",
    enabled: canShowInExplorer,
    run: () => void showInExplorer(),
  },
  // Windows' own context menu ("Show more options"), last in Jerry's (v1.0).
  { id: "windowsOptions", group: "files", keys: [], context: "list" },

  // Inline rename (PRD «ფაილური ოპერაციები»).
  {
    id: "renameSave",
    group: "rename",
    keys: ["Enter"],
    context: "rename",
    enabled: isRenaming,
    run: () => void saveRename("close"),
  },
  {
    id: "renameCancel",
    group: "rename",
    keys: ["Esc"],
    context: "rename",
    enabled: isRenaming,
    run: cancelRename,
  },
  {
    id: "renameNext",
    group: "rename",
    keys: ["Tab"],
    context: "rename",
    enabled: isRenaming,
    run: () => void saveRename("next"),
  },

  // Clipboard.
  {
    id: "copy",
    group: "clipboard",
    keys: ["Ctrl+C"],
    context: "list",
    enabled: canCopy,
    run: () => void copyEntries(false),
  },
  {
    id: "cut",
    group: "clipboard",
    keys: ["Ctrl+X"],
    context: "list",
    enabled: canCopy,
    run: () => void copyEntries(true),
  },
  {
    id: "paste",
    group: "clipboard",
    keys: ["Ctrl+V"],
    context: "list",
    enabled: canPaste,
    run: () => void pasteEntries(),
  },

  // Search.
  { id: "filter", group: "search", keys: ["Ctrl+F"], context: "global", run: focusFilter },
  {
    id: "search",
    group: "search",
    keys: ["Ctrl+Shift+F"],
    context: "global",
    enabled: canSearch,
    run: openSearch,
  },
  // Search results only: the folder of each chosen result, in a new tab, with it selected.
  {
    id: "openFileLocation",
    group: "search",
    keys: ["Ctrl+Shift+E"],
    context: "list",
    enabled: canOpenContainingFolder,
    run: openContainingFolder,
  },
  // No shortcut in the PRD: the results bar has the button, and the palette (v1.0).
  {
    id: "stopSearch",
    group: "search",
    keys: [],
    context: "global",
    enabled: canStopSearch,
    run: stopSearch,
  },

  // Preview.
  { id: "quickLook", group: "preview", keys: ["Space"], context: "list" },

  // History.
  { id: "undo", group: "history", keys: ["Ctrl+Z"], context: "list", repeat: true },
  { id: "redo", group: "history", keys: ["Ctrl+Y"], context: "list", repeat: true },

  // Other.
  // The folder the active pane shows; a folder in the list from its context menu.
  {
    id: "addFavorite",
    group: "other",
    keys: ["Ctrl+Shift+D"],
    context: "global",
    enabled: canAddFavorite,
    run: () => void addCurrentFolder(),
  },
  { id: "palette", group: "other", keys: ["Ctrl+Shift+P"], context: "global" },
  { id: "settings", group: "other", keys: ["Ctrl+,"], context: "global" },
  {
    id: "shortcuts",
    group: "other",
    keys: ["F1", "?"],
    context: "global",
    run: () => openOverlay("shortcuts"),
  },
  // Closing the last window only hides Jerry while it keeps running in the background (#43).
  { id: "exitJerry", group: "other", keys: ["Ctrl+Q"], context: "global", run: exitJerry },
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

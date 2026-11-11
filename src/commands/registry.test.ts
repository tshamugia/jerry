import { describe, expect, it } from "vitest";
import { strings } from "../strings";
import { buildKeymap, conflicts, shortcutsOf } from "./keymap";
import { COMMANDS, type Command, commandTitle, isBuilt } from "./registry";
import { parseShortcut } from "./shortcut";
import { shortcutTable } from "./table";

const keymap = buildKeymap(COMMANDS);

/** The PRD's shortcut table («ძირითადი shortcut-ები»), shortcut by shortcut. */
const PRD: Record<string, string> = {
  "Ctrl+L": "addressBar",
  "Alt+←": "back",
  "Alt+→": "forward",
  "Alt+↑": "up",
  Backspace: "back",
  "Ctrl+P": "goToFolder",
  "Ctrl+R": "refresh",
  "Ctrl+T": "newTab",
  "Ctrl+W": "closeTab",
  "Ctrl+Shift+T": "reopenTab",
  "Ctrl+Tab": "nextTab",
  "Ctrl+1": "tab1",
  "Ctrl+9": "tab9",
  "Ctrl+N": "newWindow",
  F3: "dualPane",
  Tab: "switchPane",
  F5: "copyToOtherPane",
  F6: "moveToOtherPane",
  "Shift+F5": "copyTo",
  "Shift+F6": "moveTo",
  "Ctrl+F9": "comparePanes",
  "Ctrl+Shift+1": "detailsView",
  "Ctrl+Shift+2": "listView",
  "Ctrl+Shift+3": "gridView",
  "Ctrl+H": "toggleHidden",
  "Ctrl+B": "toggleSidebar",
  "Alt+P": "togglePreview",
  "Ctrl+Alt+D": "toggleDensity",
  "Ctrl+A": "selectAll",
  "Ctrl+I": "invertSelection",
  "Ctrl+Shift+A": "selectByPattern",
  "Ctrl+Shift+N": "newFolder",
  "Ctrl+Alt+N": "newFile",
  F2: "rename",
  "Ctrl+D": "duplicate",
  Del: "recycle",
  "Shift+Del": "deletePermanently",
  "Alt+Enter": "properties",
  "Ctrl+Shift+C": "copyPath",
  "Ctrl+Alt+C": "copyName",
  "Ctrl+C": "copy",
  "Ctrl+X": "cut",
  "Ctrl+V": "paste",
  "Ctrl+F": "filter",
  "Ctrl+Shift+F": "search",
  "Ctrl+Shift+E": "openFileLocation",
  Space: "quickLook",
  "Ctrl+Z": "undo",
  "Ctrl+Y": "redo",
  "Ctrl+Shift+D": "addFavorite",
  "Ctrl+Shift+P": "palette",
  "Ctrl+,": "settings",
  "Ctrl+Q": "exitJerry",
  F1: "shortcuts",
  "?": "shortcuts",
};

describe("the default keymap", () => {
  it("has every shortcut of the PRD's table", () => {
    for (const [key, id] of Object.entries(PRD)) {
      expect(shortcutsOf(keymap, id), key).toContain(key);
    }
  });

  it("is written in valid shortcuts, one command per id", () => {
    for (const command of COMMANDS) {
      for (const key of command.keys) {
        expect(parseShortcut(key), `${command.id}: ${key}`).toBeDefined();
      }
    }
    const ids = COMMANDS.map((command) => command.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("has no conflicts", () => {
    const pairs = conflicts(keymap).map(([a, b]) => `${a.command.id} + ${b.command.id}`);
    expect(pairs).toEqual([]);
  });

  it("gives every command a title and a known group", () => {
    for (const command of COMMANDS) {
      expect(commandTitle(command), command.id).not.toBe("");
      expect(strings.commandGroups[command.group], command.id).toBeDefined();
    }
  });

  it("keeps commands that aren't built yet, disabled", () => {
    const duplicate = COMMANDS.find((command) => command.id === "duplicate");
    expect(duplicate && isBuilt(duplicate)).toBe(false);
    expect(shortcutsOf(keymap, "duplicate")).toEqual(["Ctrl+D"]);
  });
});

describe("the shortcut table", () => {
  it("shows the built commands, by group", () => {
    expect(shortcutTable(keymap)).toEqual([
      {
        group: "navigation",
        title: "Navigation",
        rows: [
          { id: "addressBar", title: "Edit address", keys: ["Ctrl+L"] },
          { id: "back", title: "Back", keys: ["Alt+←", "Backspace"] },
          { id: "forward", title: "Forward", keys: ["Alt+→"] },
          { id: "up", title: "Up", keys: ["Alt+↑"] },
          { id: "refresh", title: "Refresh", keys: ["Ctrl+R"] },
          { id: "open", title: "Open", keys: ["Enter"] },
          { id: "openInNewTab", title: "Open in new tab", keys: ["Ctrl+Enter"] },
        ],
      },
      {
        group: "tabs",
        title: "Tabs",
        rows: [
          { id: "newTab", title: "New tab", keys: ["Ctrl+T"] },
          { id: "closeTab", title: "Close tab", keys: ["Ctrl+W"] },
          { id: "reopenTab", title: "Reopen closed tab", keys: ["Ctrl+Shift+T"] },
          { id: "nextTab", title: "Next tab", keys: ["Ctrl+Tab"] },
          { id: "previousTab", title: "Previous tab", keys: ["Ctrl+Shift+Tab"] },
          { id: "tab1", title: "Tab 1", keys: ["Ctrl+1"] },
          { id: "tab2", title: "Tab 2", keys: ["Ctrl+2"] },
          { id: "tab3", title: "Tab 3", keys: ["Ctrl+3"] },
          { id: "tab4", title: "Tab 4", keys: ["Ctrl+4"] },
          { id: "tab5", title: "Tab 5", keys: ["Ctrl+5"] },
          { id: "tab6", title: "Tab 6", keys: ["Ctrl+6"] },
          { id: "tab7", title: "Tab 7", keys: ["Ctrl+7"] },
          { id: "tab8", title: "Tab 8", keys: ["Ctrl+8"] },
          { id: "tab9", title: "Tab 9", keys: ["Ctrl+9"] },
        ],
      },
      {
        group: "panes",
        title: "Windows and panes",
        rows: [
          { id: "dualPane", title: "Dual pane", keys: ["F3"] },
          { id: "switchPane", title: "Switch pane", keys: ["Tab"] },
        ],
      },
      {
        group: "view",
        title: "View",
        rows: [
          { id: "toggleHidden", title: "Show or hide hidden files", keys: ["Ctrl+H"] },
          { id: "toggleSidebar", title: "Show or hide sidebar", keys: ["Ctrl+B"] },
          { id: "toggleDensity", title: "Row density", keys: ["Ctrl+Alt+D"] },
        ],
      },
      {
        group: "selection",
        title: "Selection",
        rows: [
          { id: "selectAll", title: "Select all", keys: ["Ctrl+A"] },
          { id: "invertSelection", title: "Invert selection", keys: ["Ctrl+I"] },
          { id: "selectByPattern", title: "Select by pattern…", keys: ["Ctrl+Shift+A"] },
          { id: "toggleSelection", title: "Toggle focused item", keys: ["Ctrl+Space"] },
          { id: "clearSelection", title: "Clear selection", keys: ["Esc"] },
        ],
      },
      {
        group: "files",
        title: "Files",
        rows: [
          { id: "newFolder", title: "New folder", keys: ["Ctrl+Shift+N"] },
          { id: "rename", title: "Rename", keys: ["F2"] },
          { id: "recycle", title: "Move to Recycle Bin", keys: ["Del"] },
          { id: "deletePermanently", title: "Delete permanently", keys: ["Shift+Del"] },
          { id: "copyPath", title: "Copy path", keys: ["Ctrl+Shift+C"] },
          { id: "copyName", title: "Copy name", keys: ["Ctrl+Alt+C"] },
        ],
      },
      {
        group: "rename",
        title: "Renaming",
        rows: [
          { id: "renameSave", title: "Save name", keys: ["Enter"] },
          { id: "renameCancel", title: "Cancel rename", keys: ["Esc"] },
          { id: "renameNext", title: "Rename next file", keys: ["Tab"] },
        ],
      },
      {
        group: "clipboard",
        title: "Clipboard",
        rows: [
          { id: "copy", title: "Copy", keys: ["Ctrl+C"] },
          { id: "cut", title: "Cut", keys: ["Ctrl+X"] },
          { id: "paste", title: "Paste", keys: ["Ctrl+V"] },
        ],
      },
      {
        group: "search",
        title: "Search",
        rows: [
          { id: "filter", title: "Filter", keys: ["Ctrl+F"] },
          { id: "search", title: "Search", keys: ["Ctrl+Shift+F"] },
          { id: "openFileLocation", title: "Open file location", keys: ["Ctrl+Shift+E"] },
        ],
      },
      {
        group: "other",
        title: "Other",
        rows: [
          { id: "addFavorite", title: "Add to favorites", keys: ["Ctrl+Shift+D"] },
          { id: "shortcuts", title: "Keyboard shortcuts", keys: ["F1", "?"] },
          { id: "exitJerry", title: "Exit Jerry", keys: ["Ctrl+Q"] },
        ],
      },
    ]);
  });

  it("shows a new command by itself, from its one entry", () => {
    const newFile: Command = {
      id: "newFile",
      group: "files",
      keys: ["Ctrl+Alt+N"],
      context: "list",
      run: () => {},
    };
    const commands = [...COMMANDS.filter((command) => command.id !== "newFile"), newFile];
    const table = shortcutTable(buildKeymap(commands), commands);
    expect(table.map((group) => group.group)).toEqual([
      "navigation",
      "tabs",
      "panes",
      "view",
      "selection",
      "files",
      "rename",
      "clipboard",
      "search",
      "other",
    ]);
    expect(table[5]?.rows.map((row) => row.id)).toEqual([
      "newFolder",
      "rename",
      "recycle",
      "deletePermanently",
      "copyPath",
      "copyName",
      "newFile",
    ]);
  });

  it("shows the user's shortcuts and leaves out removed ones", () => {
    const changed = buildKeymap(COMMANDS, [
      { command: "toggleDensity", shortcut: "Ctrl+Alt+R" },
      { command: "shortcuts", shortcut: null },
    ]);
    expect(shortcutTable(changed)).toEqual([
      {
        group: "navigation",
        title: "Navigation",
        rows: [
          { id: "addressBar", title: "Edit address", keys: ["Ctrl+L"] },
          { id: "back", title: "Back", keys: ["Alt+←", "Backspace"] },
          { id: "forward", title: "Forward", keys: ["Alt+→"] },
          { id: "up", title: "Up", keys: ["Alt+↑"] },
          { id: "refresh", title: "Refresh", keys: ["Ctrl+R"] },
          { id: "open", title: "Open", keys: ["Enter"] },
          { id: "openInNewTab", title: "Open in new tab", keys: ["Ctrl+Enter"] },
        ],
      },
      {
        group: "tabs",
        title: "Tabs",
        rows: [
          { id: "newTab", title: "New tab", keys: ["Ctrl+T"] },
          { id: "closeTab", title: "Close tab", keys: ["Ctrl+W"] },
          { id: "reopenTab", title: "Reopen closed tab", keys: ["Ctrl+Shift+T"] },
          { id: "nextTab", title: "Next tab", keys: ["Ctrl+Tab"] },
          { id: "previousTab", title: "Previous tab", keys: ["Ctrl+Shift+Tab"] },
          { id: "tab1", title: "Tab 1", keys: ["Ctrl+1"] },
          { id: "tab2", title: "Tab 2", keys: ["Ctrl+2"] },
          { id: "tab3", title: "Tab 3", keys: ["Ctrl+3"] },
          { id: "tab4", title: "Tab 4", keys: ["Ctrl+4"] },
          { id: "tab5", title: "Tab 5", keys: ["Ctrl+5"] },
          { id: "tab6", title: "Tab 6", keys: ["Ctrl+6"] },
          { id: "tab7", title: "Tab 7", keys: ["Ctrl+7"] },
          { id: "tab8", title: "Tab 8", keys: ["Ctrl+8"] },
          { id: "tab9", title: "Tab 9", keys: ["Ctrl+9"] },
        ],
      },
      {
        group: "panes",
        title: "Windows and panes",
        rows: [
          { id: "dualPane", title: "Dual pane", keys: ["F3"] },
          { id: "switchPane", title: "Switch pane", keys: ["Tab"] },
        ],
      },
      {
        group: "view",
        title: "View",
        rows: [
          { id: "toggleHidden", title: "Show or hide hidden files", keys: ["Ctrl+H"] },
          { id: "toggleSidebar", title: "Show or hide sidebar", keys: ["Ctrl+B"] },
          { id: "toggleDensity", title: "Row density", keys: ["Ctrl+Alt+R"] },
        ],
      },
      {
        group: "selection",
        title: "Selection",
        rows: [
          { id: "selectAll", title: "Select all", keys: ["Ctrl+A"] },
          { id: "invertSelection", title: "Invert selection", keys: ["Ctrl+I"] },
          { id: "selectByPattern", title: "Select by pattern…", keys: ["Ctrl+Shift+A"] },
          { id: "toggleSelection", title: "Toggle focused item", keys: ["Ctrl+Space"] },
          { id: "clearSelection", title: "Clear selection", keys: ["Esc"] },
        ],
      },
      {
        group: "files",
        title: "Files",
        rows: [
          { id: "newFolder", title: "New folder", keys: ["Ctrl+Shift+N"] },
          { id: "rename", title: "Rename", keys: ["F2"] },
          { id: "recycle", title: "Move to Recycle Bin", keys: ["Del"] },
          { id: "deletePermanently", title: "Delete permanently", keys: ["Shift+Del"] },
          { id: "copyPath", title: "Copy path", keys: ["Ctrl+Shift+C"] },
          { id: "copyName", title: "Copy name", keys: ["Ctrl+Alt+C"] },
        ],
      },
      {
        group: "rename",
        title: "Renaming",
        rows: [
          { id: "renameSave", title: "Save name", keys: ["Enter"] },
          { id: "renameCancel", title: "Cancel rename", keys: ["Esc"] },
          { id: "renameNext", title: "Rename next file", keys: ["Tab"] },
        ],
      },
      {
        group: "clipboard",
        title: "Clipboard",
        rows: [
          { id: "copy", title: "Copy", keys: ["Ctrl+C"] },
          { id: "cut", title: "Cut", keys: ["Ctrl+X"] },
          { id: "paste", title: "Paste", keys: ["Ctrl+V"] },
        ],
      },
      {
        group: "search",
        title: "Search",
        rows: [
          { id: "filter", title: "Filter", keys: ["Ctrl+F"] },
          { id: "search", title: "Search", keys: ["Ctrl+Shift+F"] },
          { id: "openFileLocation", title: "Open file location", keys: ["Ctrl+Shift+E"] },
        ],
      },
      // Keyboard shortcuts is left out: its only shortcut was removed.
      {
        group: "other",
        title: "Other",
        rows: [
          { id: "addFavorite", title: "Add to favorites", keys: ["Ctrl+Shift+D"] },
          { id: "exitJerry", title: "Exit Jerry", keys: ["Ctrl+Q"] },
        ],
      },
    ]);
  });
});

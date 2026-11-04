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
    const selectAll = COMMANDS.find((command) => command.id === "selectAll");
    expect(selectAll && isBuilt(selectAll)).toBe(false);
    expect(shortcutsOf(keymap, "selectAll")).toEqual(["Ctrl+A"]);
  });
});

describe("the shortcut table", () => {
  it("shows the built commands, by group", () => {
    expect(shortcutTable(keymap)).toEqual([
      {
        group: "view",
        title: "View",
        rows: [{ id: "toggleDensity", title: "Row density", keys: ["Ctrl+Alt+D"] }],
      },
      {
        group: "other",
        title: "Other",
        rows: [{ id: "shortcuts", title: "Keyboard shortcuts", keys: ["F1", "?"] }],
      },
    ]);
  });

  it("shows a new command by itself, from its one entry", () => {
    const newFolder: Command = {
      id: "newFolder",
      group: "files",
      keys: ["Ctrl+Shift+N"],
      context: "list",
      run: () => {},
    };
    const commands = [...COMMANDS.filter((command) => command.id !== "newFolder"), newFolder];
    const table = shortcutTable(buildKeymap(commands), commands);
    expect(table.map((group) => group.group)).toEqual(["view", "files", "other"]);
    expect(table[1]?.rows).toEqual([
      { id: "newFolder", title: "New folder", keys: ["Ctrl+Shift+N"] },
    ]);
  });

  it("shows the user's shortcuts and leaves out removed ones", () => {
    const changed = buildKeymap(COMMANDS, [
      { command: "toggleDensity", shortcut: "Ctrl+Alt+R" },
      { command: "shortcuts", shortcut: null },
    ]);
    expect(shortcutTable(changed)).toEqual([
      {
        group: "view",
        title: "View",
        rows: [{ id: "toggleDensity", title: "Row density", keys: ["Ctrl+Alt+R"] }],
      },
    ]);
  });
});

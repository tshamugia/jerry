import { describe, expect, it } from "vitest";
import {
  type Bindable,
  buildKeymap,
  type CommandContext,
  conflicts,
  conflictsWith,
  type KeyTarget,
  lookup,
  shortcutsOf,
} from "./keymap";
import { type KeyInput, parseShortcut, type Shortcut } from "./shortcut";

/**
 * Key events as Windows delivers them. The Georgian values come from the "Georgian (QWERTY)"
 * layout (00010437) via ToUnicodeEx (spike #18): Ctrl+C types "ც", Shift+N types "N", AltGr+C
 * types "©".
 */
type Press = KeyInput & { key: string };

function press(code: string, key: string, mods: string[] = [], altGraph = false): Press {
  return {
    code,
    key,
    ctrlKey: mods.includes("ctrl"),
    shiftKey: mods.includes("shift"),
    altKey: mods.includes("alt"),
    metaKey: false,
    getModifierState: (name) => name === "AltGraph" && altGraph,
  };
}

const LAYOUTS = {
  "en-US": {
    copy: press("KeyC", "c", ["ctrl"]),
    paste: press("KeyV", "v", ["ctrl"]),
    filter: press("KeyF", "f", ["ctrl"]),
    newFolder: press("KeyN", "N", ["ctrl", "shift"]),
    help: press("Slash", "?", ["shift"]),
  },
  "ka-GE": {
    copy: press("KeyC", "ც", ["ctrl"]),
    paste: press("KeyV", "ვ", ["ctrl"]),
    filter: press("KeyF", "ფ", ["ctrl"]),
    newFolder: press("KeyN", "N", ["ctrl", "shift"]),
    help: press("Slash", "?", ["shift"]),
  },
} as const;

function command(id: string, keys: string[], context: CommandContext): Bindable {
  return { id, keys, context };
}

const COMMANDS = [
  command("copy", ["Ctrl+C"], "list"),
  command("paste", ["Ctrl+V"], "list"),
  command("filter", ["Ctrl+F"], "global"),
  command("newFolder", ["Ctrl+Shift+N"], "list"),
  command("copyName", ["Ctrl+Alt+C"], "list"),
  command("density", ["Ctrl+Alt+D"], "global"),
  command("back", ["Alt+←", "Backspace"], "global"),
  command("help", ["F1", "?"], "global"),
  command("quickLook", ["Space"], "list"),
  command("renameSave", ["Enter"], "rename"),
  command("renameCancel", ["Esc"], "rename"),
  command("dialogNext", ["↓"], "dialog"),
];

const WINDOW: KeyTarget = { scope: "window", focus: "other" };
const TEXT_FIELD: KeyTarget = { scope: "window", focus: "text" };
const BUTTON: KeyTarget = { scope: "window", focus: "button" };
const RENAME: KeyTarget = { scope: "rename", focus: "text" };
const DIALOG: KeyTarget = { scope: "dialog", focus: "other" };
const DIALOG_FIELD: KeyTarget = { scope: "dialog", focus: "text" };

const keymap = buildKeymap(COMMANDS);
const run = (event: KeyInput, target = WINDOW) => lookup(keymap, event, target)?.id;

function shortcut(text: string): Shortcut {
  const parsed = parseShortcut(text);
  if (!parsed) {
    throw new Error(`not a shortcut: ${text}`);
  }
  return parsed;
}

describe("matching", () => {
  for (const [layout, keys] of Object.entries(LAYOUTS)) {
    it(`runs Ctrl+C, Ctrl+V, Ctrl+F, Ctrl+Shift+N and ? on ${layout}`, () => {
      expect(run(keys.copy)).toBe("copy");
      expect(run(keys.paste)).toBe("paste");
      expect(run(keys.filter)).toBe("filter");
      expect(run(keys.newFolder)).toBe("newFolder");
      expect(run(keys.help)).toBe("help");
    });
  }

  it("would miss every Georgian press if it matched by key", () => {
    const byKey = (event: Press, letter: string) => event.ctrlKey && event.key === letter;
    const ka = LAYOUTS["ka-GE"];
    expect([byKey(ka.copy, "c"), byKey(ka.paste, "v"), byKey(ka.filter, "f")]).toEqual([
      false,
      false,
      false,
    ]);
  });

  it("needs exactly the shortcut's modifiers", () => {
    expect(run(press("KeyC", "ც"))).toBeUndefined();
    expect(run(press("KeyC", "ჩ", ["ctrl", "shift"]))).toBeUndefined();
    expect(run(press("KeyN", "ნ", ["ctrl"]))).toBeUndefined();
    expect(run({ ...press("KeyC", "c", ["ctrl"]), metaKey: true })).toBeUndefined();
  });

  it("runs a command from any of its shortcuts", () => {
    expect(run(press("ArrowLeft", "ArrowLeft", ["alt"]))).toBe("back");
    expect(run(press("Backspace", "Backspace"))).toBe("back");
    expect(run(press("F1", "F1"))).toBe("help");
    expect(shortcutsOf(keymap, "back")).toEqual(["Alt+←", "Backspace"]);
  });
});

describe("contexts", () => {
  it("leaves Ctrl+C and Ctrl+V to a text field, but not Ctrl+F", () => {
    const ka = LAYOUTS["ka-GE"];
    expect(run(ka.copy, TEXT_FIELD)).toBeUndefined();
    expect(run(ka.paste, TEXT_FIELD)).toBeUndefined();
    expect(run(ka.newFolder, TEXT_FIELD)).toBeUndefined();
    expect(run(ka.filter, TEXT_FIELD)).toBe("filter");
  });

  it("lets a text field keep its typing keys, even from global commands", () => {
    expect(run(press("Slash", "?", ["shift"]), TEXT_FIELD)).toBeUndefined();
    expect(run(press("Backspace", "Backspace"), TEXT_FIELD)).toBeUndefined();
    expect(run(press("F1", "F1"), TEXT_FIELD)).toBe("help");
    expect(run(press("ArrowLeft", "ArrowLeft", ["alt"]), TEXT_FIELD)).toBe("back");
  });

  it("lets AltGr type in a text field (AltGr+C is © on the Georgian layout)", () => {
    const altGrC = press("KeyC", "©", ["ctrl", "alt"], true);
    const altGrD = press("KeyD", "", ["ctrl", "alt"], true);
    expect(run(altGrC, TEXT_FIELD)).toBeUndefined();
    expect(run(altGrD, TEXT_FIELD)).toBeUndefined();
    expect(run(altGrD, RENAME)).toBeUndefined();
    // Outside a text field nothing is typed, so Ctrl+Alt+C and Ctrl+Alt+D still run.
    expect(run(altGrC)).toBe("copyName");
    expect(run(altGrD)).toBe("density");
    // Ctrl+Alt+D itself is a global shortcut and works in a text field.
    expect(run(press("KeyD", "დ", ["ctrl", "alt"]), TEXT_FIELD)).toBe("density");
  });

  it("lets a focused button keep Space and Enter", () => {
    expect(run(press("Space", " "))).toBe("quickLook");
    expect(run(press("Space", " "), BUTTON)).toBeUndefined();
    expect(run(LAYOUTS["ka-GE"].copy, BUTTON)).toBe("copy");
  });

  it("runs only rename commands and global ones in the rename field", () => {
    expect(run(press("Enter", "Enter"), RENAME)).toBe("renameSave");
    expect(run(press("Escape", "Escape"), RENAME)).toBe("renameCancel");
    expect(run(press("Enter", "Enter"))).toBeUndefined();
    expect(run(LAYOUTS["ka-GE"].copy, RENAME)).toBeUndefined();
    expect(run(LAYOUTS["ka-GE"].filter, RENAME)).toBe("filter");
  });

  it("runs only dialog commands in a dialog", () => {
    const down = press("ArrowDown", "ArrowDown");
    expect(run(down, DIALOG)).toBe("dialogNext");
    expect(run(down, DIALOG_FIELD)).toBe("dialogNext");
    expect(run(down)).toBeUndefined();
    expect(run(LAYOUTS["ka-GE"].filter, DIALOG)).toBeUndefined();
    expect(run(LAYOUTS["ka-GE"].copy, DIALOG)).toBeUndefined();
    expect(run(press("F1", "F1"), DIALOG)).toBeUndefined();
  });
});

describe("the user's shortcuts", () => {
  it("replace a command's defaults, or remove them", () => {
    const changed = buildKeymap(COMMANDS, [
      { command: "copy", shortcut: "Ctrl+Insert" },
      { command: "back", shortcut: null },
    ]);
    expect(lookup(changed, press("Insert", "Insert", ["ctrl"]), WINDOW)?.id).toBe("copy");
    expect(lookup(changed, LAYOUTS["ka-GE"].copy, WINDOW)).toBeUndefined();
    expect(lookup(changed, press("Backspace", "Backspace"), WINDOW)).toBeUndefined();
    expect(shortcutsOf(changed, "copy")).toEqual(["Ctrl+Ins"]);
    expect(shortcutsOf(changed, "back")).toEqual([]);
  });

  it("win over a default they collide with", () => {
    const changed = buildKeymap(COMMANDS, [{ command: "copyName", shortcut: "Ctrl+C" }]);
    expect(lookup(changed, LAYOUTS["ka-GE"].copy, WINDOW)?.id).toBe("copyName");
    expect(shortcutsOf(changed, "copy")).toEqual([]);
    expect(conflicts(changed)).toEqual([]);
    // Ctrl+F is global, Ctrl+C a list shortcut: both work in the list, so Ctrl+F is shadowed.
    const filter = buildKeymap(COMMANDS, [{ command: "copy", shortcut: "Ctrl+F" }]);
    expect(lookup(filter, LAYOUTS["en-US"].filter, WINDOW)?.id).toBe("copy");
    expect(shortcutsOf(filter, "filter")).toEqual([]);
  });

  it("leave a default alone where they can't both run", () => {
    // Enter saves a rename; a list command on Enter works only outside text fields.
    const changed = buildKeymap(COMMANDS, [{ command: "copy", shortcut: "Enter" }]);
    expect(shortcutsOf(changed, "renameSave")).toEqual(["Enter"]);
    expect(lookup(changed, press("Enter", "Enter"), RENAME)?.id).toBe("renameSave");
    expect(lookup(changed, press("Enter", "Enter"), WINDOW)?.id).toBe("copy");
  });

  it("are ignored for unknown commands and text that isn't a shortcut", () => {
    const changed = buildKeymap(COMMANDS, [
      { command: "noSuchCommand", shortcut: "Ctrl+K" },
      { command: "paste", shortcut: "Ctrl+ვ" },
    ]);
    expect(lookup(changed, press("KeyK", "კ", ["ctrl"]), WINDOW)).toBeUndefined();
    expect(shortcutsOf(changed, "paste")).toEqual(["Ctrl+V"]);
  });
});

describe("conflicts", () => {
  it("are two commands one key press could run", () => {
    const clash = buildKeymap([
      command("a", ["Ctrl+K"], "list"),
      command("b", ["Ctrl+K"], "global"),
      command("c", ["Ctrl+K"], "dialog"),
      command("d", ["Enter"], "list"),
      command("e", ["Enter"], "rename"),
      command("f", ["F2"], "list"),
      command("g", ["F2"], "rename"),
      command("h", ["Space"], "global"),
      command("i", ["Space"], "dialog"),
    ]);
    const pairs = conflicts(clash).map(([x, y]) => `${x.command.id}+${y.command.id}`);
    // a and b both run in the list; c only in a dialog. Enter in the list and in the rename
    // field never land on the same target; F2 and Space neither.
    expect(pairs).toEqual(["a+b"]);
  });

  it("show what a new shortcut would collide with, in the same context", () => {
    const copyName = COMMANDS.find((c) => c.id === "copyName");
    if (!copyName) throw new Error("missing");
    expect(conflictsWith(keymap, copyName, shortcut("Ctrl+F")).map((c) => c.id)).toEqual([
      "filter",
    ]);
    expect(conflictsWith(keymap, copyName, shortcut("Ctrl+Alt+C"))).toEqual([]);
    expect(conflictsWith(keymap, copyName, shortcut("Enter"))).toEqual([]);
    const renameCancel = COMMANDS.find((c) => c.id === "renameCancel");
    if (!renameCancel) throw new Error("missing");
    expect(conflictsWith(keymap, renameCancel, shortcut("Enter")).map((c) => c.id)).toEqual([
      "renameSave",
    ]);
  });
});

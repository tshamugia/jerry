import { fireEvent, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { type KeyInput, matches, parseShortcut } from "./keymap";
import { type Command, createKeymap, useKeymap } from "./registry";

/**
 * Key events as Windows delivers them. The Georgian values come from the "Georgian (QWERTY)"
 * layout (00010437) via ToUnicodeEx: Ctrl+C types "ც", Shift+N types "N", AltGr+C types "©".
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
  },
  "ka-GE": {
    copy: press("KeyC", "ც", ["ctrl"]),
    paste: press("KeyV", "ვ", ["ctrl"]),
    filter: press("KeyF", "ფ", ["ctrl"]),
    newFolder: press("KeyN", "N", ["ctrl", "shift"]),
  },
} as const;

function commands(): Command[] {
  return [
    { id: "copy", title: "Copy", shortcut: "Ctrl+C", context: "list", run: vi.fn() },
    { id: "paste", title: "Paste", shortcut: "Ctrl+V", context: "list", run: vi.fn() },
    { id: "filter", title: "Filter", shortcut: "Ctrl+F", context: "global", run: vi.fn() },
    {
      id: "newFolder",
      title: "New folder",
      shortcut: "Ctrl+Shift+N",
      context: "list",
      run: vi.fn(),
    },
    { id: "copyName", title: "Copy name", shortcut: "Ctrl+Alt+C", context: "list", run: vi.fn() },
  ];
}

describe("parseShortcut", () => {
  it("maps US key names to physical codes", () => {
    expect(parseShortcut("Ctrl+Shift+N")).toEqual({
      code: "KeyN",
      ctrl: true,
      shift: true,
      alt: false,
    });
    expect(parseShortcut("F5").code).toBe("F5");
    expect(parseShortcut("Ctrl+1").code).toBe("Digit1");
    expect(parseShortcut("Ctrl+,").code).toBe("Comma");
    expect(parseShortcut("Alt+←").code).toBe("ArrowLeft");
    expect(() => parseShortcut("Ctrl+ც")).toThrow();
  });
});

describe("keymap", () => {
  for (const [layout, keys] of Object.entries(LAYOUTS)) {
    it(`runs Ctrl+C, Ctrl+V, Ctrl+F and Ctrl+Shift+N on ${layout}`, () => {
      const keymap = createKeymap(commands());
      expect(keymap(keys.copy, false)?.id).toBe("copy");
      expect(keymap(keys.paste, false)?.id).toBe("paste");
      expect(keymap(keys.filter, false)?.id).toBe("filter");
      expect(keymap(keys.newFolder, false)?.id).toBe("newFolder");
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
    const keymap = createKeymap(commands());
    expect(keymap(press("KeyC", "ც"), false)).toBeUndefined();
    expect(keymap(press("KeyC", "ჩ", ["ctrl", "shift"]), false)).toBeUndefined();
    expect(keymap(press("KeyN", "ნ", ["ctrl"]), false)).toBeUndefined();
    expect(
      matches({ ...press("KeyC", "c", ["ctrl"]), metaKey: true }, parseShortcut("Ctrl+C")),
    ).toBe(false);
  });

  it("leaves Ctrl+C and Ctrl+V to a text field, but not Ctrl+F", () => {
    const keymap = createKeymap(commands());
    const ka = LAYOUTS["ka-GE"];
    expect(keymap(ka.copy, true)).toBeUndefined();
    expect(keymap(ka.paste, true)).toBeUndefined();
    expect(keymap(ka.filter, true)?.id).toBe("filter");
  });

  it("lets AltGr type in a text field (AltGr+C is © on the Georgian layout)", () => {
    const keymap = createKeymap(commands());
    const altGrC = press("KeyC", "©", ["ctrl", "alt"], true);
    expect(keymap(altGrC, true)).toBeUndefined();
    // Outside a text field nothing is typed, so Ctrl+Alt+C (copy name) still runs.
    expect(keymap(altGrC, false)?.id).toBe("copyName");
    expect(keymap(press("KeyC", "c", ["ctrl", "alt"]), true)?.id).toBeUndefined();
  });
});

function Harness({ list, onRun }: { list: Command[]; onRun: (c: Command) => void }) {
  useKeymap(list, onRun);
  return <input aria-label="filter" />;
}

describe("useKeymap", () => {
  it("runs the command for a Georgian keydown and prevents the default", () => {
    const list = commands();
    const onRun = vi.fn();
    const { getByLabelText } = render(<Harness list={list} onRun={onRun} />);

    const event = new KeyboardEvent("keydown", {
      ...LAYOUTS["ka-GE"].copy,
      bubbles: true,
      cancelable: true,
    });
    window.dispatchEvent(event);
    expect(list[0]?.run).toHaveBeenCalledOnce();
    expect(event.defaultPrevented).toBe(true);

    // In the text field Ctrl+C is the field's own copy.
    fireEvent.keyDown(getByLabelText("filter"), LAYOUTS["ka-GE"].copy);
    expect(list[0]?.run).toHaveBeenCalledOnce();
    fireEvent.keyDown(getByLabelText("filter"), LAYOUTS["ka-GE"].filter);
    expect(list[2]?.run).toHaveBeenCalledOnce();
    expect(onRun).toHaveBeenCalledTimes(2);
  });
});

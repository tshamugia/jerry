import { describe, expect, it } from "vitest";
import {
  formatShortcut,
  isButtonKey,
  isFieldKey,
  matches,
  parseShortcut,
  type Shortcut,
} from "./shortcut";

function parsed(text: string): Shortcut {
  const shortcut = parseShortcut(text);
  if (!shortcut) {
    throw new Error(`not a shortcut: ${text}`);
  }
  return shortcut;
}

describe("parseShortcut", () => {
  it("maps US key names to physical codes", () => {
    expect(parseShortcut("Ctrl+Shift+N")).toEqual({
      code: "KeyN",
      ctrl: true,
      shift: true,
      alt: false,
    });
    expect(parsed("F5").code).toBe("F5");
    expect(parsed("Ctrl+1").code).toBe("Digit1");
    expect(parsed("Ctrl+Shift+3").code).toBe("Digit3");
    expect(parsed("Ctrl+,").code).toBe("Comma");
    expect(parsed("Alt+←").code).toBe("ArrowLeft");
    expect(parsed("Del").code).toBe("Delete");
    expect(parsed("Esc").code).toBe("Escape");
    expect(parsed("Ctrl+Tab").code).toBe("Tab");
  });

  it("reads ? as Shift+/", () => {
    expect(parseShortcut("?")).toEqual(parseShortcut("Shift+/"));
  });

  it("takes other spellings and any case", () => {
    expect(parseShortcut("ctrl+shift+n")).toEqual(parseShortcut("Ctrl+Shift+N"));
    expect(parseShortcut("Alt+Left")).toEqual(parseShortcut("Alt+←"));
    expect(parseShortcut("Shift+Delete")).toEqual(parseShortcut("Shift+Del"));
  });

  it("rejects what isn't a shortcut", () => {
    for (const text of ["", "Ctrl+", "Ctrl+ც", "Win+E", "Ctrl+Ctrl+A", "Ctrl+F25", "Hyper+A"]) {
      expect(parseShortcut(text), text).toBeUndefined();
    }
  });
});

describe("formatShortcut", () => {
  it("writes shortcuts the way the PRD does", () => {
    for (const text of [
      "Ctrl+Shift+N",
      "Ctrl+Alt+D",
      "Alt+←",
      "Shift+Del",
      "Alt+Enter",
      "Ctrl+,",
      "Ctrl+Shift+1",
      "F1",
      "?",
      "Space",
      "Backspace",
    ]) {
      expect(formatShortcut(parsed(text))).toBe(text);
    }
    expect(formatShortcut(parsed("alt+shift+ctrl+x"))).toBe("Ctrl+Shift+Alt+X");
  });
});

describe("matches", () => {
  const press = { code: "KeyC", ctrlKey: true, shiftKey: false, altKey: false, metaKey: false };

  it("needs the same key and exactly the same modifiers", () => {
    expect(matches(press, parsed("Ctrl+C"))).toBe(true);
    expect(matches({ ...press, shiftKey: true }, parsed("Ctrl+C"))).toBe(false);
    expect(matches({ ...press, ctrlKey: false }, parsed("Ctrl+C"))).toBe(false);
    expect(matches({ ...press, metaKey: true }, parsed("Ctrl+C"))).toBe(false);
    expect(matches(press, parsed("Ctrl+V"))).toBe(false);
  });
});

describe("keys a control keeps", () => {
  it("leaves a text field its typing and editing keys", () => {
    for (const text of ["A", "Shift+A", "1", "?", "Space", "Backspace", "Del", "Enter", "Tab"]) {
      expect(isFieldKey(parsed(text)), text).toBe(true);
    }
    for (const text of ["F1", "Shift+F5", "Ctrl+A", "Alt+←", "Ctrl+Alt+D"]) {
      expect(isFieldKey(parsed(text)), text).toBe(false);
    }
  });

  it("leaves a button Space and Enter", () => {
    expect(isButtonKey(parsed("Space"))).toBe(true);
    expect(isButtonKey(parsed("Enter"))).toBe(true);
    expect(isButtonKey(parsed("Alt+Enter"))).toBe(false);
    expect(isButtonKey(parsed("Del"))).toBe(false);
  });
});

import { useEffect } from "react";
import { setSetting, useSettings } from "../stores/settings";

/**
 * Ctrl+Alt+D switches the row density (Comfortable 30 px ↔ Compact 24 px). Matched by
 * `KeyboardEvent.code`, so it works on the Georgian layout. A stand-in until the command
 * registry (#24) owns the keymap; move it there then.
 */
export function useDensityShortcut() {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!isToggleDensity(event)) {
        return;
      }
      event.preventDefault();
      const density = useSettings.getState().settings?.density;
      if (density) {
        void setSetting("density", density === "compact" ? "comfortable" : "compact");
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}

export function isToggleDensity(event: KeyboardEvent): boolean {
  if (event.code !== "KeyD" || !event.ctrlKey || !event.altKey || event.shiftKey) {
    return false;
  }
  if (event.metaKey || event.repeat) {
    return false;
  }
  // AltGr arrives as Ctrl+Alt; in a text field it types a letter instead (spike #8).
  return !(isTextField(event.target) && event.getModifierState("AltGraph"));
}

function isTextField(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLTextAreaElement ||
    (target instanceof HTMLInputElement && target.type === "text") ||
    (target instanceof HTMLElement && target.isContentEditable)
  );
}

import type { Appearance, Settings } from "../ipc";
import { useAppearance } from "../stores/appearance";
import { useSettings } from "../stores/settings";

/**
 * Applies the appearance settings to `<html>` and keeps them applied: `data-theme` (System =
 * the window's theme, which follows Windows), `data-density`, `data-backdrop` (Mica) and the
 * accent (`data-accent` for Jerry's colors, the Windows accent as CSS variables). The tokens in
 * design/tokens.css do the rest, so every change is instant. Call once per window, at start.
 */
export function followAppearance(): () => void {
  // WebView2's color scheme follows the window's Tauri theme, which follows the theme setting.
  const prefersDark = window.matchMedia("(prefers-color-scheme: dark)");
  const apply = () =>
    applyAppearance(
      document.documentElement,
      useSettings.getState().settings,
      useAppearance.getState().appearance,
      prefersDark.matches,
    );
  apply();
  const stopSettings = useSettings.subscribe(apply);
  const stopAppearance = useAppearance.subscribe(apply);
  prefersDark.addEventListener("change", apply);
  return () => {
    stopSettings();
    stopAppearance();
    prefersDark.removeEventListener("change", apply);
  };
}

/** Sets the attributes and variables on `root`. Before the settings arrive, only the theme. */
export function applyAppearance(
  root: HTMLElement,
  settings: Settings | undefined,
  appearance: Appearance | undefined,
  systemDark: boolean,
): void {
  const theme = settings?.theme ?? "system";
  root.dataset.theme = theme === "system" ? (systemDark ? "dark" : "light") : theme;
  if (!settings || !appearance) {
    return;
  }
  root.dataset.density = settings.density;
  root.dataset.backdrop = settings.mica && appearance.mica ? "mica" : "solid";

  const windows = settings.accent === "windows" ? appearance.accent : null;
  if (windows) {
    root.style.setProperty("--jerry-accent-base", windows.light);
    root.style.setProperty("--jerry-accent-base-dark", windows.dark);
  } else {
    root.style.removeProperty("--jerry-accent-base");
    root.style.removeProperty("--jerry-accent-base-dark");
  }
  // Blue is the tokens' default; "Match Windows" without a Windows accent falls back to it.
  if (settings.accent === "windows" || settings.accent === "blue") {
    delete root.dataset.accent;
  } else {
    root.dataset.accent = settings.accent;
  }
}

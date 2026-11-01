const prefersDark = window.matchMedia("(prefers-color-scheme: dark)");

/**
 * Sets `data-theme` on `<html>` from the Windows app theme and keeps it in sync.
 * The "App theme" setting replaces this once settings exist (#21, #22).
 */
export function followSystemTheme(): void {
  const apply = () => {
    document.documentElement.dataset.theme = prefersDark.matches ? "dark" : "light";
  };
  apply();
  prefersDark.addEventListener("change", apply);
}

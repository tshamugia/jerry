import type { ReactNode } from "react";

/** A key chip, as the palette and the shortcut table show shortcuts: "Ctrl+Shift+N", "Esc". */
export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="whitespace-nowrap rounded-kbd border border-flyout-border bg-kbd px-1.5 py-px font-sans text-caption text-fg-2">
      {children}
    </kbd>
  );
}

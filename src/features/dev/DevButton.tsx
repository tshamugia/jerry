import type { ReactNode } from "react";

/** A plain button for the dev panel. */
export function DevButton({
  onClick,
  type = "button",
  children,
}: {
  onClick?: () => void;
  type?: "button" | "submit";
  children: ReactNode;
}) {
  return (
    <button
      type={type}
      onClick={onClick}
      className="h-row rounded-ctrl border border-ctrl-border bg-ctrl px-3 text-fg hover:bg-hover"
    >
      {children}
    </button>
  );
}

import type { ButtonHTMLAttributes, ReactNode } from "react";

/** A dialog's row of buttons, at its bottom right under a line. */
export function DialogButtons({ children }: { children: ReactNode }) {
  return <div className="flex justify-end gap-2 border-stroke border-t px-4 py-3">{children}</div>;
}

/** A dialog's text button, Windows style; the one that answers the question is `primary`. */
export function DialogButton({
  primary = false,
  className = "",
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { primary?: boolean }) {
  return (
    <button
      type="button"
      className={`h-toolbar-btn min-w-20 rounded-ctrl border border-ctrl-border bg-ctrl px-3 enabled:hover:bg-hover disabled:text-disabled ${primary ? "font-semibold" : ""} ${className}`}
      {...rest}
    />
  );
}

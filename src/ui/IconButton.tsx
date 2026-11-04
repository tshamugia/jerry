import type { ButtonHTMLAttributes } from "react";

interface Props extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "aria-label"> {
  /** What the button does, with its shortcut; screen readers read it. */
  label: string;
  /** For toggle buttons: pressed buttons get the selected look. */
  pressed?: boolean;
}

/**
 * A borderless icon button: hover fill when enabled, the disabled color when not. Size and
 * radius come from `className`. Works inside a drag region (title bar).
 */
export function IconButton({ label, pressed, className = "", children, ...rest }: Props) {
  return (
    <button
      type="button"
      aria-label={label}
      aria-pressed={pressed}
      className={`no-drag inline-flex shrink-0 items-center justify-center enabled:hover:bg-hover disabled:text-disabled aria-pressed:bg-nav-sel aria-pressed:text-accent ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}

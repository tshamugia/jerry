import type { ButtonHTMLAttributes, ReactNode } from "react";
import { InfoIcon } from "./icons";

/**
 * A card like a toast that stays until it is answered (design: «Update notice»), for the
 * `Toasts` stack: an icon, a bold line, a second line, then what `children` add (a list, a bar,
 * `NoticeActions`). Screen readers hear it as it comes.
 */
export function Notice({
  title,
  detail,
  children,
}: {
  title: string;
  detail?: string;
  children?: ReactNode;
}) {
  return (
    <div
      role="status"
      className="pointer-events-auto flex flex-col gap-2.5 rounded-flyout border border-flyout-border bg-flyout py-3 pr-3 pl-3.5 shadow-flyout"
    >
      <div className="flex items-center gap-3">
        <span className="shrink-0 text-fg-2">
          <InfoIcon />
        </span>
        <div className="flex min-w-0 flex-1 flex-col gap-px">
          <span className="truncate font-semibold">{title}</span>
          {detail && <span className="truncate text-small text-fg-2 tabular-nums">{detail}</span>}
        </div>
      </div>
      {children}
    </div>
  );
}

/** A notice's buttons, at its bottom right; the one that goes ahead comes last. */
export function NoticeActions({ children }: { children: ReactNode }) {
  return <div className="flex justify-end gap-2">{children}</div>;
}

/** A 30 px dialog button in a notice. */
export function NoticeButton({ className = "", ...rest }: ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      className={`h-toolbar-btn min-w-20 rounded-ctrl border border-ctrl-border bg-ctrl px-3 hover:bg-hover ${className}`}
      {...rest}
    />
  );
}

/** The 4 px accent bar; it sweeps while the size isn't known (`percent` null). */
export function NoticeBar({ percent }: { percent: number | null }) {
  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent ?? undefined}
      className="h-1 overflow-hidden rounded-bar bg-track"
    >
      {percent === null ? (
        <div className="h-full w-1/3 animate-loading bg-accent motion-reduce:animate-none" />
      ) : (
        <div className="h-full bg-accent" style={{ width: `${percent}%` }} />
      )}
    </div>
  );
}

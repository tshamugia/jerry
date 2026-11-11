import type { ReactNode } from "react";
import { dismissToast, useToasts } from "../stores/toasts";
import { strings } from "../strings";
import { CloseIcon, InfoIcon } from "./icons";

/**
 * The window's toasts (design: «Toast»): 336 px cards in the bottom right corner, 16 px from the
 * right and 96 px from the bottom, or above the transfer panel when it is taller (its queue is
 * open), the newest lowest. Each has an icon, a bold line, a second line and a dismiss button;
 * screen readers hear them as they come. `children` are cards that stay until they are answered,
 * above the toasts.
 */
export function Toasts({ children }: { children?: ReactNode }) {
  const toasts = useToasts((state) => state.toasts);
  return (
    <div className="pointer-events-none fixed right-4 bottom-toasts z-50 flex w-toast flex-col gap-2">
      {children}
      {toasts.map((toast) => (
        <div
          key={toast.id}
          role="status"
          className="pointer-events-auto flex items-center gap-3 rounded-flyout border border-flyout-border bg-flyout py-3 pr-2 pl-3.5 shadow-flyout"
        >
          <span className="shrink-0 text-fg-2">
            <InfoIcon />
          </span>
          <div className="flex min-w-0 flex-1 flex-col gap-px">
            <span className="truncate font-semibold">{toast.title}</span>
            {toast.detail && (
              <span className="truncate text-small text-fg-2" title={toast.detail}>
                {toast.detail}
              </span>
            )}
          </div>
          <button
            type="button"
            aria-label={strings.session.dismiss}
            onClick={() => dismissToast(toast.id)}
            className="inline-flex size-7 shrink-0 items-center justify-center rounded-ctrl text-fg-2 hover:bg-hover"
          >
            <CloseIcon />
          </button>
        </div>
      ))}
    </div>
  );
}

import { create } from "zustand";

/** A short note in the window's corner: what Jerry did on its own, and why. */
export interface Toast {
  id: number;
  /** One line, bold. */
  title: string;
  /** A second line, such as a path. */
  detail?: string;
}

/** Toasts shown at once; an older one goes when a newer comes. */
export const MAX_TOASTS = 3;

/** How long a toast shows unless dismissed. */
export const TOAST_MS = 10_000;

interface ToastsState {
  /** Oldest first. */
  toasts: readonly Toast[];
}

/** This window's toasts. */
export const useToasts = create<ToastsState>(() => ({ toasts: [] }));

let lastId = 0;

/** Shows a toast for `TOAST_MS`, or until dismissed. Returns its id. */
export function showToast(toast: Omit<Toast, "id">): number {
  const id = ++lastId;
  useToasts.setState((state) => ({
    toasts: [...state.toasts, { ...toast, id }].slice(-MAX_TOASTS),
  }));
  setTimeout(() => dismissToast(id), TOAST_MS);
  return id;
}

export function dismissToast(id: number): void {
  useToasts.setState((state) => {
    const toasts = state.toasts.filter((toast) => toast.id !== id);
    return toasts.length === state.toasts.length ? state : { toasts };
  });
}

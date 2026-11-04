import { type PaneSide, useWindow } from "../../stores/window";

/** Ctrl+F: the keyboard goes to the active pane's filter field, its text selected to type over. */
export function focusFilter(): void {
  const pane = useWindow.getState().activePane;
  const field = document.querySelector<HTMLInputElement>(`input[data-filter="${pane}"]`);
  field?.focus();
  field?.select();
}

/** Gives the keyboard back to `pane`'s file list. */
export function focusList(pane: PaneSide): void {
  document.querySelector<HTMLElement>(`[data-file-list="${pane}"]`)?.focus();
}

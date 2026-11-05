import { create } from "zustand";
import type { JerryError, ListingTarget, ResolvedPath } from "../../ipc";
import type { Restore } from "../../stores/folder";
import { activeTab } from "../../stores/tabs";
import { activatePane, type PaneSide, useWindow } from "../../stores/window";
import { errorText, strings } from "../../strings";

interface AddressEditState {
  /** The pane whose address bar is in text mode, if any. */
  pane: PaneSide | undefined;
}

/** Which address bar takes typing (Ctrl+L, a click on the bar). */
export const useAddressEdit = create<AddressEditState>(() => ({ pane: undefined }));

/** Ctrl+L, or a click on the bar: `pane`'s address bar in text mode, its text selected. */
export function editAddress(pane: PaneSide = useWindow.getState().activePane): void {
  activatePane(pane);
  useAddressEdit.setState({ pane });
}

/** The address bar of `pane` goes back to its segments. */
export function stopEditing(pane: PaneSide): void {
  if (useAddressEdit.getState().pane === pane) {
    useAddressEdit.setState({ pane: undefined });
  }
}

export const canEditAddress = () => activeTab(useWindow.getState().activePane) !== undefined;

/** Where a resolved address leads: its folder, with the file the text named selected. */
export function addressTarget(resolved: ResolvedPath): {
  target: ListingTarget;
  restore: Restore | undefined;
} {
  return {
    target: { kind: "folder", folder: resolved.folder },
    restore: resolved.select === null ? undefined : { focus: resolved.select },
  };
}

/** Why `text` leads nowhere, as the address bar says it. */
export function addressError(error: JerryError, text: string): string {
  return error.code === "NotFound" || error.code === "InvalidName"
    ? strings.address.notFound(text.trim())
    : errorText(error);
}

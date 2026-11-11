import { create } from "zustand";

/** The overlays: modal dialogs over the window. The palette and Go to folder join later. */
export type Overlay = "shortcuts" | "selectByPattern" | "search";

interface OverlayState {
  /** The open overlay; at most one at a time. */
  overlay: Overlay | undefined;
}

/** Which overlay this window shows, if any. */
export const useOverlay = create<OverlayState>(() => ({ overlay: undefined }));

/** Opens `overlay`, closing any other. */
export function openOverlay(overlay: Overlay) {
  useOverlay.setState({ overlay });
}

export function closeOverlay() {
  useOverlay.setState({ overlay: undefined });
}

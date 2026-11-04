import { create } from "zustand";
import { type Appearance, commands, events, subscribe } from "../ipc";

interface AppearanceState {
  /** Whether Mica can show and the Windows accent; `undefined` until the core answers. */
  appearance: Appearance | undefined;
}

/** What the core knows about the Windows look, the same in every window. */
export const useAppearance = create<AppearanceState>(() => ({ appearance: undefined }));

/**
 * Loads the appearance and keeps the store in sync with `appearance:changed` (the Windows accent
 * changed, or Mica stopped working) until the returned function is called. Call once per window,
 * at start.
 */
export function syncAppearance(): () => void {
  let heard = false;
  return subscribe(
    events.appearanceChanged,
    (appearance) => {
      heard = true;
      useAppearance.setState({ appearance });
    },
    () => {
      void commands.appearanceGet().then((appearance) => {
        // An event that arrived first is newer than this answer.
        if (!heard) {
          useAppearance.setState({ appearance });
        }
      });
    },
  );
}

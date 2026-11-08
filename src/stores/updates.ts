import { create } from "zustand";
import { commands, events, subscribe, type UpdateOffer } from "../ipc";

interface UpdatesState {
  /** The newer version and how far its install got; `null` while there is none. */
  offer: UpdateOffer | null;
  /** The user said Later: the notice hides until the next check offers a version again. */
  later: boolean;
}

/** The new version the core found (#46). The core owns the offer; this window only shows it. */
export const useUpdates = create<UpdatesState>(() => ({ offer: null, later: false }));

/** Loads the offer and follows `update:changed` until the returned function is called. */
export function syncUpdates(): () => void {
  let heard = false;
  return subscribe(
    events.updateChanged,
    (offer) => {
      heard = true;
      useUpdates.setState({ offer, later: false });
    },
    () => {
      void commands.updateOffer().then((offer) => {
        // An event that arrived first is newer than this answer.
        if (!heard) {
          useUpdates.setState({ offer });
        }
      });
    },
  );
}

/** Hides the notice for now. */
export function updateLater(): void {
  useUpdates.setState({ later: true });
}

/** The user agreed: the core downloads the version, installs it and restarts Jerry. */
export function installUpdate(): void {
  void commands.updateInstall();
}

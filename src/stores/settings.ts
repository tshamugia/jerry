import { create } from "zustand";
import { commands, events, type Settings, subscribe } from "../ipc";

interface SettingsState {
  /** Every setting, from the core; `undefined` until the first answer arrives. */
  settings: Settings | undefined;
}

/**
 * The settings, the same in every window. The core owns them (`jerry.db`): a change goes to it
 * through `setSetting`, and every window's store, this one's included, updates from
 * `settings:changed`.
 */
export const useSettings = create<SettingsState>(() => ({ settings: undefined }));

/** One setting; `undefined` until the settings arrive. */
export function useSetting<K extends keyof Settings>(key: K): Settings[K] | undefined {
  return useSettings((state) => state.settings?.[key]);
}

/**
 * Loads the settings and keeps the store in sync with `settings:changed` until the returned
 * function is called. Call once per window, at start.
 */
export function syncSettings(): () => void {
  let heard = false;
  return subscribe(
    events.settingsChanged,
    (settings) => {
      heard = true;
      useSettings.setState({ settings });
    },
    () => {
      void commands.settingsGet().then((result) => {
        // An event that arrived first is newer than this answer.
        if (result.status === "ok" && !heard) {
          useSettings.setState({ settings: result.data });
        }
      });
    },
  );
}

/** Changes one setting and saves it. The stores follow from `settings:changed`. */
export function setSetting<K extends keyof Settings>(key: K, value: Settings[K]) {
  return commands.settingsSet(key, value);
}

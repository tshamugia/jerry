import { create } from "zustand";
import {
  commands,
  type Drive,
  type DriveList,
  events,
  type JerryError,
  type NetworkPlace,
  type SidebarPlace,
  subscribe,
} from "../../ipc";
import { setSetting, useSettings } from "../../stores/settings";
import { activeTab, openInNewTab } from "../../stores/tabs";
import { useWindow } from "../../stores/window";
import { focusListSoon } from "../filter/focus";

/** Sections that fold. */
export type Section = "favorites" | "recent" | "drives" | "network";

interface SidebarState {
  /** Desktop, Downloads, Documents, Pictures, OneDrive: what this user has. */
  places: readonly SidebarPlace[];
  /** The drives, numbered by the core: a later list has a higher generation. */
  drives: DriveList;
  /** The network locations the user added. */
  network: readonly NetworkPlace[];
  /** Folded sections, for this window's life. Recent starts folded, one line as the design has it. */
  folded: ReadonlySet<Section>;
  /** The field that adds a network location is open. */
  adding: boolean;
  /** An eject that failed, to say so and offer to try again. */
  ejectFailed: { letter: string; error: JerryError } | undefined;
  /** An eject that would get in the way of transfers, to ask first. */
  ejectWarning: { letter: string; transfers: number } | undefined;
}

const initial: SidebarState = {
  places: [],
  drives: { generation: 0, drives: [] },
  network: [],
  folded: new Set(["recent"]),
  adding: false,
  ejectFailed: undefined,
  ejectWarning: undefined,
};

/**
 * What the sidebar shows. The core owns it: the drives come from its drive monitor
 * (`drives:changed`), the network locations from `jerry.db` (`network-places:changed`), the
 * places from Windows' known folders.
 */
export const useSidebar = create<SidebarState>(() => initial);

/**
 * Loads the sidebar's places, drives and network locations and keeps them current until the
 * returned function is called. Call once per window, at start.
 */
export function syncSidebar(): () => void {
  const stopDrives = subscribe(events.drivesChanged, takeDrives, () => {
    void commands.drivesList().then((result) => {
      if (result.status === "ok") {
        takeDrives(result.data);
      }
    });
  });
  let heard = false;
  const stopNetwork = subscribe(
    events.networkPlacesChanged,
    (network) => {
      heard = true;
      useSidebar.setState({ network });
    },
    () => {
      void commands.networkPlacesList().then((result) => {
        // An event that arrived first is newer than this answer.
        if (result.status === "ok" && !heard) {
          useSidebar.setState({ network: result.data });
        }
      });
    },
  );
  void commands.sidebarPlaces().then((result) => {
    if (result.status === "ok") {
      useSidebar.setState({ places: result.data });
    }
  });
  return () => {
    stopDrives();
    stopNetwork();
  };
}

/** A drive list, unless the store has a newer one already. */
function takeDrives(list: DriveList): void {
  if (list.generation > useSidebar.getState().drives.generation) {
    useSidebar.setState({ drives: list });
  }
}

/** Back to nothing loaded (tests). */
export function resetSidebar(): void {
  useSidebar.setState(initial, true);
}

/** Local drives go under Drives, mapped network drives under Network. */
export function splitDrives(drives: readonly Drive[]): { local: Drive[]; network: Drive[] } {
  return {
    local: drives.filter((drive) => drive.kind !== "network"),
    network: drives.filter((drive) => drive.kind === "network"),
  };
}

/** A drive's root, as a sidebar item opens it. */
export const driveRoot = (letter: string) => `${letter}:\\`;

/** How full a drive is, 0 to 1; undefined before it has been read. */
export function usedShare(drive: Drive): number | undefined {
  const space = drive.space;
  if (!space || space.total <= 0) {
    return undefined;
  }
  return Math.min(1, Math.max(0, (space.total - space.free) / space.total));
}

/** 90 % used or more: the bar and the free space turn red (PRD). */
export const FULL = 0.9;

/**
 * Opens `path` in the active pane: in its tab, or a new one (Ctrl+click, a middle click), and
 * gives the list the focus.
 */
export function openPlace(path: string, newTab = false): void {
  const pane = useWindow.getState().activePane;
  const target = { kind: "path", path } as const;
  if (newTab) {
    openInNewTab(pane, target);
  } else {
    void activeTab(pane)?.open(target);
  }
  focusListSoon(pane);
}

/** Folds or unfolds a section. */
export function toggleSection(section: Section): void {
  const folded = new Set(useSidebar.getState().folded);
  if (!folded.delete(section)) {
    folded.add(section);
  }
  useSidebar.setState({ folded });
}

/** Unfolds a section, if it is folded. */
export function unfoldSection(section: Section): void {
  const folded = new Set(useSidebar.getState().folded);
  if (folded.delete(section)) {
    useSidebar.setState({ folded });
  }
}

/**
 * Ejects a drive, asking first if transfers are on it (`confirmed`: the user said go on). Windows
 * says if something holds it. A failure of Jerry's own is kept to say.
 */
export async function eject(letter: string, confirmed = false): Promise<void> {
  useSidebar.setState({ ejectFailed: undefined, ejectWarning: undefined });
  if (!confirmed) {
    const check = await commands.ejectCheck(letter);
    if (check.status === "ok" && check.data.transfers > 0) {
      useSidebar.setState({ ejectWarning: { letter, transfers: check.data.transfers } });
      return;
    }
  }
  const result = await commands.eject(letter);
  if (result.status === "error" && result.error.code !== "Cancelled") {
    useSidebar.setState({ ejectFailed: { letter, error: result.error } });
  }
}

export function dismissEjectFailed(): void {
  useSidebar.setState({ ejectFailed: undefined });
}

export function dismissEjectWarning(): void {
  useSidebar.setState({ ejectWarning: undefined });
}

export function startAdding(): void {
  useSidebar.setState(({ folded }) => {
    const unfolded = new Set(folded);
    unfolded.delete("network");
    return { adding: true, folded: unfolded };
  });
}

export function stopAdding(): void {
  useSidebar.setState({ adding: false });
}

/** Adds the network location `text`; the error when it isn't one, so the field can say so. */
export async function addNetworkPlace(text: string): Promise<JerryError | undefined> {
  const result = await commands.networkPlaceAdd(text);
  if (result.status === "error") {
    return result.error;
  }
  stopAdding();
  return undefined;
}

export function removeNetworkPlace(path: string): void {
  void commands.networkPlaceRemove(path);
}

export function openRecycleBin(): void {
  void commands.recycleBinOpen();
}

/** Shows or hides the sidebar (Ctrl+B); saved, so it stays so. */
export function toggleSidebar(): void {
  const shown = useSettings.getState().settings?.showSidebar;
  if (shown !== undefined) {
    void setSetting("showSidebar", !shown);
  }
}

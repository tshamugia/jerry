/** The sidebar: places, drives, network locations, Recycle Bin (#34). English only (PRD). */

import type { DriveKind, PlaceName } from "../ipc";

/** A drive's name when its volume has no label, as Explorer names them. */
const unlabeled: Record<DriveKind, string> = {
  fixed: "Local Disk",
  removable: "USB Drive",
  optical: "DVD Drive",
  network: "Network Drive",
  ram: "RAM Disk",
};

export const sidebarStrings = {
  places: {
    desktop: "Desktop",
    downloads: "Downloads",
    documents: "Documents",
    pictures: "Pictures",
    onedrive: "OneDrive",
  } satisfies Record<PlaceName, string>,
  drives: "Drives",
  network: "Network",
  recycleBin: "Recycle Bin",
  /** Explorer's way: `Windows (C:)`, `Local Disk (C:)`. */
  driveName: (letter: string, kind: DriveKind, label: string | null) =>
    `${label ?? unlabeled[kind]} (${letter}:)`,
  /** Under a drive's name. */
  free: (size: string) => `${size} free`,
  /** A drive's tooltip and screen reader text. */
  driveDetails: (name: string, free: string, total: string) => `${name}: ${free} free of ${total}`,
  /** A mapped network drive Windows remembers but isn't connected to. */
  disconnected: (name: string) => `${name} (disconnected)`,
  eject: (name: string) => `Eject ${name}`,
  ejectFailed: "Couldn't eject the drive",
  /** Sections fold and unfold. */
  showSection: (section: string) => `Show ${section}`,
  hideSection: (section: string) => `Hide ${section}`,
  addNetworkPlace: "Add a network location",
  /** The field that adds one. */
  networkField: "Network location",
  networkPlaceholder: "\\\\server\\share",
  networkHint: "Enter adds it, Esc cancels.",
  notAShare: "Type a network location such as \\\\server\\share.",
  /** A network location's right-click menu. */
  placeMenu: "Network location",
  openInNewTab: "Open in new tab",
  removeFromSidebar: "Remove from sidebar",
  retry: "Try again",
  close: "Close",
  closeHint: "Esc",

  // Favorites and recent folders (#35).
  favorites: "Favorites",
  recent: "Recent",
  /** Under Favorites while there are none. */
  noFavorites: (shortcut: string | undefined) =>
    shortcut ? `${shortcut} adds the folder you're in.` : "Add the folder you're in from here.",
  addCurrentFolder: "Add current folder",
  newGroup: "New group",
  /** Right-click menus' names. */
  favoritesMenu: "Favorites",
  favoriteMenu: "Favorite",
  groupMenu: "Favorite group",
  rename: "Rename",
  removeFavorite: "Remove from favorites",
  renameGroup: "Rename group",
  /** Its favorites stay, at the top level. */
  removeGroup: "Remove group, keep its favorites",
  /** The fields that name a favorite or a group. */
  favoriteField: "Favorite name",
  /** `own`: the folder's own name, which an empty name brings back. */
  favoriteHint: (own: string) => `Empty: ${own}. Enter saves, Esc cancels.`,
  groupField: "Group name",
  groupPlaceholder: "Insta, TSGroup, Personal",
  groupHint: "Enter saves, Esc cancels.",
  groupNeedsName: "Type a name for the group.",
  /** A favorite whose folder isn't there, for its tooltip and screen readers. */
  unavailable: (path: string) => `${path} (not available)`,
  /** The explanation a click on such a favorite opens. */
  cantOpen: (name: string) => `Can't open ${name}`,
  missing: (path: string) =>
    `${path} isn't there any more. It may have been deleted, moved or renamed.`,
  notConnected: (path: string) =>
    `The drive or network location of ${path} isn't connected. Connect it, then try again.`,
  favoritesFailed: "Couldn't change the favorites",
  /** Moving favorites and groups with the keyboard, for screen readers. */
  dragInstructions: "To move it, press Space, then Up or Down. Space puts it down, Esc cancels.",
  pickedUp: (name: string) => `Picked up ${name}.`,
  movedOver: (name: string, over: string) => `${name} is over ${over}.`,
  putDown: (name: string) => `Put ${name} down.`,
  dragCancelled: (name: string) => `${name} is back where it was.`,
};

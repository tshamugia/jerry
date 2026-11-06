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
};

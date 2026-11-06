import { useEffect, useMemo, useState } from "react";
import { useSidebar } from "../features/sidebar/store";
import { commands, type Drive, type Space } from "../ipc";
import type { FolderEntries } from "../stores/entries";
import { type Folder, useFolder } from "../stores/folder";

/**
 * What the status bar says of the active pane's folder (#41): its items, its selection and the
 * free space on its drive.
 */

/**
 * How often a network folder's free space is read again while the status bar shows it: as often
 * as the drive monitor reads a local drive's (`PROBE_EVERY`, `jerry_core::drives`).
 */
export const SPACE_EVERY = 10_000;

/** A selection's count and the total size of its files. */
export interface SelectionSize {
  count: number;
  /** Selected files: folders have no size of their own here. */
  files: number;
  /** Selected folders. */
  folders: number;
  /** The files' total size in bytes. */
  bytes: number;
}

/** One pass over the selection: 100 000 selected entries take a millisecond or two. */
export function selectionSize(
  entries: FolderEntries,
  selected: ReadonlySet<number>,
): SelectionSize {
  let files = 0;
  let bytes = 0;
  for (const id of selected) {
    if (!entries.isFolder(id)) {
      files += 1;
      bytes += entries.sizes[id] ?? 0;
    }
  }
  return { count: selected.size, files, folders: selected.size - files, bytes };
}

/**
 * The drive or share a folder is on, as the status bar names it: `D` or the share's name, with its
 * root for the tooltip. A local drive's space comes from the drive list (the sidebar's, kept
 * current by `drives:changed`); a network drive or share has none there and is read on demand.
 */
export type Volume =
  | { kind: "local"; name: string; root: string; space: Space | undefined }
  | { kind: "remote"; name: string; root: string };

/** The volume of the folder at `path` (display text), from the drives Jerry knows. */
export function volumeOf(path: string, drives: readonly Drive[]): Volume | undefined {
  const letter = /^([a-z]):/i.exec(path)?.[1]?.toUpperCase();
  if (letter !== undefined) {
    const drive = drives.find((each) => each.letter === letter);
    const root = `${letter}:\\`;
    // A letter the list doesn't have (yet) is read on demand too.
    return drive && drive.kind !== "network"
      ? { kind: "local", name: letter, root, space: drive.space ?? undefined }
      : { kind: "remote", name: letter, root: drive?.remote ?? root };
  }
  const share = /^\\\\([^\\]+)\\([^\\]+)/.exec(path);
  if (share?.[1] !== undefined && share[2] !== undefined) {
    return { kind: "remote", name: share[2], root: `\\\\${share[1]}\\${share[2]}` };
  }
  return undefined;
}

/**
 * The volume `folder`'s folder is on and its space, once known; nothing for a folder that failed
 * or isn't open. A network volume is read when the folder opens and every `SPACE_EVERY` while it
 * shows, one read at a time; the core waits for a server only so long, so a slow one shows none.
 */
export function useFolderSpace(folder: Folder): { volume: Volume; space: Space } | undefined {
  const path = useFolder(folder, (state) => (state.status === "failed" ? undefined : state.path));
  const listing = useFolder(folder, (state) => state.listing);
  const drives = useSidebar((state) => state.drives.drives);
  const volume = useMemo(
    () => (path === undefined ? undefined : volumeOf(path, drives)),
    [path, drives],
  );
  const read = useReadSpace(volume?.kind === "remote" ? listing : undefined);
  if (volume === undefined) {
    return undefined;
  }
  const space = volume.kind === "local" ? volume.space : read;
  return space && { volume, space };
}

/** The free space of listing `listing`'s volume, read by the core; again every `SPACE_EVERY`. */
function useReadSpace(listing: number | undefined): Space | undefined {
  const [read, setRead] = useState<{ listing: number; space: Space | undefined }>();
  useEffect(() => {
    if (listing === undefined) {
      return;
    }
    let stopped = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const ask = async () => {
      const result = await commands.folderSpace(listing);
      if (stopped) {
        return;
      }
      setRead({ listing, space: result.status === "ok" ? (result.data ?? undefined) : undefined });
      timer = setTimeout(() => void ask(), SPACE_EVERY);
    };
    void ask();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [listing]);
  return read !== undefined && read.listing === listing ? read.space : undefined;
}

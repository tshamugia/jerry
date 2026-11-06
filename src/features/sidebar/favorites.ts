import { create } from "zustand";
import {
  commands,
  events,
  type Favorite,
  type FavoriteGroup,
  type FavoriteList,
  type JerryError,
  type RecentFolder,
  subscribe,
} from "../../ipc";
import { activeFolder } from "../selection/commands";
import { openPlace, unfoldSection, useSidebar } from "./store";

/** What is being named in the sidebar: a favorite, a group, or a group still to be made. */
export type Naming =
  | { kind: "favorite"; id: number }
  | { kind: "group"; id: number }
  | { kind: "newGroup" };

interface FavoritesState {
  groups: readonly FavoriteGroup[];
  /** The top level first, then group by group, as the core orders them. */
  favorites: readonly Favorite[];
  /** The latest folders the user went to. */
  recent: readonly RecentFolder[];
  /** Why a favorite can't be opened (`NotFound`, `DeviceGone`), by id: it shows grey. */
  problems: ReadonlyMap<number, JerryError>;
  /** Folded groups, by id, for this window's life. */
  folded: ReadonlySet<number>;
  naming: Naming | undefined;
  /** The grey favorite whose click explains why it can't be opened. */
  explained: number | undefined;
  /** A favorite or group is being dragged: the items' own keys wait. */
  dragging: boolean;
  /** A change the core refused, to say so. */
  failed: JerryError | undefined;
}

const initial: FavoritesState = {
  groups: [],
  favorites: [],
  recent: [],
  problems: new Map(),
  folded: new Set(),
  naming: undefined,
  explained: undefined,
  dragging: false,
  failed: undefined,
};

/**
 * The sidebar's favorites and recent folders (#35). The core owns them: the favorites come from
 * `jerry.db` (`favorites:changed`), the recent folders from the listings (`recent-folders:changed`).
 * Whether a favorite's folder is there the core checks one favorite at a time.
 */
export const useFavorites = create<FavoritesState>(() => initial);

/** Favorites whose check is still running: a share that doesn't answer isn't asked twice. */
const checking = new Set<number>();

/**
 * Loads the favorites and recent folders and keeps them current until the returned function is
 * called. Favorites are checked when they load or change, when the window comes back to the
 * front, and when a drive comes or goes. Call once per window, at start.
 */
export function syncFavorites(): () => void {
  let heardFavorites = false;
  const stopFavorites = subscribe(
    events.favoritesChanged,
    (list) => {
      heardFavorites = true;
      takeFavorites(list);
    },
    () => {
      void commands.favoritesList().then((result) => {
        // An event that arrived first is newer than this answer.
        if (result.status === "ok" && !heardFavorites) {
          takeFavorites(result.data);
        }
      });
    },
  );
  let heardRecent = false;
  const stopRecent = subscribe(
    events.recentFoldersChanged,
    (recent) => {
      heardRecent = true;
      useFavorites.setState({ recent });
    },
    () => {
      void commands.recentFoldersList().then((result) => {
        if (result.status === "ok" && !heardRecent) {
          useFavorites.setState({ recent: result.data });
        }
      });
    },
  );
  const again = () => checkFavorites();
  window.addEventListener("focus", again);
  let drives = driveKey();
  const stopDrives = useSidebar.subscribe(() => {
    const now = driveKey();
    if (now !== drives) {
      drives = now;
      checkFavorites();
    }
  });
  return () => {
    stopFavorites();
    stopRecent();
    stopDrives();
    window.removeEventListener("focus", again);
  };
}

/** The drives there are, and whether they are connected: a change re-checks the favorites. */
function driveKey(): string {
  return useSidebar
    .getState()
    .drives.drives.map((drive) => `${drive.letter}${drive.connected ? "+" : "-"}`)
    .join();
}

/** A list from the core: what goes with removed favorites and groups goes too; all are checked. */
function takeFavorites({ groups, favorites }: FavoriteList): void {
  useFavorites.setState((state) => {
    const ids = new Set(favorites.map((favorite) => favorite.id));
    const groupIds = new Set(groups.map((group) => group.id));
    const naming = state.naming;
    return {
      groups,
      favorites,
      problems: new Map([...state.problems].filter(([id]) => ids.has(id))),
      folded: new Set([...state.folded].filter((id) => groupIds.has(id))),
      naming:
        (naming?.kind === "favorite" && !ids.has(naming.id)) ||
        (naming?.kind === "group" && !groupIds.has(naming.id))
          ? undefined
          : naming,
      explained:
        state.explained !== undefined && ids.has(state.explained) ? state.explained : undefined,
    };
  });
  checkFavorites();
}

/** Checks whether the favorites' folders are there (all, or `ids`); each answers on its own. */
export function checkFavorites(ids?: readonly number[]): void {
  for (const id of ids ?? useFavorites.getState().favorites.map((favorite) => favorite.id)) {
    void checkFavorite(id);
  }
}

/** Checks the favorite `id`; true if its folder is there. One check per favorite at a time. */
async function checkFavorite(id: number): Promise<boolean | undefined> {
  if (checking.has(id)) {
    return undefined;
  }
  checking.add(id);
  try {
    const result = await commands.favoritesCheck(id);
    if (result.status === "error") {
      return undefined;
    }
    const problem = result.data;
    useFavorites.setState(({ problems, favorites }) => {
      if (!favorites.some((favorite) => favorite.id === id)) {
        return {};
      }
      const next = new Map(problems);
      if (problem) {
        next.set(id, problem);
      } else {
        next.delete(id);
      }
      return { problems: next };
    });
    return problem === null;
  } finally {
    checking.delete(id);
  }
}

/** Back to nothing loaded (tests). */
export function resetFavorites(): void {
  checking.clear();
  useFavorites.setState(initial, true);
}

/**
 * Opens the favorite `id` in the active pane, in its tab or a new one. One that isn't there
 * explains why instead. Either way it is checked again.
 */
export function openFavorite(id: number, newTab: boolean): void {
  const { favorites, problems, dragging } = useFavorites.getState();
  const favorite = favorites.find((each) => each.id === id);
  if (!favorite || dragging) {
    return;
  }
  if (problems.has(id)) {
    useFavorites.setState({ explained: id });
  } else {
    openPlace(favorite.path, newTab);
  }
  void checkFavorite(id);
}

/** Checks the explained favorite again, and opens it if it is there now. */
export async function retryExplained(): Promise<void> {
  const id = useFavorites.getState().explained;
  if (id === undefined) {
    return;
  }
  if (await checkFavorite(id)) {
    useFavorites.setState({ explained: undefined });
    const favorite = useFavorites.getState().favorites.find((each) => each.id === id);
    if (favorite) {
      openPlace(favorite.path);
    }
  }
}

export function dismissExplained(): void {
  useFavorites.setState({ explained: undefined });
}

/** Whether Ctrl+Shift+D can add the folder the active pane shows. */
export function canAddFavorite(): boolean {
  const state = activeFolder()?.store.getState();
  return state?.listing !== undefined && state.status !== "failed";
}

/**
 * Ctrl+Shift+D: adds the folder the active pane shows to the favorites, at the end of the top
 * level, and shows it: the section unfolds, and so does its group if it was a favorite already.
 */
export async function addCurrentFolder(): Promise<void> {
  const listing = activeFolder()?.store.getState().listing;
  if (listing === undefined) {
    return;
  }
  unfoldSection("favorites");
  const result = await commands.favoritesAdd(listing, null);
  if (result.status === "error") {
    useFavorites.setState({ failed: result.error });
    return;
  }
  const group = useFavorites.getState().favorites.find((each) => each.id === result.data)?.group;
  if (group !== undefined && group !== null) {
    unfoldGroup(group);
  }
}

export function toggleGroup(id: number): void {
  const folded = new Set(useFavorites.getState().folded);
  if (!folded.delete(id)) {
    folded.add(id);
  }
  useFavorites.setState({ folded });
}

function unfoldGroup(id: number): void {
  const folded = new Set(useFavorites.getState().folded);
  if (folded.delete(id)) {
    useFavorites.setState({ folded });
  }
}

export function startNaming(naming: Naming): void {
  if (naming.kind === "newGroup") {
    unfoldSection("favorites");
  }
  useFavorites.setState({ naming });
}

export function stopNaming(): void {
  useFavorites.setState({ naming: undefined });
}

/** Names the favorite `id`; empty: its folder's own name again. */
export async function renameFavorite(id: number, name: string): Promise<void> {
  stopNaming();
  await change(commands.favoritesRename(id, name.trim() === "" ? null : name));
}

/** Names the group `id`, or makes a new group; the error when the core refuses the name. */
export async function nameGroup(
  naming: Naming & { kind: "group" | "newGroup" },
  name: string,
): Promise<JerryError | undefined> {
  const result =
    naming.kind === "group"
      ? await commands.favoritesGroupRename(naming.id, name)
      : await commands.favoritesGroupAdd(name);
  if (result.status === "error") {
    return result.error;
  }
  stopNaming();
  return undefined;
}

export function removeFavorite(id: number): void {
  void change(commands.favoritesRemove(id));
}

/** Removes the group `id`; its favorites go to the top level. */
export function removeGroup(id: number): void {
  void change(commands.favoritesGroupRemove(id));
}

export function dismissFailed(): void {
  useFavorites.setState({ failed: undefined });
}

/** A change sent to the core; a refusal is said, and the favorites loaded again. */
async function change(
  sent: Promise<{ status: "ok" } | { status: "error"; error: JerryError }>,
): Promise<void> {
  const result = await sent;
  if (result.status === "error") {
    useFavorites.setState({ failed: result.error });
    const list = await commands.favoritesList();
    if (list.status === "ok") {
      takeFavorites(list.data);
    }
  }
}

/** A line of the Favorites section: a favorite, or a group's caption. */
export type FavoriteRow =
  | { kind: "favorite"; key: string; favorite: Favorite }
  | { kind: "group"; key: string; group: FavoriteGroup };

/**
 * The Favorites section's lines: the top level, then each group's caption and, unless it is
 * folded, its favorites. Those of `lifted` (a group being dragged) are left out: they move with
 * it.
 */
export function favoriteRows(
  groups: readonly FavoriteGroup[],
  favorites: readonly Favorite[],
  folded: ReadonlySet<number>,
  lifted?: number,
): FavoriteRow[] {
  const item = (favorite: Favorite): FavoriteRow => ({
    kind: "favorite",
    key: `f${favorite.id}`,
    favorite,
  });
  return [
    ...favorites.filter((favorite) => favorite.group === null).map(item),
    ...groups.flatMap((group): FavoriteRow[] => [
      { kind: "group", key: `g${group.id}`, group },
      ...(folded.has(group.id) || group.id === lifted
        ? []
        : favorites.filter((favorite) => favorite.group === group.id).map(item)),
    ]),
  ];
}

/** Where a drop puts a dragged line. */
export type Drop =
  | { kind: "favorite"; id: number; group: number | null; index: number }
  | { kind: "group"; id: number; index: number };

/**
 * Where dragging line `from` of `rows` onto line `to` puts it. A favorite joins the group whose
 * caption is the nearest above it (none: the top level), at its place among that group's shown
 * favorites. A group goes among the groups by the captions above it, with its favorites; the top
 * level always comes first.
 */
export function dropOf(rows: readonly FavoriteRow[], from: number, to: number): Drop | undefined {
  const moving = rows[from];
  if (!moving || to < 0 || to >= rows.length) {
    return undefined;
  }
  const moved = [...rows];
  moved.splice(from, 1);
  moved.splice(to, 0, moving);
  const above = moved.slice(0, to);
  if (moving.kind === "group") {
    const index = above.filter((row) => row.kind === "group").length;
    return { kind: "group", id: moving.group.id, index };
  }
  const caption = above.findLastIndex((row) => row.kind === "group");
  const captionRow = moved[caption];
  return {
    kind: "favorite",
    id: moving.favorite.id,
    group: captionRow?.kind === "group" ? captionRow.group.id : null,
    index: to - caption - 1,
  };
}

/** Applies a drop here at once, so the list doesn't jump back, and sends it to the core. */
export function applyDrop(drop: Drop): void {
  useFavorites.setState(({ groups, favorites }) =>
    drop.kind === "favorite"
      ? { favorites: movedFavorite(groups, favorites, drop) }
      : movedGroup(groups, favorites, drop),
  );
  void change(
    drop.kind === "favorite"
      ? commands.favoritesMove(drop.id, drop.group, drop.index)
      : commands.favoritesGroupMove(drop.id, drop.index),
  );
}

/** The favorites in sidebar order: the top level, then group by group. */
function ordered(groups: readonly FavoriteGroup[], favorites: readonly Favorite[]): Favorite[] {
  return [null, ...groups.map((group) => group.id)].flatMap((group) =>
    favorites.filter((favorite) => favorite.group === group),
  );
}

function movedFavorite(
  groups: readonly FavoriteGroup[],
  favorites: readonly Favorite[],
  drop: Drop & { kind: "favorite" },
): Favorite[] {
  const moving = favorites.find((favorite) => favorite.id === drop.id);
  if (!moving) {
    return [...favorites];
  }
  const others = favorites.filter((favorite) => favorite.id !== drop.id);
  const members = others.filter((favorite) => favorite.group === drop.group);
  const at = members[drop.index];
  const placed = { ...moving, group: drop.group };
  const position = at ? others.indexOf(at) : others.length;
  return ordered(groups, [...others.slice(0, position), placed, ...others.slice(position)]);
}

function movedGroup(
  groups: readonly FavoriteGroup[],
  favorites: readonly Favorite[],
  drop: Drop & { kind: "group" },
): Pick<FavoritesState, "groups" | "favorites"> {
  const moving = groups.find((group) => group.id === drop.id);
  if (!moving) {
    return { groups, favorites };
  }
  const next = groups.filter((group) => group.id !== drop.id);
  next.splice(Math.min(drop.index, next.length), 0, moving);
  return { groups: next, favorites: ordered(next, favorites) };
}

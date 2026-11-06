import {
  type CSSProperties,
  type KeyboardEvent,
  lazy,
  type PointerEvent,
  type ReactNode,
  Suspense,
  useEffect,
  useRef,
  useState,
} from "react";
import { commandById, runCommand } from "../../commands/registry";
import type { Favorite, FavoriteGroup, JerryError } from "../../ipc";
import { useCommandLabel, useShortcut } from "../../stores/keymap";
import { errorText, strings } from "../../strings";
import { ContextMenu, MenuItem, MenuSeparator } from "../../ui/ContextMenu";
import { Dialog } from "../../ui/Dialog";
import { IconButton } from "../../ui/IconButton";
import { FolderIcon, PlusIcon, RecentIcon, SectionIcon } from "../../ui/icons";
import {
  addCurrentFolder,
  canAddFavorite,
  dismissExplained,
  dismissFailed,
  type FavoriteRow,
  favoriteRows,
  type Naming,
  nameGroup,
  openFavorite,
  removeFavorite,
  removeGroup,
  renameFavorite,
  retryExplained,
  startNaming,
  stopNaming,
  toggleGroup,
  useFavorites,
} from "./favorites";
import { BUTTON, Caption, folderKey, ITEM_LOOK, NavItem } from "./items";
import { openPlace, toggleSection, useSidebar } from "./store";

const t = strings.sidebar;

/** dnd-kit, for reordering favorites: loaded after the first paint (Tech PRD «წარმადობა»). */
const SortableFavorites = lazy(() => import("./SortableFavorites"));

/** What dragging adds to a line: dnd-kit's node, its moving style, its keys and its press. */
export interface RowDrag {
  ref: (node: HTMLElement | null) => void;
  style: CSSProperties;
  attributes: {
    tabIndex: number;
    "aria-roledescription": string;
    "aria-describedby": string;
    "aria-pressed": boolean | undefined;
  };
  onPointerDown: ((event: PointerEvent) => void) | undefined;
  onKeyDown: ((event: KeyboardEvent) => void) | undefined;
  dragging: boolean;
}

/** Draws line `row` (at `index` in the section); `drag` once dragging is loaded. */
export type RenderRow = (row: FavoriteRow, index: number, drag?: RowDrag) => ReactNode;

/** The props a line's button takes from dragging. */
function dragProps(drag: RowDrag | undefined) {
  return drag
    ? {
        ref: drag.ref,
        style: drag.style,
        ...drag.attributes,
        onPointerDown: drag.onPointerDown,
      }
    : {};
}

/**
 * A line's keys: dragging's first (Space picks it up). While something is dragged the line's own
 * keys wait, so Up and Down move it instead of the focus; F2 names it.
 */
function lineKeys(drag: RowDrag | undefined, naming: Naming) {
  return (event: KeyboardEvent<HTMLButtonElement>) => {
    drag?.onKeyDown?.(event);
    if (event.defaultPrevented) {
      return;
    }
    if (useFavorites.getState().dragging) {
      event.preventDefault();
    } else if (event.key === "F2" && !event.ctrlKey && !event.altKey && !event.shiftKey) {
      event.preventDefault();
      startNaming(naming);
    }
  };
}

/**
 * Favorites (#35), as the design draws them: the caption (its + adds the current folder), the
 * top level, then each group's caption ("Insta", "TSGroup") over its favorites. Groups fold;
 * favorites and groups move by dragging, favorites across groups too. A favorite whose folder
 * isn't there is grey, and a click on it says why.
 */
export function FavoritesSection({ current }: { current: string | undefined }) {
  const groups = useFavorites((state) => state.groups);
  const favorites = useFavorites((state) => state.favorites);
  const foldedGroups = useFavorites((state) => state.folded);
  const problems = useFavorites((state) => state.problems);
  const naming = useFavorites((state) => state.naming);
  const folded = useSidebar((state) => state.folded.has("favorites"));
  const addLabel = useCommandLabel("addFavorite");
  const shortcut = useShortcut("addFavorite");
  const [lifted, setLifted] = useState<number>();
  const rows = favoriteRows(groups, favorites, foldedGroups, lifted);

  const render: RenderRow = (row, index, drag) =>
    row.kind === "favorite" ? (
      <FavoriteItem
        key={row.key}
        favorite={row.favorite}
        selected={current !== undefined && folderKey(row.favorite.path) === current}
        problem={problems.get(row.favorite.id)}
        naming={naming?.kind === "favorite" && naming.id === row.favorite.id}
        drag={drag}
      />
    ) : (
      <GroupCaption
        key={row.key}
        group={row.group}
        first={index === 0}
        folded={foldedGroups.has(row.group.id)}
        naming={naming?.kind === "group" && naming.id === row.group.id}
        drag={drag}
      />
    );

  return (
    <>
      <ContextMenu
        label={t.favoritesMenu}
        menu={
          <>
            <AddCurrentFolder />
            <MenuItem onSelect={() => startNaming({ kind: "newGroup" })}>{t.newGroup}</MenuItem>
          </>
        }
      >
        <Caption
          section="favorites"
          label={t.favorites}
          folded={folded}
          action={
            <IconButton
              label={addLabel}
              onClick={() => {
                const add = commandById("addFavorite");
                if (add) {
                  runCommand(add);
                }
              }}
              className="size-6 rounded-ctrl text-fg-2"
            >
              <PlusIcon />
            </IconButton>
          }
        />
      </ContextMenu>
      {!folded && (
        <>
          <Suspense fallback={rows.map((row, index) => render(row, index))}>
            <SortableFavorites rows={rows} render={render} onLift={setLifted} />
          </Suspense>
          {naming?.kind === "newGroup" && <GroupField naming={naming} initial="" />}
          {rows.length === 0 && naming?.kind !== "newGroup" && (
            <p className="flex h-nav shrink-0 items-center pr-2 pl-6.5 text-caption text-fg-3">
              {t.noFavorites(shortcut)}
            </p>
          )}
        </>
      )}
    </>
  );
}

/** The Favorites menu's first item; whether it can run is asked as the menu opens. */
function AddCurrentFolder() {
  return (
    <MenuItem disabled={!canAddFavorite()} onSelect={() => void addCurrentFolder()}>
      {t.addCurrentFolder}
    </MenuItem>
  );
}

/** A favorite: its name (the user's or its folder's), grey when its folder isn't there. */
function FavoriteItem({
  favorite,
  selected,
  problem,
  naming,
  drag,
}: {
  favorite: Favorite;
  selected: boolean;
  problem: JerryError | undefined;
  naming: boolean;
  drag: RowDrag | undefined;
}) {
  if (naming) {
    return <FavoriteField favorite={favorite} />;
  }
  const own = { kind: "favorite", id: favorite.id } as const;
  return (
    <ContextMenu
      label={t.favoriteMenu}
      menu={
        <>
          <MenuItem onSelect={() => openFavorite(favorite.id, true)}>{t.openInNewTab}</MenuItem>
          <MenuItem onSelect={() => startNaming(own)}>{t.rename}</MenuItem>
          <MenuSeparator />
          <MenuItem onSelect={() => removeFavorite(favorite.id)}>{t.removeFavorite}</MenuItem>
        </>
      }
    >
      <NavItem
        {...dragProps(drag)}
        path={favorite.path}
        icon={<FolderIcon />}
        label={favorite.name}
        title={problem ? t.unavailable(favorite.path) : favorite.path}
        selected={selected}
        indent
        unavailable={problem !== undefined}
        onOpen={(newTab) => openFavorite(favorite.id, newTab)}
        onKeyDown={lineKeys(drag, own)}
      />
    </ContextMenu>
  );
}

/** A group's caption, as the design draws "Insta": a click folds or unfolds its favorites. */
function GroupCaption({
  group,
  first,
  folded,
  naming,
  drag,
}: {
  group: FavoriteGroup;
  first: boolean;
  folded: boolean;
  naming: boolean;
  drag: RowDrag | undefined;
}) {
  if (naming) {
    return <GroupField naming={{ kind: "group", id: group.id }} initial={group.name} />;
  }
  const own = { kind: "group", id: group.id } as const;
  return (
    <ContextMenu
      label={t.groupMenu}
      menu={
        <>
          <MenuItem onSelect={() => startNaming(own)}>{t.renameGroup}</MenuItem>
          <MenuItem onSelect={() => startNaming({ kind: "newGroup" })}>{t.newGroup}</MenuItem>
          <MenuSeparator />
          <MenuItem onSelect={() => removeGroup(group.id)}>{t.removeGroup}</MenuItem>
        </>
      }
    >
      <button
        {...dragProps(drag)}
        type="button"
        aria-expanded={!folded}
        aria-label={folded ? t.showSection(group.name) : t.hideSection(group.name)}
        onClick={() => !useFavorites.getState().dragging && toggleGroup(group.id)}
        onKeyDown={lineKeys(drag, own)}
        className={`group relative flex h-5 w-full shrink-0 items-center rounded-ctrl pr-2 pl-6.5 text-left text-caption text-fg-3 hover:bg-hover ${
          first ? "" : "mt-1"
        }`}
      >
        <span
          className={`absolute left-2 flex ${
            folded ? "" : "opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100"
          }`}
        >
          <SectionIcon folded={folded} />
        </span>
        <span className="truncate">{group.name}</span>
      </button>
    </ContextMenu>
  );
}

/** Renames a favorite in place; empty brings back its folder's own name. */
function FavoriteField({ favorite }: { favorite: Favorite }) {
  const own = folderName(favorite.path);
  return (
    <NameField
      label={t.favoriteField}
      initial={favorite.name}
      placeholder={own}
      hint={t.favoriteHint(own)}
      allowEmpty
      onSave={async (name) => {
        await renameFavorite(favorite.id, name);
        return undefined;
      }}
    />
  );
}

/** Names a new group, or renames one. */
function GroupField({
  naming,
  initial,
}: {
  naming: Naming & { kind: "group" | "newGroup" };
  initial: string;
}) {
  return (
    <NameField
      label={t.groupField}
      initial={initial}
      placeholder={t.groupPlaceholder}
      hint={t.groupHint}
      onSave={async (name) => {
        const error = await nameGroup(naming, name);
        if (!error) {
          return undefined;
        }
        return error.code === "InvalidName" ? t.groupNeedsName : errorText(error);
      }}
    />
  );
}

/**
 * A name typed in place: Enter saves, Esc cancels, and so does leaving it unchanged. Leaving it
 * changed saves it. `onSave` answers with what is wrong, if the name can't be used.
 */
function NameField({
  label,
  initial,
  placeholder,
  hint,
  allowEmpty = false,
  onSave,
}: {
  label: string;
  initial: string;
  placeholder: string;
  hint: string;
  /** An empty name means something (a favorite's own name); else it cancels on leaving. */
  allowEmpty?: boolean;
  onSave: (name: string) => Promise<string | undefined>;
}) {
  const input = useRef<HTMLInputElement>(null);
  const [value, setValue] = useState(initial);
  const [problem, setProblem] = useState<string>();
  const saving = useRef(false);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);

  const save = async () => {
    if (saving.current) {
      return;
    }
    if (value === initial) {
      stopNaming();
      return;
    }
    saving.current = true;
    const error = await onSave(value);
    saving.current = false;
    if (error) {
      setProblem(error);
      input.current?.focus();
    }
  };
  return (
    <div className="flex shrink-0 flex-col gap-1 py-1 pr-1 pl-6">
      <input
        ref={input}
        value={value}
        aria-label={label}
        aria-invalid={problem !== undefined}
        aria-describedby="sidebar-name-hint"
        placeholder={placeholder}
        spellCheck={false}
        onChange={(event) => {
          setValue(event.target.value);
          setProblem(undefined);
        }}
        onKeyDown={(event) => {
          if (event.nativeEvent.isComposing) {
            return;
          }
          if (event.key === "Enter") {
            event.preventDefault();
            void save();
          } else if (event.key === "Escape") {
            event.preventDefault();
            stopNaming();
          }
        }}
        onBlur={() => {
          if (value === initial || (!allowEmpty && value.trim() === "")) {
            stopNaming();
          } else {
            void save();
          }
        }}
        className="h-filter min-w-0 rounded-ctrl border border-ctrl-border bg-ctrl px-2 text-fg text-small placeholder:text-fg-3"
      />
      <span
        id="sidebar-name-hint"
        className={`px-1 text-caption ${problem ? "text-red" : "text-fg-3"}`}
      >
        {problem ?? hint}
      </span>
    </div>
  );
}

/** A folder's own name from its display path: its last part, or a root (`D:`, `\\server\share`). */
export function folderName(path: string): string {
  const trimmed = path.replace(/\\+$/, "");
  const parts = trimmed.split("\\");
  const share = trimmed.startsWith("\\\\") && parts.length <= 4;
  return share ? trimmed : (parts.at(-1) ?? trimmed);
}

/**
 * Recent, as the design draws it: one line with a clock that unfolds the 10 folders opened last.
 * Two of the same name show the folder they are in.
 */
export function RecentSection() {
  const recent = useFavorites((state) => state.recent);
  const folded = useSidebar((state) => state.folded.has("recent"));
  const names = new Map<string, number>();
  for (const folder of recent) {
    names.set(folder.name, (names.get(folder.name) ?? 0) + 1);
  }
  return (
    <>
      <button
        type="button"
        aria-expanded={!folded}
        aria-label={folded ? t.showSection(t.recent) : t.hideSection(t.recent)}
        onClick={() => toggleSection("recent")}
        className={`group ${ITEM_LOOK} mt-1 h-nav shrink-0 px-2`}
      >
        <span className="flex shrink-0 text-fg-2">
          <RecentIcon />
        </span>
        <span className="min-w-0 flex-1 truncate">{t.recent}</span>
        <span
          className={`flex shrink-0 text-fg-3 ${
            folded ? "opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100" : ""
          }`}
        >
          <SectionIcon folded={folded} />
        </span>
      </button>
      {!folded &&
        recent.map((folder) => (
          <NavItem
            key={folder.path}
            path={folder.path}
            icon={<FolderIcon />}
            label={folder.name}
            detail={(names.get(folder.name) ?? 0) > 1 ? parentName(folder.path) : undefined}
            selected={false}
            indent
            onOpen={(newTab) => openPlace(folder.path, newTab)}
          />
        ))}
    </>
  );
}

/** The name of the folder `path` is in. */
function parentName(path: string): string | undefined {
  const parts = path.replace(/\\+$/, "").split("\\");
  return parts.length > 2 ? parts.at(-2) : undefined;
}

/** Says why a grey favorite can't be opened; Try again opens it if it is there now. */
export function FavoriteExplained() {
  const explained = useFavorites((state) =>
    state.favorites.find((favorite) => favorite.id === state.explained),
  );
  const problem = useFavorites((state) =>
    state.explained === undefined ? undefined : state.problems.get(state.explained),
  );
  // The last reason known: a check that finds the folder again doesn't blank the text.
  const [reason, setReason] = useState<JerryError>();
  useEffect(() => {
    if (problem) {
      setReason(problem);
    }
  }, [problem]);
  const [checking, setChecking] = useState(false);
  const shown = problem ?? reason;
  return (
    <Dialog
      open={explained !== undefined}
      onClose={dismissExplained}
      title={explained ? t.cantOpen(explained.name) : ""}
      closeHint={t.closeHint}
    >
      <p className="break-words px-4 pt-3 text-fg">
        {explained &&
          (shown?.code === "DeviceGone"
            ? t.notConnected(explained.path)
            : t.missing(explained.path))}
      </p>
      <div className="flex justify-end gap-2 p-4">
        <button
          type="button"
          disabled={checking}
          onClick={() => {
            setChecking(true);
            void retryExplained().finally(() => setChecking(false));
          }}
          className={BUTTON}
        >
          {t.retry}
        </button>
        <button
          type="button"
          onClick={() => {
            if (explained) {
              removeFavorite(explained.id);
            }
            dismissExplained();
          }}
          className={BUTTON}
        >
          {t.removeFavorite}
        </button>
        <button type="button" onClick={dismissExplained} className={BUTTON}>
          {t.close}
        </button>
      </div>
    </Dialog>
  );
}

/** A change to the favorites the core refused (the database, say). */
export function FavoritesFailed() {
  const failed = useFavorites((state) => state.failed);
  return (
    <Dialog
      open={failed !== undefined}
      onClose={dismissFailed}
      title={t.favoritesFailed}
      closeHint={t.closeHint}
    >
      <p className="break-words px-4 pt-3 text-fg">{failed && errorText(failed)}</p>
      <div className="flex justify-end gap-2 p-4">
        <button type="button" onClick={dismissFailed} className={BUTTON}>
          {t.close}
        </button>
      </div>
    </Dialog>
  );
}

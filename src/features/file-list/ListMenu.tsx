import type { ReactNode } from "react";
import { type Keymap, shortcutsOf } from "../../commands/keymap";
import {
  type Command,
  type CommandId,
  commandById,
  isBuilt,
  isEnabled,
  runCommand,
} from "../../commands/registry";
import { DEFAULT_COLUMNS } from "../../stores/columns";
import { type Folder, useFolder } from "../../stores/folder";
import { useKeymap } from "../../stores/keymap";
import { useSetting } from "../../stores/settings";
import { strings } from "../../strings";
import {
  MenuCheckbox,
  MenuItem,
  type MenuModifiers,
  MenuRadio,
  MenuRadioGroup,
  MenuSeparator,
  MenuSub,
} from "../../ui/ContextMenu";
import { pasteEntries, useClipboardHasFiles } from "../clipboard/clipboard";
import { startDelete } from "../delete/flow";
import { chosenItems } from "../navigation/open";
import { openInTerminal } from "../navigation/shell";
import { shownRows } from "../selection/commands";
import { addFavorite } from "../sidebar/favorites";

const t = strings.fileList.menu;

/** Which menu: on the selected items, or on the folder itself (a right click on empty space). */
export type ListMenuKind = "items" | "folder";

/** A registry command in the menu, with its title and shortcut from there. */
interface Entry {
  id: CommandId;
  /** Instead of the command's title. */
  title?: string;
  /** Instead of the command's own handler: the menu's items, not the keyboard's. */
  run?: (modifiers: MenuModifiers) => void;
  /** Off for now, on top of the command's own `enabled`. */
  off?: boolean;
  /** Opens a field: runs once the menu has closed (`MenuItem`'s `afterClose`). */
  afterClose?: boolean;
  /** Shown, grayed out, before it is built: the place of "Windows options…" (v1.0). */
  reserved?: boolean;
}

type Item = Entry | { sub: "view" | "sortBy" };

/**
 * The file list's context menu (PRD «Context menu»): Jerry's own, short, from the command
 * registry, every command with its shortcut. On items: open, the clipboard, rename and delete,
 * copy path and name, favorites, terminal and Explorer. On the folder: new folder, paste, view,
 * sort, refresh and terminal. Commands not built yet (Quick Look, Duplicate, Properties, New
 * file) stay out until they are; "Windows options…" keeps its place at the end (v1.0). The items
 * act on the active pane, which the right click made the list's.
 */
export function ListMenu({ kind, folder }: { kind: ListMenuKind; folder: Folder }) {
  const keymap = useKeymap((state) => state.keymap);
  const files = useClipboardHasFiles();
  const groups = (kind === "items" ? itemGroups(folder, files) : folderGroups(files))
    .map((group) => group.filter(shown))
    .filter((group) => group.length > 0);
  return groups.map((group, at) => (
    <MenuGroup key={group.map(itemKey).join()} first={at === 0}>
      {group.map((item) =>
        "sub" in item ? (
          item.sub === "view" ? (
            <ViewMenu key="view" keymap={keymap} />
          ) : (
            <SortMenu key="sortBy" folder={folder} />
          )
        ) : (
          <CommandItem key={item.id} entry={item} keymap={keymap} />
        ),
      )}
    </MenuGroup>
  ));
}

function MenuGroup({ first, children }: { first: boolean; children: ReactNode }) {
  return (
    <>
      {!first && <MenuSeparator />}
      {children}
    </>
  );
}

/** The menu on items: the chosen rows of the active pane (the selected ones). */
function itemGroups(folder: Folder, files: boolean | undefined): Item[][] {
  const ids = chosenItems(folder, shownRows(folder));
  const { entries } = folder.store.getState();
  const folders = ids.filter((id) => entries.isFolder(id));
  const [only] = ids;
  // A folder alone takes Paste and Add to favorites; Open in Terminal opens in it too.
  const oneFolder = ids.length === 1 && folders.length === 1 ? only : undefined;
  return [
    [
      { id: "open" },
      ...(folders.length > 0 && folders.length === ids.length
        ? [{ id: "openInNewTab" } as const]
        : []),
      { id: "quickLook" },
    ],
    [
      { id: "cut" },
      { id: "copy" },
      { id: "paste", off: files !== true, run: () => void pasteEntries(oneFolder ?? null) },
    ],
    [
      { id: "rename", afterClose: true },
      { id: "duplicate" },
      // Shift+Delete, as in Explorer: for good, after its question.
      { id: "recycle", title: t.delete, run: ({ shift }) => void startDelete(shift) },
    ],
    [
      { id: "copyPath" },
      { id: "copyName" },
      ...(oneFolder === undefined
        ? []
        : [{ id: "addFavorite", run: () => void addFavorite(oneFolder) } as const]),
    ],
    [{ id: "openInTerminal" }, { id: "showInExplorer" }, { id: "properties" }],
    [{ id: "windowsOptions", reserved: true }],
  ];
}

/** The menu on the folder the list shows (a right click on empty space). */
function folderGroups(files: boolean | undefined): Item[][] {
  return [
    [{ id: "newFolder" }, { id: "newFile" }],
    [{ id: "paste", off: files !== true }],
    [{ sub: "view" }, { sub: "sortBy" }, { id: "refresh" }],
    [{ id: "openInTerminal", run: () => void openInTerminal(true) }],
    [{ id: "windowsOptions", reserved: true }],
  ];
}

/** Whether the item shows: a command that is built, or keeps its place. */
function shown(item: Item): boolean {
  if ("sub" in item) {
    return true;
  }
  const command = commandById(item.id);
  return command !== undefined && (isBuilt(command) || item.reserved === true);
}

function itemKey(item: Item): string {
  return "sub" in item ? item.sub : item.id;
}

function CommandItem({ entry, keymap }: { entry: Entry; keymap: Keymap<Command> }) {
  const command = commandById(entry.id);
  if (!command) {
    return null;
  }
  const enabled = isEnabled(command) && entry.off !== true;
  return (
    <MenuItem
      shortcut={shortcutsOf(keymap, entry.id)[0]}
      disabled={!enabled}
      afterClose={entry.afterClose}
      onSelect={(modifiers) => (entry.run ? entry.run(modifiers) : runCommand(command))}
    >
      {entry.title ?? strings.commands[entry.id]}
    </MenuItem>
  );
}

const VIEWS = ["detailsView", "listView", "gridView"] as const;

/** View: the list's layout (Details only until v0.3), hidden files and row density. */
function ViewMenu({ keymap }: { keymap: Keymap<Command> }) {
  const showHidden = useSetting("showHidden") ?? false;
  const density = useSetting("density") ?? "comfortable";
  const shortcut = (id: CommandId) => shortcutsOf(keymap, id)[0];
  const toggle = (id: CommandId) => {
    const command = commandById(id);
    if (command) runCommand(command);
  };
  return (
    <MenuSub label={t.view}>
      <MenuRadioGroup value="detailsView">
        {VIEWS.map((id) => (
          <MenuRadio key={id} value={id} shortcut={shortcut(id)} disabled={id !== "detailsView"}>
            {strings.commands[id]}
          </MenuRadio>
        ))}
      </MenuRadioGroup>
      <MenuSeparator />
      <MenuCheckbox
        checked={showHidden}
        shortcut={shortcut("toggleHidden")}
        onCheckedChange={() => toggle("toggleHidden")}
      >
        {t.hiddenFiles}
      </MenuCheckbox>
      <MenuCheckbox
        checked={density === "compact"}
        shortcut={shortcut("toggleDensity")}
        onCheckedChange={() => toggle("toggleDensity")}
      >
        {t.compactRows}
      </MenuCheckbox>
    </MenuSub>
  );
}

/** Sort by: a column, then the direction; the folder remembers it, as a header click. */
function SortMenu({ folder }: { folder: Folder }) {
  const sort = useFolder(folder, (state) => state.sort);
  const columns = DEFAULT_COLUMNS.order;
  return (
    <MenuSub label={t.sortBy}>
      <MenuRadioGroup
        value={sort.column}
        onValueChange={(value) => {
          const column = columns.find((each) => each === value);
          if (column) folder.sortTo(column, sort.descending);
        }}
      >
        {columns.map((column) => (
          <MenuRadio key={column} value={column}>
            {strings.fileList.columns[column]}
          </MenuRadio>
        ))}
      </MenuRadioGroup>
      <MenuSeparator />
      <MenuRadioGroup
        value={sort.descending ? "descending" : "ascending"}
        onValueChange={(value) => folder.sortTo(sort.column, value === "descending")}
      >
        <MenuRadio value="ascending">{t.ascending}</MenuRadio>
        <MenuRadio value="descending">{t.descending}</MenuRadio>
      </MenuRadioGroup>
    </MenuSub>
  );
}

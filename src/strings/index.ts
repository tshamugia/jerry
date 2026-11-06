/** Every user-visible string lives here. English only, no i18n library (PRD). */

import type { SortColumn } from "../ipc";
import { clipboardStrings } from "./clipboard";
import { deleteStrings } from "./delete";
import { renameStrings } from "./rename";
import { sessionStrings } from "./session";
import { shellStrings } from "./shell";
import { sidebarStrings } from "./sidebar";

const integer = new Intl.NumberFormat("en-US");
const decimal = new Intl.NumberFormat("en-US", { maximumFractionDigits: 1 });

/** Column titles, as the Details view's header shows them. */
const sortColumns: Record<SortColumn, string> = {
  name: "Name",
  modified: "Date modified",
  type: "Type",
  size: "Size",
  created: "Date created",
  extension: "Extension",
};

export const strings = {
  appName: "Jerry",
  /** The window's zones. */
  frame: {
    sidebar: "Navigation",
    leftPane: "Left pane",
    rightPane: "Right pane",
    activePane: (pane: string) => `${pane}, active`,
    preview: "Preview",
    transfers: "Transfers",
    resizeSidebar: "Resize sidebar",
    resizePanes: "Resize panes",
    resizePreview: "Resize preview",
  },
  /**
   * The title bar: each pane's tabs and the window's caption buttons. The new-tab button is a
   * command, with its title and shortcut from there.
   */
  titleBar: {
    tabs: (pane: string) => `${pane} tabs`,
    closeTab: "Close tab",
    pinnedTab: (name: string) => `${name} (pinned tab)`,
    /** A tab whose first folder hasn't opened. */
    noFolder: "No folder",
    /** A tab's context menu. */
    tabMenu: "Tab",
    duplicate: "Duplicate",
    pin: "Pin tab",
    unpin: "Unpin tab",
    closeOthers: "Close other tabs",
    closeToRight: "Close tabs to the right",
    copyPath: "Copy path",
    openInOtherPane: "Open in other pane",
    minimize: "Minimize",
    maximize: "Maximize",
    restore: "Restore down",
    close: "Close",
  },
  /** A pane's toolbar. Its buttons are commands: their titles and shortcuts come from there. */
  toolbar: {
    filter: "Quick filter",
    filterPlaceholder: "Filter this folder",
    /** While a filter is on, so no file seems lost. */
    showing: (shown: number, total: number) =>
      `Showing ${integer.format(shown)} of ${integer.format(total)}`,
    clearFilter: "Clear filter (Esc)",
    view: "View",
    /** The address bar, until its breadcrumbs come (#33): the folder's path. */
    address: "Address",
    /** Right-click on Back or Forward: the tab's folders that way. */
    backHistory: "Back history",
    forwardHistory: "Forward history",
  },
  /** The address bar (#33): segments, their folder lists, text mode. */
  address: {
    edit: "Type an address",
    subfolders: (name: string) => `Folders in ${name}`,
    collapsed: "Folders before these",
    loading: "Loading…",
    none: "No folders",
    more: (count: number) => `Showing the first ${integer.format(count)} folders`,
    suggestions: "Suggestions",
    notFound: (text: string) => `Jerry can't find “${text}”. Check the spelling and try again.`,
  },
  /** Opening many items at once (Enter on a large selection). */
  openMany: {
    title: "Open many items",
    close: "Esc",
    question: (count: number) =>
      `Open ${integer.format(count)} items? Each folder opens in a new tab, each file in its program.`,
    open: "Open all",
    cancel: "Cancel",
  },
  /** A pane's file list (Details view). */
  fileList: {
    files: (pane: string) => `${pane} files`,
    columns: sortColumns,
    /** The header's context menu. */
    columnsMenu: "Columns",
    moveLeft: "Move left",
    moveRight: "Move right",
    resetColumns: "Reset columns",
    resizeColumn: (column: string) => `Resize ${column}`,
    /** The Type column, as Explorer names types with nothing registered. */
    folderType: "File folder",
    fileType: "File",
    typeOf: (extension: string) => `${extension} File`,
    empty: "This folder is empty. Drop files here",
    noMatch: "No items match the filter",
    loading: "Loading…",
    retry: "Retry",
    /**
     * The list's context menu (right click, Shift+F10, the Menu key): on the selected items, or
     * on the folder itself (empty space). Its commands bring their titles and shortcuts.
     */
    menu: {
      items: "Selected items",
      folder: "This folder",
      /** The Recycle Bin command, as Explorer's menu names it; with Shift, for good. */
      delete: "Delete",
      view: "View",
      hiddenFiles: "Hidden files",
      compactRows: "Compact rows",
      sortBy: "Sort by",
      ascending: "Ascending",
      descending: "Descending",
    },
  },
  /** The status bar: the active pane's folder, and the switches. */
  status: {
    /** The rows the list shows; with the quick filter on, out of how many. */
    items: (shown: number, total: number, filtered: boolean) =>
      filtered
        ? `${integer.format(shown)} of ${integer.format(total)} ${total === 1 ? "item" : "items"}`
        : `${integer.format(shown)} ${shown === 1 ? "item" : "items"}`,
    /** "3 selected · 7.5 MB": the size of the selected files, none with only folders. */
    selected: (count: number, size: string | undefined) =>
      size === undefined
        ? `${integer.format(count)} selected`
        : `${integer.format(count)} selected · ${size}`,
    /** On the selection when it has folders and files: the size is the files'. */
    filesOnly: "Size of the selected files. Folder sizes aren't counted.",
    /** "D: 612 GB free of 1.8 TB"; a share by its name. */
    free: (volume: string, free: string, total: string) => `${volume}: ${free} free of ${total}`,
    comfortable: "Comfortable rows",
    compact: "Compact rows",
    singlePane: "One pane",
    dualPane: "Two panes",
  },
  /** A command's title with its shortcut, for tooltips and screen readers: "Back (Alt+←)". */
  withShortcut: (title: string, shortcut: string | undefined) =>
    shortcut ? `${title} (${shortcut})` : title,
  /**
   * Command titles, by command id (`src/commands/registry.ts`). The palette, menus, the toolbar
   * and the shortcut table (F1) show them.
   */
  commands: {
    addressBar: "Edit address",
    back: "Back",
    forward: "Forward",
    up: "Up",
    goToFolder: "Go to folder",
    refresh: "Refresh",
    open: "Open",
    openInNewTab: "Open in new tab",
    newTab: "New tab",
    closeTab: "Close tab",
    reopenTab: "Reopen closed tab",
    nextTab: "Next tab",
    previousTab: "Previous tab",
    tab1: "Tab 1",
    tab2: "Tab 2",
    tab3: "Tab 3",
    tab4: "Tab 4",
    tab5: "Tab 5",
    tab6: "Tab 6",
    tab7: "Tab 7",
    tab8: "Tab 8",
    tab9: "Tab 9",
    newWindow: "New window",
    dualPane: "Dual pane",
    switchPane: "Switch pane",
    sameFolderInOtherPane: "Same folder in other pane",
    copyToOtherPane: "Copy to other pane",
    moveToOtherPane: "Move to other pane",
    copyTo: "Copy to…",
    moveTo: "Move to…",
    comparePanes: "Compare panes",
    detailsView: "Details view",
    listView: "List view",
    gridView: "Grid view",
    toggleHidden: "Show or hide hidden files",
    toggleSidebar: "Show or hide sidebar",
    togglePreview: "Show or hide preview",
    toggleDensity: "Row density",
    selectAll: "Select all",
    invertSelection: "Invert selection",
    selectByPattern: "Select by pattern…",
    toggleSelection: "Toggle focused item",
    clearSelection: "Clear selection",
    newFolder: "New folder",
    newFile: "New file",
    rename: "Rename",
    duplicate: "Duplicate",
    recycle: "Move to Recycle Bin",
    deletePermanently: "Delete permanently",
    properties: "Properties",
    copyPath: "Copy path",
    copyName: "Copy name",
    openInTerminal: "Open in Terminal",
    showInExplorer: "Show in Explorer",
    windowsOptions: "Windows options…",
    renameSave: "Save name",
    renameCancel: "Cancel rename",
    renameNext: "Rename next file",
    copy: "Copy",
    cut: "Cut",
    paste: "Paste",
    filter: "Filter",
    search: "Search",
    openFileLocation: "Open file location",
    quickLook: "Quick Look",
    undo: "Undo",
    redo: "Redo",
    addFavorite: "Add to favorites",
    palette: "Command palette",
    settings: "Settings",
    shortcuts: "Keyboard shortcuts",
  },
  /** Command groups, in the order the shortcut table shows them. */
  commandGroups: {
    navigation: "Navigation",
    tabs: "Tabs",
    panes: "Windows and panes",
    view: "View",
    selection: "Selection",
    files: "Files",
    rename: "Renaming",
    clipboard: "Clipboard",
    search: "Search",
    preview: "Preview",
    history: "History",
    other: "Other",
  },
  /** The shortcut table (F1). */
  shortcutTable: {
    title: "Keyboard shortcuts",
    close: "Esc",
  },
  /** Deleting (Del, Shift+Del). */
  deleting: deleteStrings,
  /** New folder (Ctrl+Shift+N) and inline rename (F2). */
  rename: renameStrings,
  /** Session restore and its toasts. */
  session: sessionStrings,
  /** Copy, Cut, Paste, Copy path, Copy name. */
  clipboard: clipboardStrings,
  /** Show in Explorer, Open in Terminal. */
  shell: shellStrings,
  /** The sidebar: places, drives, network locations, Recycle Bin. */
  sidebar: sidebarStrings,
  /** Select by pattern (Ctrl+Shift+A). */
  selectByPattern: {
    title: "Select by pattern",
    close: "Esc",
    pattern: "Pattern",
    placeholder: "*.pdf, *_rev*, or a part of the name",
    hint: "* stands for any characters, ? for one.",
    /** Before Enter: how many of the shown items the pattern would select. */
    matches: (count: number, total: number) =>
      `${integer.format(count)} of ${integer.format(total)} match. Enter selects them.`,
  },
  /** The dev panel (dev builds only). */
  dev: {
    title: "Dev checks",
    sampleError: "Sample error",
    panicInJob: "Panic in a job",
    blockMainThread: "Freeze main thread",
    blockPage: "Freeze page",
    stream: "Binary stream",
    noError: "The command didn't fail.",
    frozeMainThread: (ms: number) => `Froze the main thread for ${ms} ms. The log has a warning.`,
    frozePage: (ms: number) => `Froze the page for ${ms} ms. The log has a warning.`,
    streamed: (messages: number, values: string) =>
      `Received ${messages} binary messages: ${values}.`,
    streamIncomplete: (messages: number) => `Only ${messages} of 3 messages arrived.`,
    toggleHidden: "Toggle hidden files",
    toggleExtensions: "Toggle extensions",
    toggleClickToOpen: "Toggle single click",
    nextTheme: "Next theme",
    openWindow: "Open another window",
    toggleDualPane: "Toggle dual pane",
    /** Session restore (#42) until the Settings window (#82). */
    toggleStartFolder: "Start here / previous session",
    toggleFolderViews: "Toggle folder views",
    sessionSettings: (startFolder: string | undefined, folderViews: boolean) =>
      `Start: ${startFolder ?? "previous session"}. Views ${folderViews ? "per folder" : "per tab"}.`,
    settingsLoading: "Settings: loading…",
    settings: (settings: {
      showHidden: boolean;
      showExtensions: boolean;
      clickToOpen: string;
      theme: string;
    }) =>
      `Settings: hidden files ${settings.showHidden ? "shown" : "hidden"}, extensions ${settings.showExtensions ? "shown" : "hidden"}, ` +
      `${settings.clickToOpen} click to open, theme ${settings.theme}.`,
    listingTitle: "Listing",
    listingPath: "Folder path",
    listFolder: "List",
    listSynth: (count: number) => `List ${integer.format(count)}`,
    buildingSynth: (count: number) => `Building a folder with ${integer.format(count)} items…`,
    listing: "Listing…",
    items: (count: number) => `${integer.format(count)} items`,
    listed: (timings: {
      count: number;
      batches: number;
      firstBatchMs: number;
      firstPaintMs: number;
      doneMs: number;
      donePaintMs: number;
      rustMs: number;
    }) =>
      `${integer.format(timings.count)} items in ${timings.batches} ${timings.batches === 1 ? "batch" : "batches"}. ` +
      `First rows painted ${decimal.format(timings.firstPaintMs)} ms, all ${decimal.format(timings.donePaintMs)} ms ` +
      `(first batch ${decimal.format(timings.firstBatchMs)} ms, done ${decimal.format(timings.doneMs)} ms, ` +
      `Rust ${decimal.format(timings.rustMs)} ms).`,
    sidebarIcons: "Sidebar icons",
    places: {
      desktop: "Desktop",
      documents: "Documents",
      downloads: "Downloads",
      pictures: "Pictures",
      onedrive: "OneDrive",
      recyclebin: "Recycle Bin",
      share: "Network share",
    },
    drive: (letter: string) => `${letter}:`,
    scrollTest: "Scroll test",
    scrolling: "Scrolling…",
    scrolled: (result: {
      rows: number;
      ms: number;
      frames: number;
      medianMs: number;
      p95Ms: number;
      maxMs: number;
      janky: number;
      longTasks: number;
      fromMemory: number;
      fromShell: number;
      rendered: number;
      failed: number;
    }) =>
      `Scrolled ${integer.format(result.rows)} rows in ${decimal.format(result.ms / 1000)} s: ` +
      `${integer.format(result.frames)} frames, median ${decimal.format(result.medianMs)} ms, ` +
      `95th percentile ${decimal.format(result.p95Ms)} ms, longest ${decimal.format(result.maxMs)} ms, ` +
      `${result.janky} janky, ${result.longTasks} long tasks. Icons: ${integer.format(result.fromMemory)} from memory, ` +
      `${integer.format(result.fromShell)} from the Shell, ${integer.format(result.rendered)} rendered, ${result.failed} failed.`,
    sorted: (timings: {
      count: number;
      sort: { column: SortColumn; descending: boolean };
      orderMs: number;
      paintMs: number;
    }) =>
      `Sorted ${integer.format(timings.count)} items by ${sortColumns[timings.sort.column].toLowerCase()}, ` +
      `${timings.sort.descending ? "descending" : "ascending"}: painted ${decimal.format(timings.paintMs)} ms ` +
      `(order in JS ${decimal.format(timings.orderMs)} ms).`,
    listFirst: "List a folder first, and wait until it is all in.",
    filterTest: "Filter test",
    filtered: (timings: {
      count: number;
      keys: number;
      workMedianMs: number;
      workMaxMs: number;
      paintMedianMs: number;
      paintMaxMs: number;
    }) =>
      `Filtered ${integer.format(timings.count)} items, ${timings.keys} keys typed: ` +
      `filter and render median ${decimal.format(timings.workMedianMs)} ms, longest ${decimal.format(timings.workMaxMs)} ms; ` +
      `painted median ${decimal.format(timings.paintMedianMs)} ms, longest ${decimal.format(timings.paintMaxMs)} ms.`,
    timeSelectAll: "Time select all",
    selectedAll: (timings: { count: number; selectMs: number; paintMs: number }) =>
      `Selected ${integer.format(timings.count)} items: painted ${decimal.format(timings.paintMs)} ms ` +
      `(selection ${decimal.format(timings.selectMs)} ms).`,
    /** The last change the watcher sent, and how long it took to paint. */
    changed: (change: { added: number; changed: number; removed: number; paintMs: number }) =>
      `Live: ${integer.format(change.added)} new, ${integer.format(change.changed)} changed, ` +
      `${integer.format(change.removed)} removed; painted ${decimal.format(change.paintMs)} ms after it arrived.`,
  },
} as const;

export { errorText, fileName, sizeText } from "./errors";

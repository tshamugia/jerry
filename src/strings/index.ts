/** Every user-visible string lives here. English only, no i18n library (PRD). */

import type { SortColumn } from "../ipc";

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
    minimize: "Minimize",
    maximize: "Maximize",
    restore: "Restore down",
    close: "Close",
  },
  /** A pane's toolbar. Its buttons are commands: their titles and shortcuts come from there. */
  toolbar: {
    filter: "Quick filter",
    filterPlaceholder: "Filter this folder",
    view: "View",
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
    loading: "Loading…",
    retry: "Retry",
  },
  /** The status bar. */
  status: {
    comfortable: "Comfortable rows",
    compact: "Compact rows",
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
    newTab: "New tab",
    closeTab: "Close tab",
    reopenTab: "Reopen closed tab",
    nextTab: "Next tab",
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
    sampleTabs: "Sample tabs",
    sampleTabNames: [
      "PROJECTS",
      "Drawings",
      "Downloads",
      "04_Issued",
      "ფაილები და ნახაზები",
      "პროექტის დოკუმენტაცია 2026",
    ],
    toggleDualPane: "Toggle dual pane",
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
    nothingToSort: "List a folder first, and wait until it is all in.",
    /** The last change the watcher sent, and how long it took to paint. */
    changed: (change: { added: number; changed: number; removed: number; paintMs: number }) =>
      `Live: ${integer.format(change.added)} new, ${integer.format(change.changed)} changed, ` +
      `${integer.format(change.removed)} removed; painted ${decimal.format(change.paintMs)} ms after it arrived.`,
  },
} as const;

export { errorText, fileName, sizeText } from "./errors";

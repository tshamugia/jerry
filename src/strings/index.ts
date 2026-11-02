/** Every user-visible string lives here. English only, no i18n library (PRD). */
export const strings = {
  appName: "Jerry",
  version: (version: string) => `Version ${version}`,

  titleBar: {
    minimize: "Minimize",
    maximize: "Maximize",
    restore: "Restore",
    close: "Close",
    newTab: "New tab (Ctrl+T)",
    closeTab: "Close tab",
    pinnedTab: (name: string) => `${name} (pinned tab)`,
    leftTabs: "Left pane tabs",
    rightTabs: "Right pane tabs",
    newTabName: "New tab",
  },

  // Spike #17 only: the readout that helps check the frame by hand.
  spike: {
    title: "Title bar spike",
    hovered: "Hovered",
    pressed: "Pressed",
    maximized: "Maximized",
    active: "Active window",
    mica: "Mica",
    snapLayouts: "Snap Layouts",
    scale: "Scale",
    size: "Viewport",
    none: "none",
    yes: "yes",
    no: "no",
    hint: "Hover and click the caption buttons; the readout follows the native hit-test.",
  },

  fileList: {
    name: "Name",
    modified: "Date modified",
    type: "Type",
    size: "Size",
    folderType: "File folder",
    fileType: "File",
    /** `ext` comes uppercased (`upperExtension` in features/file-list/format.ts). */
    extType: (ext: string) => `${ext} File`,
    sizeKb: (kb: string) => `${kb} KB`,
    items: (count: number) => `${new Intl.NumberFormat("en-US").format(count)} items`,
    showing: (shown: number, count: number) => {
      const n = new Intl.NumberFormat("en-US");
      return `Showing ${n.format(shown)} of ${n.format(count)}`;
    },
    loading: "Loading…",
    error: (code: string) => `Couldn't open the folder (${code}).`,
  },
  commands: {
    copy: "Copy",
    paste: "Paste",
    filter: "Filter",
    newFolder: "New folder",
  },

  pane: {
    address: "Address",
    filter: "Filter (Ctrl+F)",
    leftPane: "Left pane",
    rightPane: "Right pane",
  },

  // Spike #18 only: the readout for the Georgian rendering and keyboard checks.
  georgian: {
    title: "Georgian spike",
    font: "Font",
    fonts: { jerry: "Jerry (Segoe UI Variable)", segoe: "Segoe UI", sylfaen: "Sylfaen" },
    windowTitle: "Window title",
    lastKeys: "Last shortcuts",
    noKeys: "Press Ctrl+C, Ctrl+V, Ctrl+F or Ctrl+Shift+N.",
    fired: (title: string, shortcut: string, key: string, code: string) =>
      `${title} (${shortcut}) · key "${key}" · code ${code}`,
    hint: "Switch to the Georgian layout and press the shortcuts; each one shows up here.",
  },

  spikeList: {
    open: (count: number) => `Open ${new Intl.NumberFormat("en-US").format(count)}`,
    json: "JSON",
    scroll: "Scroll test",
  },
} as const;

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
    extType: (ext: string) => `${ext.toUpperCase()} File`,
    sizeKb: (kb: string) => `${kb} KB`,
    items: (count: number) => `${new Intl.NumberFormat("en-US").format(count)} items`,
    loading: "Loading…",
    error: (code: string) => `Couldn't open the folder (${code}).`,
  },
  spikeList: {
    open: (count: number) => `Open ${new Intl.NumberFormat("en-US").format(count)}`,
    json: "JSON",
    scroll: "Scroll test",
  },
} as const;

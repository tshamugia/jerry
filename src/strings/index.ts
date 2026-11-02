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
} as const;

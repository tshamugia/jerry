import type { Appearance, Settings } from "../ipc";

/** The core's defaults (`jerry_core::settings`), for tests. */
export const defaultSettings: Settings = {
  theme: "system",
  accent: "windows",
  density: "comfortable",
  fontScale: 100,
  mica: true,
  dateFormat: "DD.MM.YYYY",
  startup: { kind: "previousSession" },
  showHidden: false,
  showExtensions: true,
  foldersFirst: true,
  defaultView: "details",
  dualPane: true,
  rememberFolderViews: true,
  clickToOpen: "double",
  confirmRecycle: false,
  dragAction: "windows",
  springLoadDelayMs: 700,
  parallelTransfers: true,
  notifyOnFinish: true,
  startWithWindows: false,
  keepRunning: true,
  globalHotkey: "Ctrl+Alt+J",
  openInJerry: false,
  checkUpdates: true,
  sidebarWidth: 232,
  previewWidth: 288,
  paneSplit: 0.5,
};

/** Windows 11 with the default blue accent, for tests. */
export const windows11: Appearance = {
  mica: true,
  accent: { light: "#005fb8", dark: "#60cdff" },
};

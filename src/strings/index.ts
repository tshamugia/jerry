/** Every user-visible string lives here. English only, no i18n library (PRD). */
export const strings = {
  appName: "Jerry",
  version: (version: string) => `Version ${version}`,
  /** Spike #16 lab (throwaway, not on main). */
  spikeMenu: {
    hint: "Right-click a file (Shift for more verbs), or press the Menu key or Shift+F10 on one. Ctrl+click selects several.",
    items: "Lab items",
    lastMenu: "Last menu",
    cases: {
      oneFile: "One file",
      severalFiles: "Several files",
      folder: "Folder",
      longPath: "Long path",
      mixedFolders: "Mixed folders",
    },
    summary: (chosen: string | null, invoked: boolean, ackDuringMenu: boolean) =>
      `${chosen === null ? "Nothing chosen" : `Chose "${chosen}"${invoked ? "" : " (Jerry's own verb, not invoked)"}`}. Events during the menu: ${ackDuringMenu ? "yes" : "no"}.`,
    errors: {
      UnknownId: "That item is gone. Reload the lab.",
      MixedFolders:
        "Windows options aren't available for items from different folders. Select items of one folder.",
      Busy: "A Windows menu is already open.",
      NotFound: "Windows can't find that item any more.",
      Io: "The lab folder couldn't be created.",
      Os: "Windows couldn't show its menu. Try again.",
    },
  },
} as const;

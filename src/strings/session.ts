/** Session restore (#42) and the toasts that explain it. English only (PRD). */

export const sessionStrings = {
  /** A restored tab's folder went away; the tab shows the nearest folder left instead. */
  folderGone: (name: string) => `“${name}” no longer exists`,
  nearestOpened: (name: string) => `Opened “${name}”, the nearest folder left`,
  /** A toast's close button. */
  dismiss: "Dismiss",
};

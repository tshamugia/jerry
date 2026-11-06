/** The clipboard (Ctrl+C, Ctrl+X, Ctrl+V, Copy path, Copy name). English only (PRD). */

export const clipboardStrings = {
  /** A clipboard command failed: the title says which, the core's reason follows. */
  failed: {
    copy: "Couldn't copy",
    cut: "Couldn't cut",
    paste: "Couldn't paste",
    copyPath: "Couldn't copy the path",
    copyName: "Couldn't copy the name",
  },
  retry: "Retry",
  close: "Close",
  closeHint: "Esc",
} as const;

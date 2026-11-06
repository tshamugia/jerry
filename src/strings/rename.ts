/** New folder (Ctrl+Shift+N) and inline rename (F2). English only (PRD). */

/** Forbidden characters as the hint lists them. */
const FORBIDDEN_LIST = '\\ / : * ? " < > |';

export const renameStrings = {
  /** A new folder's name until the user types one, as Explorer names them. */
  newFolder: "New folder",
  /** The inline field, for screen readers. */
  field: "Name",
  /** Why the typed text can't be the name, shown under the field as the user types. */
  problems: {
    empty: "Type a name.",
    character: (character: string) =>
      (character.codePointAt(0) ?? 0) < 0x20
        ? `A name can't contain control characters or any of these: ${FORBIDDEN_LIST}`
        : `A name can't contain “${character}”. These characters aren't allowed: ${FORBIDDEN_LIST}`,
    trailing: "A name can't end with a space or a dot.",
    reserved: (device: string) =>
      `“${device}” is a name Windows keeps for a device. Choose another name.`,
    tooLong: "A name can be at most 255 characters long.",
    taken: (name: string) => `There's already an item named “${name}” here.`,
  },
  /** Under the core's reason after a save failed. */
  tryAgain: "Enter tries again, Esc keeps the old name.",
  /** Saving a name with another extension. */
  extension: {
    title: "Change the extension?",
    question: (from: string, to: string) =>
      from === ""
        ? `Add the extension “${to}”? The file might open in another program, or not at all.`
        : to === ""
          ? `Remove the extension “${from}”? The file might not open in its program any more.`
          : `Change the extension from “${from}” to “${to}”? The file might not open in its program any more.`,
    change: "Change",
    cancel: "Keep editing",
    close: "Esc",
  },
  /** Ctrl+Shift+N failed: no field to show the reason in. */
  createFailed: {
    title: "Couldn't create the folder",
    retry: "Retry",
    close: "Close",
    closeHint: "Esc",
  },
} as const;

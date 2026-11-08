/** The notice about a new version of Jerry (#46). English only (PRD). */

const integer = new Intl.NumberFormat("en-US");
const percent = new Intl.NumberFormat("en-US", { style: "percent" });

export const updateStrings = {
  available: (version: string) => `Jerry ${version} is available`,
  /** After the first few changes. */
  moreChanges: (count: number) => `and ${integer.format(count)} more`,
  update: "Update",
  later: "Later",
  retry: "Retry",
  updating: (version: string) => `Updating to Jerry ${version}`,
  /** `done` is 0 to 100, or `null` while the size isn't known. */
  downloading: (done: number | null) =>
    done === null ? "Downloading…" : `Downloading… ${percent.format(done / 100)}`,
  installing: "Installing. Jerry will restart.",
  failed: "Couldn't update Jerry",
  failedHint: "Check your connection and try again.",
};

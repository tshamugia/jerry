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
  /** Update was pressed while transfers run (#52): Jerry closes to update. */
  transfersRunning: (count: number) =>
    count === 1
      ? "A transfer is still running"
      : `${integer.format(count)} transfers are still running`,
  transfersHint: (count: number) =>
    count === 1
      ? "Jerry closes to update, and it stops. When Jerry opens again, you can resume it where it stopped."
      : "Jerry closes to update, and they stop. When Jerry opens again, you can resume them where they stopped.",
  updateAnyway: "Update anyway",
  notNow: "Not now",
};

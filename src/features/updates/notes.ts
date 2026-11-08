/** Changes the notice lists; the rest are counted. */
const LISTED = 4;

const ITEM = /^\s*[-*]\s+(.+)$/;
/** The commit link git-cliff ends an item with: ` ([abc1234](https://…))`. */
const COMMIT_LINK = /\s*\(\[[0-9a-f]{7,}\]\([^)]*\)\)\s*$/;
const LINK = /\[([^\]]+)\]\([^)]*\)/g;
const EMPHASIS = /\*\*|__|`/g;

export interface ShortChanges {
  /** The first few changes, as plain text. */
  changes: string[];
  /** How many more there are. */
  more: number;
}

/**
 * A short list from a release's Markdown notes: its list items as plain text, without links or
 * emphasis. Nothing from the notes is rendered as Markdown or HTML, and no link can be followed.
 */
export function shortChanges(notes: string): ShortChanges {
  const items = notes.split(/\r?\n/).flatMap((line) => {
    const item = ITEM.exec(line)?.[1];
    return item === undefined ? [] : [plain(item)];
  });
  const unique = [...new Set(items.filter((item) => item !== ""))];
  return { changes: unique.slice(0, LISTED), more: Math.max(unique.length - LISTED, 0) };
}

function plain(item: string): string {
  return item.replace(COMMIT_LINK, "").replace(LINK, "$1").replace(EMPHASIS, "").trim();
}

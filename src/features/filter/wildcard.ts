/** Tells whether a file name matches. */
export type NameMatcher = (name: string) => boolean;

/** Regular expression syntax other than `*` and `?`, which are the pattern's own. */
const SYNTAX = /[\\^$.+()[\]{}|]/g;

const matchAll: NameMatcher = () => true;

/**
 * The one wildcard implementation: the quick filter (Ctrl+F) and Select by pattern (Ctrl+Shift+A)
 * both match names with it.
 *
 * - Without `*` or `?`, the pattern is a part of the name: `rev` matches `plan_rev2.dwg`.
 * - With them, it is the whole name: `*` stands for any run of characters (none too), `?` for
 *   exactly one. `*.pdf` matches names that end in `.pdf`, `rev*` names that start with `rev`,
 *   `*_rev*` names with `_rev` anywhere.
 * - Case doesn't count, in any script: Unicode case folding takes Mtavruli to Mkhedruli, so
 *   `ნახაზ` matches `ᲜᲐᲮᲐᲖᲘ.dwg` (spike #18).
 * - The empty pattern matches every name.
 *
 * Compile once, then call the matcher for every name: 100 000 names take a few ms (the PRD's
 * budget is one frame, 16 ms).
 */
export function wildcardMatcher(pattern: string): NameMatcher {
  if (pattern === "") {
    return matchAll;
  }
  const regex = compile(pattern);
  return (name) => regex.test(name);
}

/**
 * `i` + `u` compare by Unicode case folding without making a lowered copy of each name:
 * `toLowerCase()` alone takes ~13 ms for 100 000 Georgian names. `s` lets `?` match any character.
 */
function compile(pattern: string): RegExp {
  if (!/[*?]/.test(pattern)) {
    return new RegExp(literal(pattern), "iu");
  }
  // A star at either end leaves that end open instead of matching `.*` there, which backtracks:
  // `*.pdf` as `\.pdf$` is ten times faster than `^.*\.pdf$`.
  const openStart = pattern.startsWith("*");
  const openEnd = pattern.endsWith("*");
  const body = pattern
    .replace(/^\*+|\*+$/g, "")
    .split(/\*+/)
    .map((piece) => literal(piece).replaceAll("?", "."))
    .join(".*?");
  return new RegExp(`${openStart ? "" : "^"}${body}${openEnd ? "" : "$"}`, "isu");
}

function literal(text: string): string {
  return text.replace(SYNTAX, "\\$&");
}

/**
 * Whether text can name a file or folder on Windows (Tech PRD «გადარქმევა»). One set of rules in
 * two places: here while the user types, and in the core (`crates/jerry-core/src/name.rs`) as the
 * final check before anything is named. Both are tested against the same table
 * (`crates/jerry-core/tests/name_cases.json`), so they can't drift apart.
 *
 * The rules, checked in this order: not empty; none of `\ / : * ? " < > |` and no control
 * characters; not ending in a space or a dot; not a device name (`CON`, `NUL`, `COM1`…), with or
 * without an extension; at most 255 UTF-16 units.
 */

/** The longest name NTFS takes, in UTF-16 units (a string's `length`). */
export const MAX_NAME_UNITS = 255;

/** Characters a name can't have, besides control characters. */
export const FORBIDDEN = '\\/:*?"<>|';

const RESERVED = new Set(["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]);
/** `COM` and `LPT` with a digit or a superscript ¹, ² or ³. */
const NUMBERED = /^(COM|LPT)[0-9¹²³]$/;

/** Why text can't be a name. `taken` is the UI's own: another entry in the folder has it. */
export type NameProblem =
  | { kind: "empty" }
  | { kind: "character"; character: string }
  | { kind: "trailing" }
  | { kind: "reserved"; device: string }
  | { kind: "tooLong" }
  | { kind: "taken"; name: string };

/** The first naming rule `name` breaks, if any. */
export function checkName(name: string): NameProblem | undefined {
  if (name === "") {
    return { kind: "empty" };
  }
  for (const character of name) {
    if (FORBIDDEN.includes(character) || (character.codePointAt(0) ?? 0) < 0x20) {
      return { kind: "character", character };
    }
  }
  if (name.endsWith(" ") || name.endsWith(".")) {
    return { kind: "trailing" };
  }
  const device = deviceName(name);
  if (device !== undefined) {
    return { kind: "reserved", device };
  }
  if (name.length > MAX_NAME_UNITS) {
    return { kind: "tooLong" };
  }
  return undefined;
}

/**
 * The device name `name` stands for, if any: the part before the first dot, without the spaces
 * at its end. Only Latin letters match without regard to case, as Windows matches them.
 */
function deviceName(name: string): string | undefined {
  const stem = (name.split(".")[0] ?? name).replace(/ +$/, "");
  const upper = stem.replace(/[a-z]+/g, (latin) => latin.toUpperCase());
  return RESERVED.has(upper) || NUMBERED.test(upper) ? upper : undefined;
}

/** Georgian letters: `toUpperCase()` would turn Mkhedruli into Mtavruli, which Windows doesn't. */
const GEORGIAN = /[Ⴀ-ჿᲐ-Ჿⴀ-⴯]/;

/**
 * `name` as Windows compares names, as far as the UI can tell: without regard to case, letter by
 * letter. Georgian letters and letters whose capital is longer (ß) stay as they are, so two names
 * the folder can hold side by side never fold alike; the core has the last word.
 */
export function foldName(name: string): string {
  const upper = name.toUpperCase();
  if (upper.length === name.length && !GEORGIAN.test(name)) {
    return upper;
  }
  let folded = "";
  for (const letter of name) {
    const capital = letter.toUpperCase();
    folded += GEORGIAN.test(letter) || capital.length !== letter.length ? letter : capital;
  }
  return folded;
}

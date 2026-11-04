/** Typing starts over after this long without a key, as in Explorer. */
export const TYPE_RESET_MS = 1000;

/**
 * Type-to-select: typing the first letters of a name goes to the next row whose name starts
 * with them, as in Explorer. Keys typed within `TYPE_RESET_MS` of each other add up to one
 * prefix; typing one letter again and again goes through the names that start with it.
 */
export class TypeAhead {
  private text = "";
  private last = Number.NEGATIVE_INFINITY;

  /** Whether a prefix is being typed: Space then adds to it instead of being a shortcut. */
  active(now: number): boolean {
    return this.text !== "" && now - this.last < TYPE_RESET_MS;
  }

  /**
   * Adds a typed character. Returns the prefix to look for, and whether the search starts after
   * the focused row (a new prefix, or one letter again) or at it (a longer prefix).
   */
  type(char: string, now: number): { prefix: string; after: boolean } {
    this.text = this.active(now) ? this.text + char : char;
    this.last = now;
    const chars = [...this.text];
    const first = chars[0] ?? "";
    // One letter, or the same one again: the next name with it. More: the prefix, from here.
    const repeated = chars.every((each) => each === first);
    return repeated ? { prefix: first, after: true } : { prefix: this.text, after: false };
  }

  reset(): void {
    this.text = "";
  }
}

/**
 * The index of the first row, from `start` on and round to the top, whose name starts with
 * `prefix`, ignoring case. Georgian has no case: Mkhedruli stays as it is.
 */
export function findByPrefix(
  rows: Uint32Array,
  names: readonly string[],
  prefix: string,
  start: number,
): number | undefined {
  const wanted = prefix.toLowerCase();
  const count = rows.length;
  for (let step = 0; step < count; step++) {
    const at = (start + step) % count;
    const name = names[rows[at] ?? 0] ?? "";
    if (name.slice(0, wanted.length).toLowerCase() === wanted) {
      return at;
    }
  }
  return undefined;
}

/** `FILE_ATTRIBUTE_DIRECTORY`. */
export const ATTRIBUTE_DIRECTORY = 0x10;

/**
 * A listing's entries as growable columns; an entry's id is its index. Typed arrays keep 100 000
 * entries at a few MB with no object per entry.
 */
export class Columns {
  count = 0;
  names: string[] = [];
  sizes = new Float64Array(0);
  modified = new Float64Array(0);
  attributes = new Uint32Array(0);

  append(
    firstId: number,
    names: readonly string[],
    sizes: ArrayLike<number>,
    modified: ArrayLike<number>,
    attributes: ArrayLike<number>,
  ): void {
    const count = names.length;
    this.reserve(firstId + count);
    for (let i = 0; i < count; i++) {
      this.names[firstId + i] = names[i] ?? "";
    }
    this.sizes.set(sizes, firstId);
    this.modified.set(modified, firstId);
    this.attributes.set(attributes, firstId);
    this.count = Math.max(this.count, firstId + count);
  }

  isDir(id: number): boolean {
    return ((this.attributes[id] ?? 0) & ATTRIBUTE_DIRECTORY) !== 0;
  }

  private reserve(needed: number): void {
    if (needed <= this.sizes.length) {
      return;
    }
    const capacity = Math.max(needed, this.sizes.length * 2, 1024);
    this.sizes = grow(this.sizes, new Float64Array(capacity));
    this.modified = grow(this.modified, new Float64Array(capacity));
    this.attributes = grow(this.attributes, new Uint32Array(capacity));
  }
}

function grow<T extends Float64Array | Uint32Array>(from: T, to: T): T {
  to.set(from);
  return to;
}

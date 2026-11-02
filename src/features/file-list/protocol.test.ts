import { expect, test } from "vitest";
import { decodeMessage, MSG_BATCH, MSG_ORDER } from "./protocol";

/** Builds a batch exactly as `encode_batch` in `src-tauri/src/listing.rs` does. */
function encodeBatch(firstId: number, entries: [string, number, number, number][]): ArrayBuffer {
  const count = entries.length;
  const units = entries.flatMap(([name]) => Array.from(name, (c) => c.charCodeAt(0)));
  const buffer = new ArrayBuffer(16 + count * 24 + units.length * 2);
  new Uint32Array(buffer, 0, 4).set([MSG_BATCH, count, firstId, units.length]);
  new Float64Array(buffer, 16, count).set(entries.map((e) => e[1]));
  new Float64Array(buffer, 16 + count * 8, count).set(entries.map((e) => e[2]));
  new Uint32Array(buffer, 16 + count * 16, count).set(entries.map((e) => e[3]));
  let end = 0;
  new Uint32Array(buffer, 16 + count * 20, count).set(
    entries.map(([name]) => {
      end += name.length;
      return end;
    }),
  );
  new Uint16Array(buffer, 16 + count * 24, units.length).set(units);
  return buffer;
}

test("decodes a binary batch with Georgian names", () => {
  const message = decodeMessage(
    encodeBatch(7, [
      ["file10.txt", 2 ** 40, 1_700_000_000_000, 0x20],
      ["ნახაზი 2.dwg", 0, 1_600_000_000_000, 0x20],
      ["Folder", 0, 1_500_000_000_000, 0x10],
    ]),
  );
  expect(message.kind).toBe("batch");
  if (message.kind !== "batch") return;
  expect(message.firstId).toBe(7);
  expect(message.names).toEqual(["file10.txt", "ნახაზი 2.dwg", "Folder"]);
  expect(Array.from(message.sizes)).toEqual([2 ** 40, 0, 0]);
  expect(Array.from(message.attributes)).toEqual([0x20, 0x20, 0x10]);
});

test("decodes an order message", () => {
  const buffer = new ArrayBuffer(16 + 3 * 4);
  new Uint32Array(buffer, 0, 2).set([MSG_ORDER, 3]);
  new Float64Array(buffer, 8, 1).set([1.5]);
  new Uint32Array(buffer, 16, 3).set([2, 0, 1]);
  const message = decodeMessage(buffer);
  expect(message).toEqual({ kind: "order", ids: new Uint32Array([2, 0, 1]), rustMs: 1.5 });
});

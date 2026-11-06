import { expect, test } from "vitest";
import type { Drive } from "../ipc";
import { FolderEntries } from "../stores/entries";
import { selectionSize, volumeOf } from "./status";

function drive(letter: string, more: Partial<Drive> = {}): Drive {
  return {
    letter,
    kind: "fixed",
    label: null,
    remote: null,
    connected: true,
    ejectable: false,
    space: { total: 100, free: 40 },
    ...more,
  };
}

const drives = [
  drive("C"),
  drive("E", { kind: "removable", space: null }),
  drive("Z", { kind: "network", remote: String.raw`\\INSTA-NAS01\Projects`, space: null }),
];

test("a local drive's space comes from the drive list", () => {
  expect(volumeOf(String.raw`C:\Work\ქართული`, drives)).toEqual({
    kind: "local",
    name: "C",
    root: "C:\\",
    space: { total: 100, free: 40 },
  });
  expect(volumeOf("c:\\", drives)).toMatchObject({ kind: "local", name: "C" });
  // Not read yet, or no media: nothing to show until the list has it.
  expect(volumeOf(String.raw`E:\Photos`, drives)).toEqual({
    kind: "local",
    name: "E",
    root: "E:\\",
    space: undefined,
  });
});

test("network drives, shares and letters the list lacks are read on demand", () => {
  expect(volumeOf(String.raw`Z:\BMW\Drawings`, drives)).toEqual({
    kind: "remote",
    name: "Z",
    root: String.raw`\\INSTA-NAS01\Projects`,
  });
  expect(volumeOf(String.raw`\\INSTA-NAS01\Projects\BMW`, drives)).toEqual({
    kind: "remote",
    name: "Projects",
    root: String.raw`\\INSTA-NAS01\Projects`,
  });
  expect(volumeOf(String.raw`\\server\share`, [])).toMatchObject({ name: "share" });
  expect(volumeOf(String.raw`Q:\Mounted`, drives)).toEqual({
    kind: "remote",
    name: "Q",
    root: "Q:\\",
  });
  expect(volumeOf(String.raw`\\server`, drives)).toBeUndefined();
});

test("a selection's size is its files' sizes; folders count but have none", () => {
  const entries = new FolderEntries();
  entries.appendBatch({
    kind: "batch",
    firstId: 0,
    names: ["Drawings", "plan.dwg", "spec.pdf", "empty.txt"],
    sizes: new Float64Array([0, 1536, 2048, 0]),
    modified: new Float64Array(4),
    created: new Float64Array(4),
    attributes: new Uint32Array([0x10, 0x20, 0x20, 0x20]),
    iconKeys: new Uint32Array(4),
  });

  expect(selectionSize(entries, new Set())).toEqual({ count: 0, files: 0, folders: 0, bytes: 0 });
  expect(selectionSize(entries, new Set([1, 2, 3]))).toEqual({
    count: 3,
    files: 3,
    folders: 0,
    bytes: 3584,
  });
  expect(selectionSize(entries, new Set([0, 1]))).toEqual({
    count: 2,
    files: 1,
    folders: 1,
    bytes: 1536,
  });
});

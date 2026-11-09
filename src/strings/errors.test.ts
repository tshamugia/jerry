import { describe, expect, test } from "vitest";
import type { JerryError } from "../ipc";
import { errorText, fileName, sizeText } from "./errors";

const at = String.raw`C:\Work\ნახაზი.dwg`;

describe("errorText", () => {
  test.each<[JerryError, string]>([
    [{ code: "AccessDenied", path: at }, "You don't have permission to access “ნახაზი.dwg”."],
    [{ code: "AccessDenied", path: null }, "You don't have permission to do this."],
    [
      { code: "NotFound", path: at },
      "“ნახაზი.dwg” couldn't be found. It may have been moved or deleted.",
    ],
    [
      { code: "InUse", params: { app: "AutoCAD" }, path: at },
      "“ნახაზი.dwg” is open in AutoCAD. Close it there and try again.",
    ],
    [
      { code: "InUse", params: { app: null }, path: null },
      "The file is open in another program. Close it there and try again.",
    ],
    [
      {
        code: "NoSpace",
        params: { needed: 5 * 1024 ** 3, available: 1.5 * 1024 ** 3 },
        path: null,
      },
      "There isn't enough space on the drive: 5 GB needed, 1.5 GB free.",
    ],
    [
      { code: "NoSpace", params: { needed: null, available: null }, path: at },
      "There isn't enough space on the drive.",
    ],
    [{ code: "InvalidName", path: String.raw`C:\a?b` }, "“a?b” isn't a valid name here."],
    [
      { code: "DeviceGone", path: String.raw`E:\Photos` },
      "The drive or network location of “Photos” is no longer available.",
    ],
    [
      { code: "NoRecycleBin", path: String.raw`\\server\share\plan.pdf` },
      "“plan.pdf” can't go to the Recycle Bin. Deleting it would be permanent.",
    ],
    [{ code: "Cancelled", path: at }, "Cancelled."],
    [
      { code: "IntoItself", path: String.raw`C:\Work\Plans` },
      "“Plans” can't be copied or moved into itself or a folder inside it. Choose another destination.",
    ],
    [
      { code: "IntoItself", path: null },
      "A folder can't be copied or moved into itself. Choose another destination.",
    ],
    [
      { code: "Os", params: { os: 87 }, path: at },
      "Windows couldn't complete this for “ნახაზი.dwg” (error 87).",
    ],
    [
      { code: "Os", params: { os: 0x8027_0064 }, path: null },
      "Windows couldn't complete this (error 0x80270064).",
    ],
    [{ code: "Internal", path: null }, "Something went wrong in Jerry. Details are in the log."],
  ])("%o", (error, text) => {
    expect(errorText(error)).toBe(text);
  });
});

test("fileName takes the last part of a Windows path", () => {
  expect(fileName(String.raw`C:\Work\ნახაზი.dwg`)).toBe("ნახაზი.dwg");
  expect(fileName(String.raw`C:\Work\Folder\\`)).toBe("Folder");
  expect(fileName(String.raw`C:\\`)).toBe("C:");
  expect(fileName("plain")).toBe("plain");
});

test("sizeText uses 1024-based units", () => {
  expect(sizeText(0)).toBe("0 bytes");
  expect(sizeText(1023)).toBe("1,023 bytes");
  expect(sizeText(1536)).toBe("1.5 KB");
  expect(sizeText(12.34 * 1024 ** 2)).toBe("12.3 MB");
  expect(sizeText(2 * 1024 ** 4)).toBe("2 TB");
});

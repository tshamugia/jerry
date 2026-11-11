import { expect, test } from "vitest";
import { dateText, extensionOf, locationText, nameText, sizeCell, typeName } from "./format";

test("dates are DD.MM.YYYY HH:mm in local time, or as the date format setting says", () => {
  const ms = new Date(2026, 0, 31, 9, 5).getTime();
  expect(dateText(ms, "DD.MM.YYYY")).toBe("31.01.2026 09:05");
  expect(dateText(ms, "YYYY-MM-DD")).toBe("2026-01-31 09:05");
  expect(dateText(new Date(2026, 10, 5, 23, 59).getTime(), "DD.MM.YYYY")).toBe("05.11.2026 23:59");
});

test("a time that was never recorded shows nothing", () => {
  // A zero FILETIME: 1601-01-01.
  expect(dateText(-11_644_473_600_000, "DD.MM.YYYY")).toBe("");
});

test("sizes show for files only", () => {
  expect(sizeCell(2048, false)).toBe("2 KB");
  expect(sizeCell(5_033_164, false)).toBe("4.8 MB");
  expect(sizeCell(0, true)).toBe("");
});

test("types are named as Explorer names unregistered ones, Georgian left as it is", () => {
  expect(typeName("ELV-001.dwg", false)).toBe("DWG File");
  expect(typeName("Report.Final.pdf", false)).toBe("PDF File");
  expect(typeName("README", false)).toBe("File");
  expect(typeName("trailing.", false)).toBe("File");
  expect(typeName(".gitignore", false)).toBe("GITIGNORE File");
  expect(typeName("Drawings.v2", true)).toBe("File folder");
  // Mkhedruli stays Mkhedruli; toUpperCase() would make it Mtavruli (spike #18).
  expect(typeName("გეგმა.ნახ", false)).toBe("ნახ File");
  expect(typeName("ნახაზი.dwgნ", false)).toBe("DWGნ File");
});

test("the extension column shows what follows the last dot, as written", () => {
  expect(extensionOf("Plan.Rev B.DWG", false)).toBe("DWG");
  expect(extensionOf("README", false)).toBe("");
  expect(extensionOf("v1.2", true)).toBe("");
});

test("names lose their extension only when extensions are hidden", () => {
  expect(nameText("plan.dwg", false, true)).toBe("plan.dwg");
  expect(nameText("plan.rev.dwg", false, false)).toBe("plan.rev");
  expect(nameText(".gitignore", false, false)).toBe(".gitignore");
  expect(nameText("v1.2", true, false)).toBe("v1.2");
});

test("a result's Location runs from the folder searched down", () => {
  const root = String.raw`D:\Work\Projects`;
  expect(locationText(String.raw`D:\Work\Projects\BMW\Drawings`, root)).toBe(
    String.raw`Projects\BMW\Drawings`,
  );
  expect(locationText(root, root)).toBe("Projects");
  // A whole drive: the drive starts it.
  expect(locationText(String.raw`C:\Users\TS`, "C:\\")).toBe(String.raw`C:\Users\TS`);
  expect(locationText("C:\\", "C:\\")).toBe("C:\\");
  // Not under it (a name that only starts the same): as it is.
  expect(locationText(String.raw`D:\Work\Projects 2\a`, root)).toBe(
    String.raw`D:\Work\Projects 2\a`,
  );
});

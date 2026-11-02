import { describe, expect, it } from "vitest";
import { Columns } from "./columns";
import { filtered } from "./FileList";
import { formatType, upperExtension } from "./format";

describe("Georgian names (spike #18)", () => {
  it("uppercases Latin extensions but not Georgian ones", () => {
    // What the naive version would show: Unicode 11 maps Mkhedruli to Mtavruli.
    expect("ფაილი".toUpperCase()).toBe("ᲤᲐᲘᲚᲘ");
    expect(upperExtension("pdf")).toBe("PDF");
    expect(upperExtension("ფაილი")).toBe("ფაილი");
    expect(formatType("ანგარიში.ფაილი", false)).toBe("ფაილი File");
    expect(formatType("ნახაზი 10.dwg", false)).toBe("DWG File");
  });

  it("filters Georgian names case-insensitively, Mtavruli included", () => {
    const columns = new Columns();
    const names = ["ნახაზი 1.dwg", "ᲜᲐᲮᲐᲖᲘ ᲡᲐᲑᲝᲚᲝᲝ.dwg", "ხელშეკრულება.pdf", "Project ბათუმი.dwg"];
    const zeros = names.map(() => 0);
    columns.append(0, names, zeros, zeros, zeros);
    const show = (filter: string) =>
      Array.from(filtered(columns, null, names.length, filter) ?? [], (id) => names[id]);
    expect(show("ნახაზ")).toEqual(["ნახაზი 1.dwg", "ᲜᲐᲮᲐᲖᲘ ᲡᲐᲑᲝᲚᲝᲝ.dwg"]);
    expect(show("ბათუმი")).toEqual(["Project ბათუმი.dwg"]);
    expect(show("PROJECT")).toEqual(["Project ბათუმი.dwg"]);
    expect(filtered(columns, null, names.length, "")).toBeNull();
  });
});

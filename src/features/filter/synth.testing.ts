import { FolderEntries } from "../../stores/entries";

/**
 * Tests and benchmarks only: `count` file names in the mix of the core's synthetic folders
 * (`crates/jerry-core/src/synth.rs`): Latin and Georgian, upper and lower case, some in Mtavruli.
 */
export function synthNames(count: number): string[] {
  const names: string[] = [];
  for (let i = 1; i <= count; i++) {
    names.push(synthName(i));
  }
  return names;
}

function synthName(i: number): string {
  switch (i % 20) {
    case 0:
      return `IMG_${String(i).padStart(4, "0")}.jpg`;
    case 1:
      return `DSC${String(i).padStart(5, "0")}.JPG`;
    case 2:
      return `Report ${i}.docx`;
    case 3:
      return `report ${i} final.pdf`;
    case 4:
      return `file${i}.txt`;
    case 5:
      return `ნახაზი ${i}.dwg`;
    case 6:
      return `სართული ${(i % 20) + 1} გეგმა ${i}.dwg`;
    case 7:
      return `ხელშეკრულება №${i}.pdf`;
    case 8:
      return `ინვოისი ${i}.xlsx`;
    case 9:
      return `ᲤᲝᲢᲝ ${i}.png`;
    case 10:
      return `Invoice-2025-${String((i % 12) + 1).padStart(2, "0")}-${i}.pdf`;
    case 11:
      return `Project_v${Math.floor(i / 10_000)}.${Math.floor(i / 100) % 100}.${i % 100}.zip`;
    case 12:
      return `Readme${i}.MD`;
    case 13:
      return `data_${i}.csv`;
    case 14:
      return `Scan ${i}.tif`;
    case 15:
      return `ᲜᲐᲮᲐᲖᲘ ${i} rev${i % 4}.dwg`;
    case 16:
      return `ELV-${i} LAN Layout_rev${i % 7}.dwg`;
    case 17:
      return `ანგარიში ${i} (ასლი).docx`;
    case 18:
      return `Plan ${i}A.dwg`;
    default:
      return `video-${i}.mp4`;
  }
}

/** A listing's entries with these names, ids in arrival order. */
export function synthEntries(names: readonly string[]): FolderEntries {
  const entries = new FolderEntries();
  entries.names = [...names];
  entries.count = names.length;
  return entries;
}

/** Patterns as people type them: a part of a name, Georgian, wildcards. */
export const SYNTH_PATTERNS = ["report", "ნახაზ", "*.pdf", "rev*", "*_rev*", "ᲡᲐᲠᲗ*.dwg", "zzz"];

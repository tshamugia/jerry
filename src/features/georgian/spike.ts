import { filtered } from "../file-list/FileList";
import { formatType } from "../file-list/format";
import { afterNextPaint } from "../file-list/measure";
import { useListing } from "../file-list/store";
import { type Fired, type FontChoice, useGeorgian } from "./store";

/** Where a text's ink sits in its box (px from the box's top); clipped if it leaves the box. */
export interface InkReport {
  zone: string;
  text: string;
  box: number;
  top: number;
  bottom: number;
  clipped: boolean;
  /** The text is cut with an ellipsis. */
  truncated: boolean;
}

/** Spike #18 harness, driven over CDP (`Runtime.evaluate`) by `scripts/spike-18-check.mjs`. */
export interface GeorgianSpikeApi {
  open: () => Promise<number>;
  /** Names in display order, quick filter applied. */
  names: () => string[];
  fired: () => Fired[];
  clear: () => void;
  setFont: (font: FontChoice) => Promise<void>;
  windowTitle: () => string | null;
  ink: () => InkReport[];
  casing: () => Record<string, string>;
  rowHeights: () => number[];
}

declare global {
  interface Window {
    jerryGeorgian?: GeorgianSpikeApi;
  }
}

const TEXT = "[data-name], [data-tab-label], [data-segment], [data-app-name]";

export function installGeorgianSpike(): void {
  window.jerryGeorgian = {
    open: async () => (await useListing.getState().open("georgian")).count,
    names: () => {
      const { columns, order, count, filter } = useListing.getState();
      const shown = filtered(columns, order, count, filter);
      return Array.from(shown ?? columns.names.keys(), (id) => columns.names[id] ?? "");
    },
    fired: () => useGeorgian.getState().fired,
    clear: () => useGeorgian.getState().clear(),
    setFont: async (font) => {
      useGeorgian.getState().setFont(font);
      await document.fonts.ready;
      await afterNextPaint();
    },
    windowTitle: () => useGeorgian.getState().windowTitle,
    ink: () => Array.from(document.querySelectorAll<HTMLElement>(TEXT), inkOf),
    casing: () => ({
      upper: "ფაილი".toUpperCase(),
      upperKa: "ფაილი".toLocaleUpperCase("ka"),
      lowerMtavruli: "ᲜᲐᲮᲐᲖᲘ".toLowerCase(),
      typeColumn: formatType("ანგარიში.ფაილი", false),
    }),
    rowHeights: () => [
      ...new Set(
        Array.from(
          document.querySelectorAll<HTMLElement>('[data-zone="list"] [data-name]'),
          (name) => name.parentElement?.getBoundingClientRect().height ?? 0,
        ),
      ),
    ],
  };
}

let context: CanvasRenderingContext2D | null = null;

/**
 * Measures the text's ink with the element's own font (fallback fonts included) and places it
 * on the line: the line box is the primary font's ascent + descent plus half-leading each side.
 */
function inkOf(element: HTMLElement): InkReport {
  context ??= document.createElement("canvas").getContext("2d");
  const style = getComputedStyle(element);
  const text = element.textContent ?? "";
  const box = element.getBoundingClientRect().height;
  const zone = element.dataset.tabLabel !== undefined ? "tab" : zoneOf(element);
  const truncated = element.scrollWidth > element.clientWidth;
  if (!context) return { zone, text, box, top: 0, bottom: 0, clipped: false, truncated };
  context.font = `${style.fontWeight} ${style.fontSize} ${style.fontFamily}`;
  const m = context.measureText(text);
  const lineHeight = Number.parseFloat(style.lineHeight);
  const halfLeading = (lineHeight - m.fontBoundingBoxAscent - m.fontBoundingBoxDescent) / 2;
  const baseline = halfLeading + m.fontBoundingBoxAscent;
  const top = baseline - m.actualBoundingBoxAscent;
  const bottom = baseline + m.actualBoundingBoxDescent;
  const clipped = top < -0.5 || bottom > box + 0.5;
  return { zone, text, box, top, bottom, clipped, truncated };
}

function zoneOf(element: HTMLElement): string {
  return element.closest<HTMLElement>("[data-zone]")?.dataset.zone ?? "other";
}

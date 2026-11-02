import { create } from "zustand";

/** Spike #18: the font stacks compared on screen. `jerry` is the token's own stack. */
export type FontChoice = "jerry" | "segoe" | "sylfaen";

export const FONT_STACKS: Record<FontChoice, string | null> = {
  jerry: null,
  segoe: '"Segoe UI", sans-serif',
  // A worse fallback for comparison: Georgian from Sylfaen instead of Segoe UI.
  sylfaen: '"Segoe UI Variable Text", Sylfaen, sans-serif',
};

/** A shortcut that ran, with the event that ran it. */
export interface Fired {
  /** Running number, for list keys. */
  seq: number;
  id: string;
  title: string;
  shortcut: string;
  key: string;
  code: string;
}

interface GeorgianState {
  font: FontChoice;
  /** The native window title as Windows read it back. */
  windowTitle: string | null;
  fired: Fired[];
  setFont: (font: FontChoice) => void;
  setWindowTitle: (title: string | null) => void;
  log: (fired: Omit<Fired, "seq">) => void;
  clear: () => void;
}

export const useGeorgian = create<GeorgianState>()((set) => ({
  font: "jerry",
  windowTitle: null,
  fired: [],
  setFont: (font) => {
    const stack = FONT_STACKS[font];
    const root = document.documentElement.style;
    if (stack) root.setProperty("--jerry-font", stack);
    else root.removeProperty("--jerry-font");
    set({ font });
  },
  setWindowTitle: (windowTitle) => set({ windowTitle }),
  log: (fired) =>
    set((state) => ({ fired: [...state.fired, { ...fired, seq: state.fired.length }] })),
  clear: () => set({ fired: [] }),
}));

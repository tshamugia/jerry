import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterAll, afterEach, beforeAll, describe, expect, test } from "vitest";
import { useCommandKeys } from "../../commands/dispatch";
import {
  batchMessage,
  diffMessage,
  doneMessage,
  type FakeListings,
  fakeListings,
  orderMessage,
  type TestEntry,
} from "../../ipc/listing.testing";
import { matchingRows, visibleRows } from "../../stores/entries";
import { Folder } from "../../stores/folder";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
import { useWindow } from "../../stores/window";
import { FileList } from "../file-list/FileList";
import { FilterField } from "./FilterField";
import { SYNTH_PATTERNS, synthEntries, synthNames } from "./synth.testing";
import { wildcardMatcher } from "./wildcard";

// jsdom lays nothing out: give the list a viewport, so the virtualizer renders rows.
const clientHeight = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientHeight");
beforeAll(() => {
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get: () => 600,
  });
});
afterAll(() => {
  if (clientHeight) Object.defineProperty(HTMLElement.prototype, "clientHeight", clientHeight);
});
afterEach(() => {
  useSettings.setState({ settings: undefined });
  useWindow.setState({ activePane: "left" });
});

const WORK: TestEntry[] = [
  { name: "plan_rev2.dwg" },
  { name: "Plot", attributes: 0x10 },
  { name: "ᲜᲐᲮᲐᲖᲘ 1.dwg" },
  { name: "ნახაზი 2.pdf" },
  { name: "report.PDF" },
  { name: "desktop.ini", attributes: 0x2 | 0x4 },
];
const work = { kind: "path", path: String.raw`C:\Work` } as const;

/** A folder with `WORK` open and all in, in this order. */
async function opened(): Promise<{ core: FakeListings; folder: Folder }> {
  useSettings.setState({ settings: defaultSettings });
  const core = fakeListings({
    messages: () => [
      batchMessage(0, WORK),
      orderMessage([1, 0, 2, 3, 4, 5]),
      doneMessage(WORK.length, 1, 1),
    ],
  });
  const folder = new Folder(7);
  await folder.open(work);
  await waitFor(() => expect(folder.store.getState().status).toBe("ready"));
  return { core, folder };
}

const shownNames = (folder: Folder, showHidden = false) => {
  const { entries } = folder.store.getState();
  return [...folder.shown(showHidden).rows].map((id) => entries.names[id]);
};

describe("the folder's filter", () => {
  test("narrows the shown rows; the total counts them without it", async () => {
    const { folder } = await opened();
    expect(shownNames(folder)).toEqual([
      "Plot",
      "plan_rev2.dwg",
      "ᲜᲐᲮᲐᲖᲘ 1.dwg",
      "ნახაზი 2.pdf",
      "report.PDF",
    ]);

    folder.setFilter("*.pdf");
    expect(shownNames(folder)).toEqual(["ნახაზი 2.pdf", "report.PDF"]);
    expect(folder.shown(false).total).toBe(5);

    folder.setFilter("ნახაზ");
    expect(shownNames(folder)).toEqual(["ᲜᲐᲮᲐᲖᲘ 1.dwg", "ნახაზი 2.pdf"]);

    // Hidden entries count only while they show.
    folder.setFilter("i");
    expect(shownNames(folder)).toEqual([]);
    expect(shownNames(folder, true)).toEqual(["desktop.ini"]);
    expect(folder.shown(true).total).toBe(6);

    folder.setFilter("");
    expect(folder.shown(false).rows).toHaveLength(5);
  });

  test("gives the same rows until the folder, the filter or showHidden change", async () => {
    const { folder } = await opened();
    folder.setFilter("plan");
    const first = folder.shown(false);
    expect(folder.shown(false)).toBe(first);

    folder.setFilter("plan_");
    const second = folder.shown(false);
    expect(second).not.toBe(first);
    expect(folder.shown(true)).not.toBe(second);
  });

  test("entries it hides leave the selection and the focus", async () => {
    const { folder } = await opened();
    folder.select(0);
    const selected = folder.store.getState().selected;

    folder.setFilter("plan");
    expect(folder.store.getState().selected).toBe(selected);
    expect(folder.store.getState().focused).toBe(0);

    folder.setFilter("*.pdf");
    expect(folder.store.getState().selected.size).toBe(0);
    expect(folder.store.getState().focused).toBeUndefined();

    // Clearing it doesn't bring them back.
    folder.setFilter("");
    expect(folder.store.getState().selected.size).toBe(0);
  });

  test("watcher changes pass through it", async () => {
    const { core, folder } = await opened();
    folder.setFilter("*.dwg");
    folder.select(0);

    // A new drawing, a new text file, and the selected drawing renamed in Explorer.
    core.send(
      diffMessage(
        [0, 6, 7],
        [],
        [{ name: "plan_rev2.txt" }, { name: "new.dwg" }, { name: "a.txt" }],
      ),
    );
    expect(shownNames(folder)).toEqual(["ᲜᲐᲮᲐᲖᲘ 1.dwg"]);
    expect(folder.store.getState().selected.size).toBe(0);
    expect(folder.store.getState().focused).toBeUndefined();

    // The core's order that follows has the new ones.
    core.send(orderMessage([1, 7, 6, 0, 2, 3, 4, 5]));
    expect(shownNames(folder)).toEqual(["new.dwg", "ᲜᲐᲮᲐᲖᲘ 1.dwg"]);
    expect(folder.shown(false).total).toBe(7);
  });

  test("another folder starts without it", async () => {
    const { folder } = await opened();
    folder.setFilter("Plot");
    folder.openEntry(1);
    expect(folder.store.getState().filter).toBe("");
  });
});

/** The filter field over the list, as a pane's toolbar has it, with the keyboard commands on. */
function Pane({ folder }: { folder: Folder }) {
  useCommandKeys();
  return (
    <section>
      <FilterField folder={folder} pane="left" />
      <FileList folder={folder} pane="left" />
    </section>
  );
}

async function shown() {
  const { folder } = await opened();
  render(<Pane folder={folder} />);
  return folder;
}

const field = () => screen.getByRole("textbox", { name: "Quick filter" });
const grid = () => screen.getByRole("grid", { name: "Left pane files" });
const rowNames = () =>
  within(grid())
    .getAllByRole("row")
    .slice(1)
    .map((row) => within(row).getAllByRole("gridcell")[0]?.textContent);
const type = (text: string) => fireEvent.change(field(), { target: { value: text } });

describe("the filter field", () => {
  test("filters the list as you type and says how many it shows", async () => {
    await shown();
    expect(screen.getByText("Ctrl+F")).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();

    type("ნ");
    type("ნახ");
    expect(rowNames()).toEqual(["ᲜᲐᲮᲐᲖᲘ 1.dwg", "ნახაზი 2.pdf"]);
    expect(screen.getByRole("status")).toHaveTextContent("Showing 2 of 5");
    expect(screen.queryByText("Ctrl+F")).not.toBeInTheDocument();
    expect(grid()).toHaveAttribute("aria-rowcount", "3");

    type("rev*");
    expect(rowNames()).toEqual([]);
    expect(screen.getByText("No items match the filter")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Showing 0 of 5");
  });

  test("Esc clears it and gives the keyboard back to the list", async () => {
    const folder = await shown();
    field().focus();
    type("*.pdf");

    fireEvent.keyDown(field(), { key: "Escape", code: "Escape" });

    expect(field()).toHaveValue("");
    expect(folder.store.getState().filter).toBe("");
    expect(rowNames()).toHaveLength(5);
    expect(grid()).toHaveFocus();
  });

  test("Enter goes to the list and keeps it", async () => {
    await shown();
    field().focus();
    type("plan");

    fireEvent.keyDown(field(), { key: "Enter", code: "Enter" });

    expect(grid()).toHaveFocus();
    expect(rowNames()).toEqual(["plan_rev2.dwg"]);
  });

  test("the clear button clears it", async () => {
    await shown();
    type("plan");

    fireEvent.click(screen.getByRole("button", { name: "Clear filter (Esc)" }));

    expect(field()).toHaveValue("");
    expect(field()).toHaveFocus();
    expect(rowNames()).toHaveLength(5);
  });

  test("Ctrl+F puts the keyboard in it, its text selected to type over", async () => {
    await shown();
    type("plan");
    grid().focus();

    const event = new KeyboardEvent("keydown", {
      code: "KeyF",
      key: "ფ",
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    });
    act(() => {
      grid().dispatchEvent(event);
    });

    expect(event.defaultPrevented).toBe(true);
    expect(field()).toHaveFocus();
    const input = field() as HTMLInputElement;
    expect([input.selectionStart, input.selectionEnd]).toEqual([0, 4]);
  });
});

describe("the frame budget", () => {
  // The PRD's budget for 100 000 names is one frame. The best of a few runs, so a GC pause on a
  // busy CI machine doesn't fail it; `pnpm bench` gives the numbers.
  test("100 000 names filter within one frame (16 ms)", () => {
    const entries = synthEntries(synthNames(100_000));
    const all = visibleRows(entries, undefined, false);
    for (const pattern of SYNTH_PATTERNS) {
      let best = Number.POSITIVE_INFINITY;
      let hits = 0;
      for (let run = 0; run < 5; run++) {
        const started = performance.now();
        hits = matchingRows(entries, all, wildcardMatcher(pattern)).length;
        best = Math.min(best, performance.now() - started);
      }
      expect(best, `${pattern}: ${hits} of 100 000`).toBeLessThan(16);
    }
  });
});

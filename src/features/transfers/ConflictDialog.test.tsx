import { mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import type { ConflictItem, TransferConflict, TransferJob, TransferList } from "../../ipc";
import { useTransfers } from "../../stores/transfers";
import { dateText } from "../file-list/format";
import { ConflictDialog, parentOf } from "./ConflictDialog";

const MB = 1024 * 1024;
const NEW = Date.UTC(2026, 10, 10, 9, 30);
const OLD = Date.UTC(2026, 9, 1, 14, 5);

function item(overrides: Partial<ConflictItem> = {}): ConflictItem {
  return { folder: false, size: 3 * MB, modified: NEW, icon: 7, ...overrides };
}

function conflict(overrides: Partial<TransferConflict> = {}): TransferConflict {
  return {
    source: String.raw`C:\Users\თენგო\პროექტი\ნახაზი.dwg`,
    target: String.raw`D:\არქივი\ნახაზი.dwg`,
    incoming: item(),
    existing: item({ size: 1024, modified: OLD }),
    choices: ["replace", "skip", "keepBoth", "replaceIfNewer"],
    noRecycleBin: false,
    ...overrides,
  };
}

function job(id: number, asked?: TransferConflict): TransferJob {
  return {
    id,
    kind: "copy",
    state: asked ? { kind: "conflict", conflict: asked } : { kind: "running" },
    source: String.raw`C:\Users\თენგო\პროექტი`,
    sources: 3,
    destination: String.raw`D:\არქივი`,
    progress: {
      id,
      phase: "working",
      file: null,
      filesDone: 1,
      filesTotal: 3,
      bytesDone: MB,
      bytesTotal: 3 * MB,
      speed: 0,
      failed: 0,
    },
  };
}

function listed(generation: number, jobs: TransferJob[]): void {
  useTransfers.setState({ list: { generation, jobs } });
}

/** A fake core that records `conflict_resolve` and `transfer_control` as `skip 2 all`, `cancel 2`. */
function fakeCore(): string[] {
  const calls: string[] = [];
  mockIPC((cmd, args) => {
    if (cmd === "conflict_resolve") {
      const { job, answer } = args as { job: number; answer: { choice: string; all: boolean } };
      calls.push(`${answer.choice} ${job}${answer.all ? " all" : ""}`);
    }
    if (cmd === "transfer_control") {
      const { action, job } = args as { action: string; job: number };
      calls.push(`${action} ${job}`);
    }
    return null;
  });
  return calls;
}

function shown(list: TransferList) {
  useTransfers.setState({ list });
  return render(<ConflictDialog />);
}

afterEach(() => {
  useTransfers.setState({ list: { generation: -1, jobs: [] } });
});

test("no job asks, no window", () => {
  shown({ generation: 1, jobs: [job(1)] });
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("both files with size, date and icon, and the newer one marked", () => {
  shown({ generation: 1, jobs: [job(1), job(2, conflict())] });
  const dialog = screen.getByRole("dialog", { name: "A file with this name is already there" });
  expect(dialog).toHaveTextContent("ნახაზი.dwg is already in არქივი.");
  expect(dialog).toHaveTextContent("Copying from პროექტი");
  expect(dialog).toHaveTextContent("Already in არქივი");
  expect(dialog).toHaveTextContent("3 MB");
  expect(dialog).toHaveTextContent("1 KB");
  const newer = within(dialog).getByText(`${dateText(NEW, "DD.MM.YYYY")} · Newer`);
  expect(newer).toHaveClass("font-semibold");
  expect(within(dialog).getByText(dateText(OLD, "DD.MM.YYYY"))).not.toHaveClass("font-semibold");
  const images = [...dialog.querySelectorAll("img")].map((img) => img.getAttribute("src") ?? "");
  expect(images.filter((src) => src.includes("/type-7-"))).toHaveLength(2);
  // Each side's thumbnail, by job and side, over its icon until it is in.
  const thumbs = images.filter((src) => src.startsWith("http://thumb.localhost/"));
  expect(thumbs.map((src) => new URL(src).pathname.replace(/-\d+$/, ""))).toEqual([
    "/conflict-2-incoming",
    "/conflict-2-existing",
  ]);
  const choices = within(dialog)
    .getAllByRole("button")
    .map((button) => button.textContent);
  expect(choices).toEqual([
    "ReplaceThe one there goes to the Recycle BinR",
    "SkipLeave the one there as it isS",
    "Keep bothThe new one gets a numbered nameK",
    "Replace if newerReplace it only if the new one was modified laterN",
    "Cancel transfer",
  ]);
});

test("a click answers, with Apply to all if it is ticked", () => {
  const calls = fakeCore();
  shown({ generation: 1, jobs: [job(2, conflict())] });
  fireEvent.click(screen.getByRole("checkbox", { name: /Apply to all/ }));
  fireEvent.click(screen.getByRole("button", { name: "Keep both" }));
  expect(calls).toEqual(["keepBoth 2 all"]);
  expect(screen.queryByRole("dialog")).toBeNull();
});

/** A key press where the focus is. */
function press(code: string, key: string, init: KeyboardEventInit = {}): void {
  fireEvent.keyDown(document.activeElement ?? document.body, { code, key, ...init });
}

test("the keyboard answers: letters, A for all, arrows between the choices", () => {
  const calls = fakeCore();
  shown({ generation: 1, jobs: [job(2, conflict())] });
  const replace = screen.getByRole("button", { name: "Replace" });
  expect(replace).toHaveFocus();
  expect(replace).toHaveAccessibleDescription("The one there goes to the Recycle Bin");
  expect(replace).toHaveAttribute("aria-keyshortcuts", "R");
  press("ArrowDown", "ArrowDown");
  expect(screen.getByRole("button", { name: "Skip" })).toHaveFocus();
  press("ArrowUp", "ArrowUp");
  press("ArrowUp", "ArrowUp");
  expect(screen.getByRole("button", { name: "Replace if newer" })).toHaveFocus();

  // The Georgian layout: the key's code is what counts.
  press("KeyA", "ა");
  expect(screen.getByRole("checkbox")).toBeChecked();
  press("KeyS", "ს");
  expect(calls).toEqual(["skip 2 all"]);
});

test("a letter that isn't offered does nothing, nor does one with Ctrl", () => {
  const calls = fakeCore();
  shown({ generation: 1, jobs: [job(2, conflict())] });
  press("KeyM", "m");
  press("KeyR", "r", { ctrlKey: true });
  expect(calls).toEqual([]);
});

test("Esc and Cancel transfer cancel the job that asks", () => {
  const calls = fakeCore();
  shown({ generation: 1, jobs: [job(2, conflict())] });
  fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape", code: "Escape" });
  expect(calls).toEqual(["cancel 2"]);
});

test("a folder offers Merge and Keep both and shows no size", () => {
  shown({
    generation: 1,
    jobs: [
      job(
        3,
        conflict({
          source: String.raw`C:\Work\პროექტი`,
          target: String.raw`D:\პროექტი`,
          incoming: item({ folder: true, size: 0 }),
          existing: item({ folder: true, size: 0, modified: null }),
          choices: ["merge", "keepBoth"],
        }),
      ),
    ],
  });
  const dialog = screen.getByRole("dialog", { name: "A folder with this name is already there" });
  expect(dialog).toHaveTextContent("პროექტი is already in D:\\.");
  expect(dialog).toHaveTextContent("Folder");
  expect(dialog).toHaveTextContent("Date unknown");
  expect(within(dialog).queryByText(/Newer/)).toBeNull();
  expect(screen.getByRole("button", { name: "Merge" })).toHaveFocus();
  expect(screen.queryByRole("button", { name: "Skip" })).toBeNull();
  // Folders have no thumbnail: their icons only.
  const images = [...dialog.querySelectorAll("img")].map((img) => img.getAttribute("src") ?? "");
  expect(images).toHaveLength(2);
  expect(images.every((src) => src.startsWith("http://icon.localhost/"))).toBe(true);
});

test("where there is no Recycle Bin it warns and Skip has the focus", () => {
  shown({
    generation: 1,
    jobs: [job(2, conflict({ choices: ["replace", "skip", "keepBoth"], noRecycleBin: true }))],
  });
  const dialog = screen.getByRole("dialog", { name: "This file can't go to the Recycle Bin" });
  expect(dialog).toHaveTextContent("deletes it for good");
  expect(dialog).toHaveTextContent("Replace and delete the one there for good");
  expect(screen.getByRole("button", { name: "Skip" })).toHaveFocus();
});

test("once answered it waits for the queue, then shows what comes next", () => {
  fakeCore();
  shown({ generation: 1, jobs: [job(2, conflict()), job(4, conflict())] });
  const thumbsOf = () =>
    [...screen.getByRole("dialog").querySelectorAll("img")]
      .map((img) => img.getAttribute("src") ?? "")
      .filter((src) => src.startsWith("http://thumb.localhost/"));
  const before = thumbsOf();
  fireEvent.click(screen.getByRole("button", { name: "Skip" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  act(() => listed(1, [job(2, conflict()), job(4, conflict())]));
  expect(screen.queryByRole("dialog")).toBeNull();

  const next = conflict({ target: String.raw`D:\არქივი\გეგმა.pdf` });
  act(() => listed(2, [job(2), job(4, next)]));
  expect(screen.getByRole("dialog")).toHaveTextContent("გეგმა.pdf is already in არქივი.");
  // The same job, another question: other thumbnails.
  expect(thumbsOf()).toHaveLength(2);
  expect(thumbsOf().some((src) => before.includes(src))).toBe(false);
  expect(screen.getByRole("checkbox")).not.toBeChecked();
  expect(screen.getByRole("button", { name: "Replace" })).toHaveFocus();
});

test("a path's folder, a drive's root stays whole", () => {
  expect(parentOf(String.raw`C:\Work\a.dwg`)).toBe(String.raw`C:\Work`);
  expect(parentOf(String.raw`C:\a.dwg`)).toBe("C:\\");
  expect(parentOf(String.raw`\\nas\share\a.dwg`)).toBe(String.raw`\\nas\share`);
});

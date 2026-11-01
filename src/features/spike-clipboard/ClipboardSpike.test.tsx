import { mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, test } from "vitest";
import type { ClipboardState, CopyReport, PasteReport, SampleEntry } from "../../ipc";
import { ClipboardSpike } from "./ClipboardSpike";

const entries: SampleEntry[] = [
  { id: 0, name: "საქაღალდე", isDir: true, size: 0 },
  { id: 1, name: "ნახაზი — სართული 1.dwg", isDir: false, size: 12 },
  { id: 2, name: "ხარჯთაღრიცხვა 2026.pdf", isDir: false, size: 2048 },
];

const copyReport: CopyReport = {
  files: 2,
  op: "cut",
  queuedMs: 0.3,
  prepareMs: 4,
  setMs: 1.5,
  formats: [],
};

const pasteReport: PasteReport = {
  op: "copy",
  preferredEffect: 5,
  fromJerry: false,
  pasted: ["a.txt"],
  skipped: [],
  readMs: 1,
  pasteMs: 2,
};

function setup(clipboard: Partial<ClipboardState> = {}) {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    switch (cmd) {
      case "sample_list":
        return entries;
      case "clipboard_state":
        return {
          owner: "none",
          ownerPid: null,
          files: 0,
          op: null,
          formats: [],
          cutIds: [],
          ...clipboard,
        };
      case "clipboard_copy":
        return copyReport;
      case "clipboard_paste":
        return pasteReport;
      default:
        return undefined;
    }
  });
  render(<ClipboardSpike />);
  return calls;
}

test("Ctrl+X by key code cuts the selected rows", async () => {
  const calls = setup();
  fireEvent.click(await screen.findByText("ნახაზი — სართული 1.dwg"));
  fireEvent.click(screen.getByText("ხარჯთაღრიცხვა 2026.pdf"), {
    button: 0,
    ctrlKey: true,
  });

  // The Georgian layout types "ხ" on that key; the code is still KeyX.
  fireEvent.keyDown(window, { code: "KeyX", key: "ხ", ctrlKey: true });

  await waitFor(() =>
    expect(calls).toContainEqual({ cmd: "clipboard_copy", args: { ids: [1, 2], cut: true } }),
  );
  expect(await screen.findByTestId("clip-status")).toHaveTextContent("Cut 2 item(s)");
});

test("Ctrl+V pastes and lists the folder again", async () => {
  const calls = setup();
  await screen.findByText("საქაღალდე");
  const listsBefore = calls.filter((c) => c.cmd === "sample_list").length;

  fireEvent.keyDown(window, { code: "KeyV", key: "ვ", ctrlKey: true });

  expect(await screen.findByTestId("clip-status")).toHaveTextContent(
    "Copied 1 item(s) from another app (Preferred DropEffect 5)",
  );
  expect(calls.filter((c) => c.cmd === "sample_list").length).toBeGreaterThan(listsBefore);
});

test("rows of Jerry's cut are dimmed while the clipboard holds it", async () => {
  setup({ owner: "jerry", files: 1, op: "cut", cutIds: [2] });

  const row = (await screen.findByText("ხარჯთაღრიცხვა 2026.pdf")).closest("[role=option]");
  await waitFor(() => expect(row).toHaveAttribute("data-cut", "true"));
  expect(screen.getByText("საქაღალდე").closest("[role=option]")).not.toHaveAttribute("data-cut");
  expect(screen.getByTestId("clip-state")).toHaveTextContent("1 file(s), Cut · owner: Jerry");
});

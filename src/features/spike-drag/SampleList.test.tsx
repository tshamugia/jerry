import { mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, test } from "vitest";
import { SampleList } from "./SampleList";

let dragCalls: { ids: number[] }[] = [];

beforeEach(() => {
  dragCalls = [];
  mockIPC((cmd, args) => {
    if (cmd === "sample_list") {
      return [
        { id: 0, name: "საქაღალდე", isDir: true, size: 0 },
        { id: 1, name: "ნახაზი — სართული 1.dwg", isDir: false, size: 12 },
        { id: 2, name: "plain.pdf", isDir: false, size: 2048 },
      ];
    }
    if (cmd === "drag_start") {
      dragCalls.push(args as { ids: number[] });
      return {
        started: true,
        dropped: true,
        effect: "copy",
        performed: null,
        uiToRustMs: 1,
        queuedMs: 0.2,
        prepareMs: 3,
        dragMs: 900,
        capture: null,
      };
    }
    return undefined;
  });
});

function press(row: HTMLElement, init: { button?: number; ctrlKey?: boolean } = {}) {
  fireEvent.pointerDown(row, { button: 0, buttons: 1, clientX: 10, clientY: 10, ...init });
}

test("a move past the threshold with the button held starts one OLE drag", async () => {
  render(<SampleList />);
  const row = await screen.findByText("ნახაზი — სართული 1.dwg");

  press(row);
  fireEvent.pointerMove(window, { buttons: 1, clientX: 12, clientY: 11 }); // under the threshold
  expect(dragCalls).toHaveLength(0);
  fireEvent.pointerMove(window, { buttons: 1, clientX: 15, clientY: 10 });
  fireEvent.pointerMove(window, { buttons: 1, clientX: 30, clientY: 10 });

  expect(await screen.findByText(/Dropped: copy/)).toBeInTheDocument();
  expect(dragCalls).toHaveLength(1);
  expect(dragCalls[0]?.ids).toEqual([1]);
});

test("the whole selection drags, by id", async () => {
  render(<SampleList />);
  const dwg = await screen.findByText("ნახაზი — სართული 1.dwg");
  const pdf = screen.getByText("plain.pdf");

  press(dwg);
  fireEvent.pointerUp(dwg);
  press(pdf, { ctrlKey: true });
  fireEvent.pointerUp(pdf, { ctrlKey: true });
  press(dwg);
  fireEvent.pointerMove(window, { buttons: 1, clientX: 40, clientY: 40 });

  await screen.findByText(/Dropped/);
  expect(dragCalls[0]?.ids.sort()).toEqual([1, 2]);
});

test("a released button means a click, not a drag", async () => {
  render(<SampleList />);
  const row = await screen.findByText("plain.pdf");

  press(row);
  fireEvent.pointerMove(window, { buttons: 0, clientX: 40, clientY: 40 });
  expect(dragCalls).toHaveLength(0);
});

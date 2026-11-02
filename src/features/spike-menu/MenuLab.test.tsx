import { mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import type { LabItem } from "../../ipc";
import { caseIds, MenuLab } from "./MenuLab";

const items: LabItem[] = [
  { id: 0, name: "ნახაზი — სართული 1.dwg", kind: "file" },
  { id: 1, name: "plain.txt", kind: "file" },
  { id: 2, name: "პროექტი", kind: "folder" },
  { id: 3, name: "გრძელი გზა.txt", kind: "long" },
  { id: 4, name: "elsewhere.txt", kind: "elsewhere" },
];

test("each case opens the menu for the right items", () => {
  expect(caseIds(items)).toEqual({
    oneFile: [0],
    severalFiles: [0, 1],
    folder: [2],
    longPath: [3],
    mixedFolders: [0, 4],
  });
});

test("right-click asks Rust for Windows' menu of the selection and shows the outcome", async () => {
  const menus: unknown[] = [];
  mockIPC(
    (cmd, args) => {
      if (cmd === "lab_items") return items;
      if (cmd === "windows_menu") {
        menus.push(args);
        return {
          seq: 1,
          via: "jerry",
          chosen: "Compressed (zipped) folder",
          verb: null,
          invoked: true,
          queuedMs: 1,
          resolveMs: 1,
          uiObjectMs: 1,
          queryMs: 1,
          menuMs: 1,
          invokeMs: 1,
          popups: [],
          initPopup: 2,
          measureItem: 0,
          drawItem: 0,
          menuChar: 0,
          ackDuringMenu: true,
          ackMs: 3,
        };
      }
      return undefined;
    },
    { shouldMockEvents: true },
  );

  render(<MenuLab />);
  const drawing = await screen.findByText("ნახაზი — სართული 1.dwg");
  fireEvent.click(drawing);
  fireEvent.click(screen.getByText("plain.txt"), { ctrlKey: true });
  fireEvent.contextMenu(drawing, { clientX: 10.4, clientY: 20.6, shiftKey: true });

  expect(
    await screen.findByText('Chose "Compressed (zipped) folder". Events during the menu: yes.'),
  ).toBeInTheDocument();
  expect(menus).toEqual([{ ids: [0, 1], x: 10, y: 21, extended: true }]);
});

test("explains why items of different folders get no Windows menu", async () => {
  mockIPC(
    (cmd) => {
      if (cmd === "lab_items") return items;
      if (cmd === "windows_menu") throw { code: "MixedFolders" };
      return undefined;
    },
    { shouldMockEvents: true },
  );

  render(<MenuLab />);
  fireEvent.click(await screen.findByText("Mixed folders"));

  expect(await screen.findByRole("alert")).toHaveTextContent(/different folders/);
});

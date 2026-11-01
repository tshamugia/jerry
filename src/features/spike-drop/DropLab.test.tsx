import { mockIPC } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/react";
import { beforeEach, expect, test } from "vitest";
import { DropLab } from "./DropLab";

const calls: string[] = [];

beforeEach(() => {
  calls.length = 0;
  mockIPC(
    (cmd, args) => {
      calls.push(cmd);
      if (cmd === "lab_folders") {
        return [
          { id: 0, name: "jerry-spike-14", parent: null, depth: 0 },
          { id: 1, name: "Drawings", parent: 0, depth: 1 },
          { id: 2, name: "Outlook", parent: 0, depth: 1 },
        ];
      }
      if (cmd === "lab_list") {
        const folder = (args as { folder: number }).folder;
        return folder === 0
          ? [
              { id: 1, name: "Drawings", isDir: true, size: 0 },
              { id: 5, name: "ნახაზი.dwg", isDir: false, size: 12 },
            ]
          : [];
      }
      if (cmd === "dnd_install") {
        return { before: [], after: [], registered: 4, webviewDropOff: true };
      }
      return undefined;
    },
    { shouldMockEvents: true },
  );
});

test("every kind of drop target carries its folder for the hit test", async () => {
  render(<DropLab />);

  // A file row isn't a target itself: it drops into the list's folder.
  const file = await screen.findByText("ნახაზი.dwg");
  expect(file.closest("[data-drop]")?.getAttribute("data-drop")).toBe("pane");

  const row = screen.getAllByText("Drawings").map((el) => el.closest("[data-drop]"));
  const kinds = row.map((el) => el?.getAttribute("data-drop")).sort();
  // The Drawings tab, its sidebar favorite and its row in the list.
  expect(kinds).toEqual(["row", "sidebar", "tab"]);
  for (const el of row) expect(el?.getAttribute("data-folder")).toBe("1");

  expect(await screen.findByText(/Drop target on 4 windows/)).toBeInTheDocument();
  expect(calls).toContain("dnd_install");
});

import { afterEach, expect, test, vi } from "vitest";
import { strings } from "../../strings";
import { dropHit, dropHitAt } from "./target";

afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

function mount(html: string) {
  document.body.innerHTML = html;
}

test("a child of a drop target resolves to that target", () => {
  mount(`
    <div data-drop="row" data-drop-key="row:7" data-folder="7" data-name="ნახაზები">
      <span id="label">ნახაზები</span>
    </div>`);
  expect(dropHit(document.getElementById("label"))).toEqual({
    key: "row:7",
    kind: "row",
    folder: 7,
    name: "ნახაზები",
  });
});

test("places that refuse drops, and unmarked ones, resolve to nothing", () => {
  mount(`
    <nav data-drop="none"><div id="network" data-drop-key="side:network">Network</div></nav>
    <p id="plain">text</p>`);
  expect(dropHit(document.getElementById("network"))).toBeNull();
  expect(dropHit(document.getElementById("plain"))).toBeNull();
  expect(dropHit(null)).toBeNull();
});

test("a file row inside the list drops into the list's folder", () => {
  mount(`
    <div data-drop="pane" data-drop-key="pane" data-folder="3" data-name="Drawings">
      <div id="file" data-drop-key="row:9">plan.dwg</div>
    </div>`);
  expect(dropHit(document.getElementById("file"))).toMatchObject({ kind: "pane", folder: 3 });
});

test("OLE's client pixels are divided by the device pixel ratio", () => {
  mount(
    `<div id="tab" data-drop="tab" data-drop-key="tab:1" data-folder="2" data-name="Outlook"></div>`,
  );
  const tab = document.getElementById("tab");
  const elementFromPoint = vi.fn(() => tab);
  document.elementFromPoint = elementFromPoint;
  expect(dropHitAt(300, 90, 1.5)).toMatchObject({ kind: "tab", folder: 2 });
  expect(elementFromPoint).toHaveBeenCalledWith(200, 60);
});

test("drop descriptions name the item count and keep %1 for Windows", () => {
  expect(strings.drop.labels(1)).toEqual({
    copy: "Copy to %1",
    moves: "Move to %1",
    link: "Create link in %1",
  });
  expect(strings.drop.labels(3).moves).toBe("Move 3 items to %1");
});

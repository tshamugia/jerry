import type { Channel } from "@tauri-apps/api/core";
import { mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { CaptionButtonRect, FrameState } from "../ipc";
import { fakeListings } from "../ipc/listing.testing";
import { addTab, resetTabs, type Tab, useTabs } from "../stores/tabs";
import { useWindow } from "../stores/window";
import { TOOLTIP_DELAY_MS } from "./CaptionButtons";
import { TitleBar } from "./TitleBar";

let calls: { cmd: string; args: unknown }[];
let frame: Channel<FrameState> | undefined;

beforeEach(() => {
  calls = [];
  frame = undefined;
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "title_bar_attach") {
      frame = (args as { updates: Channel<FrameState> }).updates;
    }
  });
});

afterEach(() => {
  vi.useRealTimers();
  resetTabs();
  useWindow.setState({ activePane: "left" });
});

/** The native frame state arrives from the core. */
function send(state: Partial<FrameState>) {
  act(() =>
    frame?.onmessage({ hovered: null, pressed: null, maximized: false, active: true, ...state }),
  );
}

/** A tab in `pane` on folder `name` (token `folder`), not opened. */
function tab(pane: "left" | "right", name: string, { pinned = false, folder = 1 } = {}): Tab {
  const path = `D:\\Projects\\${name}`;
  return addTab(pane, { pinned, from: { history: [{ folder, path, name, parent: null }] } });
}

const button = (name: string) => screen.getByRole("button", { name });

test("the app mark over the sidebar, a tab strip per pane and the caption buttons", () => {
  render(<TitleBar sidebar preview dual />);

  const bar = screen.getByRole("banner");
  expect(bar).toHaveClass("drag-region", "frame-columns", "h-titlebar");
  expect(within(bar).getByText("Jerry")).toBeInTheDocument();
  expect(screen.getByRole("tablist", { name: "Left pane tabs" })).not.toHaveClass(
    "pr-caption-btns",
  );
  // Over the preview, the caption buttons leave the right strip alone.
  expect(screen.getByRole("tablist", { name: "Right pane tabs" })).not.toHaveClass(
    "pr-caption-btns",
  );
  for (const name of ["Minimize", "Maximize", "Close"]) {
    expect(button(name)).toHaveClass("w-caption-btn", "h-titlebar");
  }
  for (const add of screen.getAllByRole("button", { name: "New tab (Ctrl+T)" })) {
    expect(add).toBeEnabled();
  }
});

test("single-pane mode has one strip; without the preview it keeps clear of the buttons", () => {
  render(<TitleBar sidebar={false} preview={false} dual={false} />);

  expect(screen.queryByText("Jerry")).toBeNull();
  const strips = screen.getAllByRole("tablist");
  expect(strips).toHaveLength(1);
  expect(strips[0]).toHaveAccessibleName("Left pane tabs");
  expect(strips[0]).toHaveClass("pr-caption-btns");
});

test("the caption buttons tell the core where they are and follow the frame state", () => {
  render(<TitleBar sidebar preview dual />);

  const placed = calls.find((call) => call.cmd === "title_bar_set_buttons");
  const { buttons } = (placed?.args ?? { buttons: [] }) as { buttons: CaptionButtonRect[] };
  expect(buttons.map((rect) => rect.button)).toEqual(["minimize", "maximize", "close"]);
  expect(frame).toBeDefined();

  const maximize = button("Maximize");
  expect(maximize).toHaveAttribute("data-look", "rest");
  send({ hovered: "maximize" });
  expect(maximize).toHaveAttribute("data-look", "hover");
  send({ hovered: "maximize", pressed: "maximize" });
  expect(maximize).toHaveAttribute("data-look", "pressed");
  // Held and dragged off: neither it nor the button under the mouse lights up.
  send({ hovered: "close", pressed: "maximize" });
  expect(maximize).toHaveAttribute("data-look", "rest");
  expect(button("Close")).toHaveAttribute("data-look", "rest");

  // Maximized, the button restores; in an inactive window the glyphs dim.
  send({ maximized: true, active: false });
  expect(button("Restore down")).toHaveAttribute("data-look", "inactive");
  expect(button("Minimize")).toHaveAttribute("data-look", "inactive");
});

test("a tooltip shows once the mouse rests on a caption button", () => {
  vi.useFakeTimers();
  render(<TitleBar sidebar preview dual />);
  const tooltip = () => document.querySelector("[data-tooltip]");

  send({ hovered: "close" });
  act(() => vi.advanceTimersByTime(TOOLTIP_DELAY_MS - 1));
  expect(tooltip()).toBeNull();
  act(() => vi.advanceTimersByTime(1));
  expect(button("Close")).toContainElement(tooltip() as HTMLElement);
  expect(tooltip()).toHaveTextContent("Close");

  // Pressing or leaving hides it.
  send({ hovered: "close", pressed: "close" });
  expect(tooltip()).toBeNull();
  send({ hovered: "maximize", maximized: true });
  act(() => vi.advanceTimersByTime(TOOLTIP_DELAY_MS));
  expect(tooltip()).toHaveTextContent("Restore down");
  send({});
  expect(tooltip()).toBeNull();
});

test("a click on a caption button (a screen reader's) presses it through the core", () => {
  render(<TitleBar sidebar preview dual />);
  expect(button("Close")).toHaveAttribute("tabindex", "-1");

  fireEvent.click(button("Close"));
  fireEvent.click(button("Maximize"));

  expect(calls.filter((call) => call.cmd === "title_bar_press")).toEqual([
    { cmd: "title_bar_press", args: { button: "close" } },
    { cmd: "title_bar_press", args: { button: "maximize" } },
  ]);
});

test("a strip shows pinned tabs first, then the others; long Georgian names truncate", () => {
  const drawings = tab("left", "Drawings");
  tab("left", "PROJECTS", { pinned: true });
  tab("left", "ფაილები და ნახაზები");
  render(<TitleBar sidebar preview dual />);

  const strip = screen.getByRole("tablist", { name: "Left pane tabs" });
  const [pinned, shown, other] = within(strip).getAllByRole("tab");
  expect(pinned).toHaveAccessibleName("PROJECTS (pinned tab)");
  expect(pinned).toHaveClass("w-10");
  expect(within(pinned as HTMLElement).queryByRole("button")).toBeNull();
  // A separator ends the pinned group.
  expect(pinned?.nextElementSibling).toHaveAttribute("role", "presentation");

  expect(useTabs.getState().left.active).toBe(drawings.id);
  expect(shown).toHaveAttribute("aria-selected", "true");
  expect(shown).toHaveClass("w-tab-active", "bg-bar");
  // The full path is the tooltip.
  expect(shown).toHaveAttribute("title", String.raw`D:\Projects\Drawings`);
  // The active pane's shown tab gets the primary text color.
  expect(within(shown as HTMLElement).getByText("Drawings")).toHaveClass("text-fg");

  expect(other).toHaveAttribute("aria-selected", "false");
  expect(other).toHaveClass("w-tab-inactive");
  expect(within(other as HTMLElement).getByText("ფაილები და ნახაზები")).toHaveClass(
    "truncate",
    "text-fg-2",
  );
});

test("pressing a tab shows it and activates its pane; its close button closes it", () => {
  const issued = tab("right", "04_Issued");
  const tengo = tab("right", "tengo");
  render(<TitleBar sidebar preview dual />);
  const strip = screen.getByRole("tablist", { name: "Right pane tabs" });
  const [first, second] = within(strip).getAllByRole("tab") as HTMLElement[];
  // The right pane isn't active: its shown tab's name stays secondary.
  expect(within(first as HTMLElement).getByText("04_Issued")).toHaveClass("text-fg-2");

  fireEvent.pointerDown(second as HTMLElement, { button: 0 });
  expect(useTabs.getState().right.active).toBe(tengo.id);
  expect(useWindow.getState().activePane).toBe("right");
  expect(second).toHaveAttribute("aria-selected", "true");

  fireEvent.keyDown(first as HTMLElement, { key: "Enter" });
  expect(useTabs.getState().right.active).toBe(issued.id);

  // The close button doesn't show the tab it closes.
  const close = within(second as HTMLElement).getByRole("button", { name: "Close tab" });
  fireEvent.pointerDown(close, { button: 0 });
  fireEvent.click(close);
  expect(useTabs.getState().right.tabs).toEqual([{ id: issued.id, pinned: false }]);
  expect(useTabs.getState().right.active).toBe(issued.id);
  // A pane keeps its last tab: it has no close button.
  expect(within(strip).queryByRole("button", { name: "Close tab" })).toBeNull();
});

test("a middle click closes a tab; the + button opens the folder shown in a new tab", () => {
  const issued = tab("left", "04_Issued", { folder: 7 });
  const other = tab("left", "tengo");
  render(<TitleBar sidebar preview dual />);
  const strip = screen.getByRole("tablist", { name: "Left pane tabs" });

  fireEvent(
    within(strip).getByText("tengo"),
    new MouseEvent("auxclick", { bubbles: true, button: 1 }),
  );
  expect(useTabs.getState().left.tabs.map((each) => each.id)).toEqual([issued.id]);
  expect(other.folder.store.getState().status).toBe("none");

  const core = fakeListings();
  fireEvent.click(within(strip).getByRole("button", { name: "New tab (Ctrl+T)" }));
  const { tabs, active } = useTabs.getState().left;
  expect(tabs).toHaveLength(2);
  expect(active).toBe(tabs[1]?.id);
  expect(core.calls.find((call) => call.cmd === "listing_open")?.args.target).toEqual({
    kind: "folder",
    folder: 7,
  });
});

test("a tab's context menu pins it, closes the others and copies its path", async () => {
  const projects = tab("left", "PROJECTS", { folder: 9 });
  tab("left", "Drawings");
  tab("left", "Downloads");
  render(<TitleBar sidebar preview dual />);
  const menuOn = (name: string) => {
    fireEvent.contextMenu(screen.getByText(name).closest('[role="tab"]') as HTMLElement);
    return screen.findByRole("menu", { name: "Tab" });
  };

  fireEvent.click(within(await menuOn("PROJECTS")).getByRole("menuitem", { name: "Pin tab" }));
  expect(useTabs.getState().left.tabs[0]).toEqual({ id: projects.id, pinned: true });

  fireEvent.click(
    within(await menuOn("Drawings")).getByRole("menuitem", { name: "Close other tabs" }),
  );
  // The pinned tab stays.
  expect(useTabs.getState().left.tabs).toHaveLength(2);

  const menu = await menuOn("Drawings");
  expect(within(menu).getByRole("menuitem", { name: "Close tabs to the right" })).toHaveAttribute(
    "data-disabled",
  );
  fireEvent.click(within(menu).getByRole("menuitem", { name: "Copy path" }));
  expect(calls.at(-1)).toEqual({ cmd: "folder_copy_path", args: { folder: 1 } });
});

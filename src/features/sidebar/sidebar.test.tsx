import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, test } from "vitest";
import { commandById, runCommand } from "../../commands/registry";
import type { Drive, DriveList, JerryError } from "../../ipc";
import { fakeListings } from "../../ipc/listing.testing";
import { useSettings } from "../../stores/settings";
import { defaultSettings } from "../../stores/settings.testing";
import { addTab, resetTabs, useTabs } from "../../stores/tabs";
import { useWindow } from "../../stores/window";
import { Sidebar } from "./Sidebar";
import { resetSidebar, syncSidebar, useSidebar } from "./store";

const GB = 1024 ** 3;

function drive(letter: string, more: Partial<Drive> = {}): Drive {
  return {
    letter,
    kind: "fixed",
    label: null,
    remote: null,
    connected: true,
    ejectable: false,
    space: { total: 100 * GB, free: 40 * GB },
    ...more,
  };
}

const places = [
  { place: "desktop", path: String.raw`C:\Users\TS\Desktop`, name: null },
  { place: "downloads", path: String.raw`C:\Users\TS\Downloads`, name: null },
  {
    place: "onedrive",
    path: String.raw`C:\Users\TS\OneDrive - Insta LLC`,
    name: "OneDrive - Insta LLC",
  },
] as const;

function show(drives: Drive[], generation = 1) {
  useSidebar.setState({
    places: [...places],
    drives: { generation, drives },
    network: [
      { path: String.raw`\\INSTA-NAS01\Projects`, name: "Projects", server: "INSTA-NAS01" },
    ],
  });
}

let stop: (() => void) | undefined;

beforeEach(() => {
  useSettings.setState({ settings: defaultSettings });
});

afterEach(() => {
  stop?.();
  stop = undefined;
  resetSidebar();
  resetTabs();
  useWindow.setState({ activePane: "left" });
  useSettings.setState({ settings: undefined });
});

const nav = () => screen.getByRole("navigation", { name: "Navigation" });

test("shows the places, the drives with their free space, network and Recycle Bin", () => {
  mockIPC(() => null);
  show([
    drive("C"),
    drive("D", { label: "Data", space: { total: 100 * GB, free: 5 * GB } }),
    drive("Z", { kind: "network", label: "Projects", remote: String.raw`\\INSTA-NAS01\Projects` }),
  ]);
  render(<Sidebar />);

  for (const name of ["Desktop", "Downloads", "OneDrive - Insta LLC", "Recycle Bin"]) {
    expect(within(nav()).getByRole("button", { name })).toBeInTheDocument();
  }
  // Explorer's names: the label, or the kind's name without one.
  const c = within(nav()).getByRole("button", { name: /Local Disk \(C:\)/ });
  expect(c).toHaveTextContent("40 GB free");
  expect(within(c).getByText("40 GB free")).toHaveClass("text-fg-3");
  // 95 % used: red, as the PRD says from 90 %.
  const d = within(nav()).getByRole("button", { name: /Data \(D:\)/ });
  expect(within(d).getByText("5 GB free")).toHaveClass("text-red");
  expect(d.querySelector(".bg-red")).toHaveStyle({ width: "95%" });
  // A mapped drive goes under Network, with its server; so does an added location.
  expect(
    within(nav()).getByRole("button", { name: /Projects \(Z:\)\s*INSTA-NAS01/ }),
  ).toBeInTheDocument();
  expect(
    within(nav()).getByRole("button", { name: /^Projects\s*INSTA-NAS01$/ }),
  ).toBeInTheDocument();
});

test("a click opens the place in the active tab, Ctrl+click and a middle click in a new tab", async () => {
  const core = fakeListings({ path: "C:\\" });
  const tab = addTab("left");
  show([drive("C")]);
  render(<Sidebar />);
  const c = within(nav()).getByRole("button", { name: /Local Disk \(C:\)/ });

  fireEvent.click(c);
  await waitFor(() => expect(tab.current()?.path).toBe("C:\\"));
  expect(core.calls.find((call) => call.cmd === "listing_open")?.args.target).toEqual({
    kind: "path",
    path: "C:\\",
  });
  // The folder the active tab shows is the selected item.
  expect(c).toHaveAttribute("aria-current", "location");

  fireEvent.click(within(nav()).getByRole("button", { name: "Downloads" }), { ctrlKey: true });
  expect(useTabs.getState().left.tabs).toHaveLength(2);
  fireEvent(
    within(nav()).getByRole("button", { name: "Desktop" }),
    new MouseEvent("auxclick", { bubbles: true, button: 1 }),
  );
  expect(useTabs.getState().left.tabs).toHaveLength(3);
  const opened = core.calls
    .filter((call) => call.cmd === "listing_open")
    .map((call) => call.args.target);
  expect(opened.slice(1)).toEqual([
    { kind: "path", path: String.raw`C:\Users\TS\Downloads` },
    { kind: "path", path: String.raw`C:\Users\TS\Desktop` },
  ]);
});

test("the arrow keys move between the items", () => {
  mockIPC(() => null);
  show([drive("C")]);
  render(<Sidebar />);
  const desktop = within(nav()).getByRole("button", { name: "Desktop" });
  desktop.focus();

  fireEvent.keyDown(desktop, { key: "ArrowDown" });
  expect(within(nav()).getByRole("button", { name: "Downloads" })).toHaveFocus();
  fireEvent.keyDown(document.activeElement as Element, { key: "End" });
  expect(within(nav()).getByRole("button", { name: "Recycle Bin" })).toHaveFocus();
  fireEvent.keyDown(document.activeElement as Element, { key: "ArrowDown" });
  expect(within(nav()).getByRole("button", { name: "Recycle Bin" })).toHaveFocus();
  fireEvent.keyDown(document.activeElement as Element, { key: "Home" });
  expect(desktop).toHaveFocus();
});

test("Eject ejects the drive, and says so when Jerry couldn't", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  let fail: JerryError | undefined = { code: "Os", params: { os: 1460 }, path: null };
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "eject_check") {
      return { transfers: 0 };
    }
    if (cmd === "eject" && fail) {
      throw fail;
    }
    return null;
  });
  show([drive("C"), drive("E", { kind: "removable", label: "USB", ejectable: true })]);
  render(<Sidebar />);
  expect(within(nav()).queryByRole("button", { name: /Eject Local Disk/ })).toBeNull();

  fireEvent.click(within(nav()).getByRole("button", { name: "Eject USB (E:)" }));
  const dialog = await screen.findByRole("dialog", { name: "Couldn't eject the drive" });
  expect(calls).toContainEqual({ cmd: "eject", args: { letter: "E" } });

  fail = undefined;
  fireEvent.click(within(dialog).getByRole("button", { name: "Try again" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(calls.filter((call) => call.cmd === "eject")).toHaveLength(2);
});

test("Eject warns when transfers are on the drive and goes on only if told to", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return cmd === "eject_check" ? { transfers: 2 } : null;
  });
  show([drive("E", { kind: "removable", label: "USB", ejectable: true })]);
  render(<Sidebar />);
  const ejects = () => calls.filter((call) => call.cmd === "eject");

  fireEvent.click(within(nav()).getByRole("button", { name: "Eject USB (E:)" }));
  let dialog = await screen.findByRole("dialog", { name: "Transfers are using this drive" });
  expect(dialog).toHaveTextContent("2 transfers are using E:");
  expect(calls).toContainEqual({ cmd: "eject_check", args: { letter: "E" } });
  expect(ejects()).toHaveLength(0);

  fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(ejects()).toHaveLength(0);

  fireEvent.click(within(nav()).getByRole("button", { name: "Eject USB (E:)" }));
  dialog = await screen.findByRole("dialog", { name: "Transfers are using this drive" });
  fireEvent.click(within(dialog).getByRole("button", { name: "Eject anyway" }));
  await waitFor(() => expect(ejects()).toHaveLength(1));
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
});

test("a network location is added by typing it, and removed from its menu", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    const text = (args as { text?: string }).text;
    if (cmd === "network_place_add" && !text?.startsWith("\\\\")) {
      throw { code: "InvalidName", path: null };
    }
    return null;
  });
  show([]);
  render(<Sidebar />);

  fireEvent.click(within(nav()).getByRole("button", { name: "Add a network location" }));
  const field = within(nav()).getByRole("textbox", { name: "Network location" });
  expect(field).toHaveFocus();
  fireEvent.change(field, { target: { value: "nas" } });
  fireEvent.keyDown(field, { key: "Enter" });
  expect(await within(nav()).findByText(/Type a network location such as/)).toHaveClass("text-red");

  fireEvent.change(field, { target: { value: String.raw`\\nas\პროექტები` } });
  fireEvent.keyDown(field, { key: "Enter" });
  await waitFor(() => expect(within(nav()).queryByRole("textbox")).toBeNull());
  expect(calls).toContainEqual({
    cmd: "network_place_add",
    args: { text: String.raw`\\nas\პროექტები` },
  });

  fireEvent.contextMenu(within(nav()).getByRole("button", { name: /^Projects/ }));
  fireEvent.click(await screen.findByRole("menuitem", { name: "Remove from sidebar" }));
  expect(calls).toContainEqual({
    cmd: "network_place_remove",
    args: { path: String.raw`\\INSTA-NAS01\Projects` },
  });
});

test("a section folds and unfolds", () => {
  mockIPC(() => null);
  show([drive("C")]);
  render(<Sidebar />);

  fireEvent.click(within(nav()).getByRole("button", { name: "Hide Drives" }));
  expect(within(nav()).queryByRole("button", { name: /Local Disk/ })).toBeNull();
  fireEvent.click(within(nav()).getByRole("button", { name: "Show Drives" }));
  expect(within(nav()).getByRole("button", { name: /Local Disk/ })).toBeInTheDocument();
});

test("Recycle Bin opens in Explorer", () => {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
  });
  show([]);
  render(<Sidebar />);
  fireEvent.click(within(nav()).getByRole("button", { name: "Recycle Bin" }));
  expect(calls).toContain("recycle_bin_open");
});

test("Ctrl+B and the sidebar's own button hide it, and the setting keeps it so", () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
  });
  show([]);
  render(<Sidebar />);

  const command = commandById("toggleSidebar");
  expect(command && runCommand(command)).toBe(true);
  fireEvent.click(within(nav()).getByRole("button", { name: "Show or hide sidebar (Ctrl+B)" }));
  expect(calls).toEqual([
    { cmd: "settings_set", args: { key: "showSidebar", value: false } },
    { cmd: "settings_set", args: { key: "showSidebar", value: false } },
  ]);
});

test("the store loads everything and keeps the newest drive list", async () => {
  const list = (generation: number, drives: Drive[]): DriveList => ({ generation, drives });
  let answer: ((list: DriveList) => void) | undefined;
  mockIPC(
    (cmd) => {
      switch (cmd) {
        case "drives_list":
          return new Promise<DriveList>((resolve) => {
            answer = resolve;
          });
        case "sidebar_places":
          return places;
        case "network_places_list":
          return [];
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
  stop = syncSidebar();
  await waitFor(() => expect(answer).toBeDefined());

  // A USB drive arrived after the list was read: the late answer is older and loses.
  await emit("drives:changed", list(3, [drive("C"), drive("E", { kind: "removable" })]));
  act(() => answer?.(list(2, [drive("C")])));
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(useSidebar.getState().drives.drives.map((each) => each.letter)).toEqual(["C", "E"]);
  expect(useSidebar.getState().places).toHaveLength(3);

  await emit("network-places:changed", [
    { path: String.raw`\\nas\share`, name: "share", server: "nas" },
  ]);
  expect(useSidebar.getState().network).toHaveLength(1);
});

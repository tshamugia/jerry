import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import type { TransferJob, TransferState, UpdateOffer, UpdateStage } from "../../ipc";
import { resetTransfers, useTransfers } from "../../stores/transfers";
import { syncUpdates, useUpdates } from "../../stores/updates";
import { UpdateNotice } from "./UpdateNotice";

const notes = "### Features\n\n- Copy and move ([abc1234](https://example.com/c))\n";

function offer(stage: UpdateStage = { kind: "offered" }, version = "0.2.0"): UpdateOffer {
  return { version, notes, stage };
}

function shown(stage?: UpdateStage) {
  useUpdates.setState({ offer: offer(stage), later: false });
  return render(<UpdateNotice />);
}

/** A fake core that records the commands it gets. */
function fakeCore(current: UpdateOffer | null = null): string[] {
  const calls: string[] = [];
  mockIPC(
    (cmd) => {
      calls.push(cmd);
      return cmd === "update_offer" ? current : null;
    },
    { shouldMockEvents: true },
  );
  return calls;
}

afterEach(() => {
  useUpdates.setState({ offer: null, later: false });
  resetTransfers();
});

/** A transfer in `state`, listed by the core. */
function transfer(id: number, state: TransferState): TransferJob {
  return {
    id,
    kind: "copy",
    state,
    source: String.raw`C:\Projects`,
    sources: 3,
    destination: String.raw`D:\Archive`,
    progress: {
      id,
      phase: "working",
      file: null,
      filesDone: 1,
      filesTotal: 3,
      bytesDone: 1,
      bytesTotal: 3,
      speed: 0,
      failed: 0,
    },
  };
}

test("no offer, no notice", () => {
  const { container } = render(<UpdateNotice />);
  expect(container).toBeEmptyDOMElement();
});

test("an offer shows the version, its changes and asks first", () => {
  const calls = fakeCore();
  shown();
  expect(screen.getByRole("status")).toHaveTextContent("Jerry 0.2.0 is available");
  expect(screen.getByText("Copy and move")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Update" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Later" })).toBeInTheDocument();
  expect(calls).toEqual([]);
});

test("Update asks the core to install", () => {
  const calls = fakeCore();
  shown();
  fireEvent.click(screen.getByRole("button", { name: "Update" }));
  expect(calls).toEqual(["update_install"]);
});

test("Later hides the notice until the next check offers it again", async () => {
  fakeCore();
  const stop = syncUpdates();
  await waitFor(() => expect(useUpdates.getState().offer).toBeNull());
  shown();
  fireEvent.click(screen.getByRole("button", { name: "Later" }));
  expect(screen.queryByRole("status")).toBeNull();
  await act(() => emit("update:changed", offer()));
  expect(screen.getByRole("status")).toHaveTextContent("Jerry 0.2.0 is available");
  stop();
});

test("the download shows its progress and can't be pressed again", () => {
  shown({ kind: "downloading", percent: 42 });
  expect(screen.getByRole("status")).toHaveTextContent("Updating to Jerry 0.2.0");
  expect(screen.getByText("Downloading… 42%")).toBeInTheDocument();
  expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "42");
  expect(screen.queryByRole("button")).toBeNull();
});

test("a download of unknown size has no percent", () => {
  shown({ kind: "downloading", percent: null });
  expect(screen.getByText("Downloading…")).toBeInTheDocument();
  expect(screen.getByRole("progressbar")).not.toHaveAttribute("aria-valuenow");
});

test("the install says Jerry will restart", () => {
  shown({ kind: "installing" });
  expect(screen.getByText("Installing. Jerry will restart.")).toBeInTheDocument();
  expect(screen.queryByRole("button")).toBeNull();
});

test("while transfers run, Update first says Jerry closes and stops them", () => {
  const calls = fakeCore();
  useTransfers.setState({
    list: {
      generation: 1,
      jobs: [transfer(1, { kind: "running" }), transfer(2, { kind: "paused" })],
    },
  });
  shown();
  fireEvent.click(screen.getByRole("button", { name: "Update" }));
  expect(calls).toEqual([]);
  const notice = screen.getByRole("status");
  expect(notice).toHaveTextContent("2 transfers are still running");
  expect(notice).toHaveTextContent("Jerry 0.2.0 is available");
  expect(notice).toHaveTextContent("you can resume them where they stopped");

  fireEvent.click(screen.getByRole("button", { name: "Not now" }));
  expect(screen.getByRole("button", { name: "Update" })).toBeInTheDocument();
  expect(calls).toEqual([]);

  fireEvent.click(screen.getByRole("button", { name: "Update" }));
  fireEvent.click(screen.getByRole("button", { name: "Update anyway" }));
  expect(calls).toEqual(["update_install"]);
});

test("jobs that ended, or an earlier Jerry left, don't hold Update up", () => {
  const calls = fakeCore();
  useTransfers.setState({
    list: {
      generation: 1,
      jobs: [transfer(1, { kind: "done" }), transfer(2, { kind: "unfinished" })],
    },
  });
  shown();
  fireEvent.click(screen.getByRole("button", { name: "Update" }));
  expect(calls).toEqual(["update_install"]);
});

test("a failure says so and offers Retry", () => {
  const calls = fakeCore();
  shown({ kind: "failed" });
  expect(screen.getByRole("status")).toHaveTextContent("Couldn't update Jerry");
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(calls).toEqual(["update_install"]);
  fireEvent.click(screen.getByRole("button", { name: "Later" }));
  expect(screen.queryByRole("status")).toBeNull();
});

test("a window that starts late gets the offer from the core", async () => {
  fakeCore(offer());
  const stop = syncUpdates();
  await waitFor(() => expect(useUpdates.getState().offer?.version).toBe("0.2.0"));
  stop();
});

test("an event is newer than the answer to the first question", async () => {
  fakeCore(offer({ kind: "offered" }, "0.2.0"));
  let stop = () => {};
  await act(async () => {
    stop = syncUpdates();
    await emit("update:changed", offer({ kind: "offered" }, "0.3.0"));
  });
  await waitFor(() => expect(useUpdates.getState().offer?.version).toBe("0.3.0"));
  stop();
});

test("the core can take an offer back", async () => {
  fakeCore(offer());
  const stop = syncUpdates();
  await waitFor(() => expect(useUpdates.getState().offer).not.toBeNull());
  await act(() => emit("update:changed", null));
  expect(useUpdates.getState().offer).toBeNull();
  stop();
});

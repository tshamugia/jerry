import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import { waitFor } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import { syncAppearance, useAppearance } from "./appearance";
import { windows11 } from "./settings.testing";

let stop: (() => void) | undefined;

afterEach(() => {
  stop?.();
  stop = undefined;
  useAppearance.setState({ appearance: undefined });
});

test("loads the appearance and follows appearance:changed", async () => {
  mockIPC((cmd) => (cmd === "appearance_get" ? windows11 : undefined), {
    shouldMockEvents: true,
  });
  stop = syncAppearance();
  await waitFor(() => expect(useAppearance.getState().appearance).toEqual(windows11));

  const teal = { mica: true, accent: { light: "#0f7b74", dark: "#87bdba" } };
  await emit("appearance:changed", teal);

  expect(useAppearance.getState().appearance).toEqual(teal);
});

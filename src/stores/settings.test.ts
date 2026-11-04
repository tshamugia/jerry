import { emit } from "@tauri-apps/api/event";
import { mockIPC } from "@tauri-apps/api/mocks";
import { waitFor } from "@testing-library/react";
import { afterEach, expect, test } from "vitest";
import type { Settings } from "../ipc";
import { setSetting, syncSettings, useSettings } from "./settings";
import { defaultSettings as defaults } from "./settings.testing";

let stop: (() => void) | undefined;

afterEach(() => {
  stop?.();
  stop = undefined;
  useSettings.setState({ settings: undefined });
});

test("loads the settings from the core", async () => {
  mockIPC((cmd) => (cmd === "settings_get" ? defaults : undefined), { shouldMockEvents: true });

  stop = syncSettings();

  await waitFor(() => expect(useSettings.getState().settings).toEqual(defaults));
});

test("follows settings:changed from any window", async () => {
  mockIPC((cmd) => (cmd === "settings_get" ? defaults : undefined), { shouldMockEvents: true });
  stop = syncSettings();
  await waitFor(() => expect(useSettings.getState().settings).toEqual(defaults));

  await emit("settings:changed", { ...defaults, theme: "dark" });

  expect(useSettings.getState().settings?.theme).toBe("dark");
});

test("an event that beats the first answer wins", async () => {
  let answer: ((settings: Settings) => void) | undefined;
  mockIPC(
    (cmd) =>
      cmd === "settings_get"
        ? new Promise<Settings>((resolve) => {
            answer = resolve;
          })
        : undefined,
    { shouldMockEvents: true },
  );
  stop = syncSettings();
  await waitFor(() => expect(answer).toBeDefined());

  await emit("settings:changed", { ...defaults, showHidden: true });
  answer?.(defaults);
  await new Promise((resolve) => setTimeout(resolve, 0));

  expect(useSettings.getState().settings?.showHidden).toBe(true);
});

test("setSetting sends the key and the value to the core", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
  });

  const result = await setSetting("startup", { kind: "folder", path: String.raw`D:\პროექტები` });

  expect(result).toEqual({ status: "ok", data: undefined });
  expect(calls).toEqual([
    {
      cmd: "settings_set",
      args: { key: "startup", value: { kind: "folder", path: String.raw`D:\პროექტები` } },
    },
  ]);
});

import { useState } from "react";
import {
  commands,
  HEADER_BYTES,
  type JerryError,
  messageHeader,
  type Startup,
  stream,
  type Theme,
} from "../../ipc";
import { setSetting, useSettings } from "../../stores/settings";
import { activeTab } from "../../stores/tabs";
import { useWindow } from "../../stores/window";
import { errorText, strings } from "../../strings";
import { DevButton } from "./DevButton";
import { DevListing } from "./DevListing";

const FREEZE_MS = 200;
const STREAM_MESSAGES = 3;
const STREAM_TIMEOUT_MS = 2000;
const THEMES: readonly Theme[] = ["system", "light", "dark"];

/**
 * Dev builds only, in the preview zone: buttons that exercise the IPC, error and settings
 * foundation in the running app, and the listing checks for the file lists. Each shows what came
 * back; freezes and panics also land in the log. The settings line follows the store, so a change
 * made in another window shows here at once.
 */
export function DevPanel() {
  const [result, setResult] = useState<string>();
  const settings = useSettings((state) => state.settings);
  const run = (check: () => Promise<string | undefined>) => () => {
    void check().then(setResult);
  };

  return (
    <section aria-label={strings.dev.title} className="flex flex-col items-center gap-3 text-small">
      <div className="flex flex-wrap justify-center gap-2">
        <DevButton onClick={run(sampleError)}>{strings.dev.sampleError}</DevButton>
        <DevButton onClick={run(panicInJob)}>{strings.dev.panicInJob}</DevButton>
        <DevButton onClick={run(freezeMainThread)}>{strings.dev.blockMainThread}</DevButton>
        <DevButton onClick={run(freezePage)}>{strings.dev.blockPage}</DevButton>
        <DevButton onClick={run(binaryStream)}>{strings.dev.stream}</DevButton>
      </div>
      <div className="flex flex-wrap justify-center gap-2">
        <DevButton
          onClick={run(() => saved(setSetting("showHidden", !(settings?.showHidden ?? false))))}
        >
          {strings.dev.toggleHidden}
        </DevButton>
        <DevButton
          onClick={run(() =>
            saved(setSetting("showExtensions", !(settings?.showExtensions ?? true))),
          )}
        >
          {strings.dev.toggleExtensions}
        </DevButton>
        <DevButton
          onClick={run(() =>
            saved(
              setSetting("clickToOpen", settings?.clickToOpen === "single" ? "double" : "single"),
            ),
          )}
        >
          {strings.dev.toggleClickToOpen}
        </DevButton>
        <DevButton onClick={run(() => saved(setSetting("theme", nextTheme(settings?.theme))))}>
          {strings.dev.nextTheme}
        </DevButton>
        <DevButton onClick={run(() => saved(commands.devOpenWindow()))}>
          {strings.dev.openWindow}
        </DevButton>
        <DevButton
          onClick={run(() => saved(setSetting("dualPane", !(settings?.dualPane ?? true))))}
        >
          {strings.dev.toggleDualPane}
        </DevButton>
        <DevButton onClick={run(() => saved(toggleStartFolder(settings?.startup)))}>
          {strings.dev.toggleStartFolder}
        </DevButton>
        <DevButton
          onClick={run(() =>
            saved(setSetting("rememberFolderViews", !(settings?.rememberFolderViews ?? true))),
          )}
        >
          {strings.dev.toggleFolderViews}
        </DevButton>
        <DevButton
          onClick={run(() => saved(setSetting("keepRunning", !(settings?.keepRunning ?? true))))}
        >
          {strings.dev.toggleKeepRunning}
        </DevButton>
      </div>
      <p className="text-small text-fg-2">
        {settings ? strings.dev.settings(settings) : strings.dev.settingsLoading}
      </p>
      {settings?.startup && (
        <p className="text-small text-fg-2">
          {strings.dev.sessionSettings(
            settings.startup.kind === "folder" ? settings.startup.path : undefined,
            settings.rememberFolderViews,
            settings.keepRunning,
          )}
        </p>
      )}
      {result && (
        <p role="status" className="text-small text-fg-2">
          {result}
        </p>
      )}
      <DevListing />
    </section>
  );
}

/**
 * The start place: the active pane's folder (as its path reads), or the previous session again.
 * The Settings window (#82) will choose it properly.
 */
function toggleStartFolder(startup: Startup | undefined) {
  const path = activeTab(useWindow.getState().activePane)?.current()?.path;
  return setSetting(
    "startup",
    startup?.kind === "folder" || path === undefined
      ? { kind: "previousSession" }
      : { kind: "folder", path },
  );
}

function nextTheme(theme: Theme | undefined): Theme {
  const index = THEMES.indexOf(theme ?? "system");
  return THEMES[(index + 1) % THEMES.length] ?? "system";
}

/** The error text if the command failed; nothing otherwise (the settings line shows the rest). */
async function saved(
  command: Promise<{ status: "ok" } | { status: "error"; error: JerryError }>,
): Promise<string | undefined> {
  const result = await command;
  return result.status === "error" ? errorText(result.error) : undefined;
}

async function sampleError(): Promise<string> {
  const result = await commands.devSampleError();
  return result.status === "error" ? errorText(result.error) : strings.dev.noError;
}

async function panicInJob(): Promise<string> {
  const result = await commands.devPanicInJob();
  return result.status === "error" ? errorText(result.error) : strings.dev.noError;
}

async function freezeMainThread(): Promise<string> {
  const result = await commands.devBlockMainThread(FREEZE_MS);
  return result.status === "error"
    ? errorText(result.error)
    : strings.dev.frozeMainThread(FREEZE_MS);
}

async function freezePage(): Promise<string> {
  const until = performance.now() + FREEZE_MS;
  while (performance.now() < until) {
    // Busy on purpose: a long task for the long-task observer.
  }
  return strings.dev.frozePage(FREEZE_MS);
}

async function binaryStream(): Promise<string> {
  const values: number[] = [];
  let received = 0;
  let allArrived: () => void = () => {};
  const arrived = new Promise<void>((resolve) => {
    allArrived = resolve;
  });
  const channel = stream<ArrayBuffer>((message) => {
    const [, index = 0] = messageHeader(message);
    values[index] = new Float64Array(message, HEADER_BYTES, 1)[0] ?? Number.NaN;
    received += 1;
    if (received === STREAM_MESSAGES) {
      allArrived();
    }
  });
  const result = await commands.devStream(channel);
  if (result.status === "error") {
    return errorText(result.error);
  }
  const timeout = new Promise<void>((resolve) => setTimeout(resolve, STREAM_TIMEOUT_MS));
  await Promise.race([arrived, timeout]);
  return received === STREAM_MESSAGES
    ? strings.dev.streamed(received, values.join(", "))
    : strings.dev.streamIncomplete(received);
}

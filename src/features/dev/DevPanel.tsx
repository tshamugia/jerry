import { useState } from "react";
import {
  commands,
  HEADER_BYTES,
  type JerryError,
  messageHeader,
  stream,
  type Theme,
} from "../../ipc";
import { setSetting, useSettings } from "../../stores/settings";
import { errorText, strings } from "../../strings";
import { DevButton } from "./DevButton";
import { DevListing } from "./DevListing";

const FREEZE_MS = 200;
const STREAM_MESSAGES = 3;
const STREAM_TIMEOUT_MS = 2000;
const THEMES: readonly Theme[] = ["system", "light", "dark"];

/**
 * Dev builds only: buttons that exercise the IPC, error and settings foundation in the running
 * app. Each shows what came back; freezes and panics also land in the log. The settings line
 * follows the store, so a change made in another window shows here at once.
 */
export function DevPanel() {
  const [result, setResult] = useState<string>();
  const settings = useSettings((state) => state.settings);
  const run = (check: () => Promise<string | undefined>) => () => {
    void check().then(setResult);
  };

  return (
    <section aria-label={strings.dev.title} className="flex flex-col items-center gap-3">
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
        <DevButton onClick={run(() => saved(setSetting("theme", nextTheme(settings?.theme))))}>
          {strings.dev.nextTheme}
        </DevButton>
        <DevButton onClick={run(() => saved(commands.devOpenWindow()))}>
          {strings.dev.openWindow}
        </DevButton>
      </div>
      <p className="text-small text-fg-2">
        {settings
          ? strings.dev.settings(settings.showHidden, settings.theme)
          : strings.dev.settingsLoading}
      </p>
      {result && (
        <p role="status" className="text-small text-fg-2">
          {result}
        </p>
      )}
      <DevListing />
    </section>
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

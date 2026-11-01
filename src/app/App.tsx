import { useEffect, useLayoutEffect, useState } from "react";
import { FileList } from "../features/file-list/FileList";
import { useListing } from "../features/file-list/store";
import { firstRowPaint, sendReport, stamp, stampNextFrame } from "../features/startup/stamps";
import { commands } from "../ipc";
import { strings } from "../strings";

/** The window frame. Spike #11: one Details list of the startup folder. */
export function App() {
  const [version, setVersion] = useState<string>();

  useLayoutEffect(() => stamp("react:commit"), []);

  useEffect(() => {
    // The window is created hidden; show it once the first render is committed (no white flash).
    stamp("app_ready:sent");
    void commands.appReady();
    void commands.appVersion().then(setVersion);
    void firstRowPaint.then(() => runSpikeReport());
  }, []);

  return (
    <div className="flex h-full flex-col bg-mica text-fg">
      <header className="flex h-titlebar shrink-0 items-center gap-3 px-4 text-small text-fg-2">
        {strings.appName}
        {version && <span className="text-fg-3 tabular-nums">{strings.version(version)}</span>}
      </header>
      <FileList />
    </div>
  );
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/** Spike #11: sends the startup report; in the memory test, opens the other tabs first. */
async function runSpikeReport(): Promise<void> {
  await sendReport("startup");
  if (await commands.startupWaitShown()) {
    // Resident mode: the window was hidden until now; time its first frame after show.
    stamp("resident:shown");
    stampNextFrame("resident");
    await sleep(500);
    await sendReport("resident");
    return;
  }
  const tabs = await commands.startupTabs();
  if (tabs.length === 0) {
    return;
  }
  await sleep(2000);
  await sendReport("one-tab");
  await Promise.all(tabs.map((count) => useListing.getState().openBackground(count)));
  await sleep(3000);
  await sendReport("tabs");
}

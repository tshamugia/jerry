import { useEffect, useState } from "react";
import { FileList } from "../features/file-list/FileList";
import { SpikeBar } from "../features/file-list/SpikeBar";
import { installSpikeApi } from "../features/file-list/spike";
import { commands } from "../ipc";
import { strings } from "../strings";

/** The window frame. Spike #12: a single virtualized Details list over synthetic folders. */
export function App() {
  const [version, setVersion] = useState<string>();

  useEffect(() => {
    installSpikeApi();
    // The window is created hidden; show it once the first render is committed (no white flash).
    void commands.appReady();
    void commands.appVersion().then(setVersion);
  }, []);

  return (
    <div className="flex h-full flex-col bg-mica text-fg">
      <header className="flex h-titlebar shrink-0 items-center gap-3 px-4 text-small text-fg-2">
        {strings.appName}
        {version && <span className="text-fg-3 tabular-nums">{strings.version(version)}</span>}
      </header>
      <SpikeBar />
      <FileList />
    </div>
  );
}

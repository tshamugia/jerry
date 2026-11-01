import { useEffect, useState } from "react";
import { commands } from "../ipc";
import { strings } from "../strings";

/** The window frame. For now an empty Jerry window; zones arrive with #22 and #23. */
export function App() {
  const [version, setVersion] = useState<string>();

  useEffect(() => {
    // The window is created hidden; show it once the first render is committed (no white flash).
    void commands.appReady();
    void commands.appVersion().then(setVersion);
  }, []);

  return (
    <div className="flex h-full flex-col bg-mica text-fg">
      <header className="flex h-titlebar shrink-0 items-center px-4 text-small text-fg-2">
        {strings.appName}
      </header>
      <main className="flex flex-1 items-center justify-center bg-pane">
        {version && <p className="text-fg-3 tabular-nums">{strings.version(version)}</p>}
      </main>
    </div>
  );
}

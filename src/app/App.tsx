import { useEffect, useState } from "react";
import { MenuLab } from "../features/spike-menu/MenuLab";
import { commands } from "../ipc";
import { strings } from "../strings";

/** The window frame. For spike #16 it hosts the Windows-menu lab. */
export function App() {
  const [version, setVersion] = useState<string>();

  useEffect(() => {
    // The window is created hidden; show it once the first render is committed (no white flash).
    void commands.appReady();
    void commands.appVersion().then(setVersion);
  }, []);

  return (
    <div className="flex h-full flex-col bg-mica text-fg">
      <header className="flex h-titlebar shrink-0 items-center justify-between px-4 text-small text-fg-2">
        <span>{strings.appName}</span>
        {version && <span className="text-fg-3 tabular-nums">{strings.version(version)}</span>}
      </header>
      <main className="flex flex-1 flex-col bg-pane">
        <MenuLab />
      </main>
    </div>
  );
}

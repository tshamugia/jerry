import { useEffect, useState } from "react";
import { DropLab } from "../features/spike-drop/DropLab";
import { commands } from "../ipc";
import { strings } from "../strings";

/** The window frame. Spikes #13 and #14 fill it with the drag & drop lab. */
export function App() {
  const [version, setVersion] = useState<string>();

  useEffect(() => {
    // The window is created hidden; show it once the first render is committed (no white flash).
    void commands.appReady();
    void commands.appVersion().then(setVersion);
  }, []);

  return (
    <div className="flex h-full flex-col bg-mica text-fg">
      {/* Spike only: a bare title bar to move and close the window. The real one is #17. */}
      <header
        className="flex h-titlebar shrink-0 items-center gap-3 pl-4 text-small text-fg-2"
        onPointerDown={(event) => {
          if (event.button === 0 && event.target === event.currentTarget) {
            void commands.spikeWindow("move");
          }
        }}
      >
        {strings.appName}
        {version && <span className="text-fg-3 tabular-nums">{strings.version(version)}</span>}
        <button
          type="button"
          aria-label={strings.close}
          className="ml-auto h-full w-caption-btn hover:bg-red hover:text-on-accent"
          onClick={() => void commands.spikeWindow("close")}
        >
          ✕
        </button>
      </header>
      <main className="min-h-0 flex-1 bg-pane">
        <DropLab />
      </main>
    </div>
  );
}

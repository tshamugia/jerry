import { useEffect } from "react";
import { ClipboardSpike } from "../features/spike-clipboard/ClipboardSpike";
import { commands } from "../ipc";

/** The window frame. On the spike #15 branch it shows the clipboard experiment. */
export function App() {
  useEffect(() => {
    // The window is created hidden; show it once the first render is committed (no white flash).
    void commands.appReady();
  }, []);

  return <ClipboardSpike />;
}

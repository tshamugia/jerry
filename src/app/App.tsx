import { useEffect, useMemo } from "react";
import { useCommandKeys } from "../commands/dispatch";
import { ShortcutTable } from "../commands/ShortcutTable";
import { ClipboardDialog } from "../features/clipboard/ClipboardDialog";
import { DeleteDialog } from "../features/delete/DeleteDialog";
import { DevPanel } from "../features/dev/DevPanel";
import { PaneList } from "../features/file-list/PaneList";
import { OpenManyDialog } from "../features/navigation/OpenManyDialog";
import { RenameDialogs } from "../features/rename/RenameDialogs";
import { SelectByPattern } from "../features/selection/SelectByPattern";
import { commands } from "../ipc";
import { useAppearance } from "../stores/appearance";
import { useSettings } from "../stores/settings";
import { Frame } from "./Frame";

/**
 * The window. It renders once the settings and the Windows appearance are known, so the first
 * frame already has the right theme, density, accent and zone sizes; then the core shows the
 * window, which was created hidden (no flash).
 */
export function App() {
  const settings = useSettings((state) => state.settings);
  const appearance = useAppearance((state) => state.appearance);
  const ready = settings !== undefined && appearance !== undefined;
  const dev = useMemo(() => (import.meta.env.DEV ? <DevPanel /> : undefined), []);
  const left = useMemo(() => <PaneList pane="left" />, []);
  const right = useMemo(() => <PaneList pane="right" />, []);

  useEffect(() => {
    if (ready) {
      void commands.appReady();
    }
  }, [ready]);
  useCommandKeys();

  if (!settings || !appearance) {
    return null;
  }
  return (
    <>
      <Frame settings={settings} left={left} right={right} previewContent={dev} />
      <ShortcutTable />
      <SelectByPattern />
      <OpenManyDialog />
      <DeleteDialog />
      <RenameDialogs />
      <ClipboardDialog />
    </>
  );
}

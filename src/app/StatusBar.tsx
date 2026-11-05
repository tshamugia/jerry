import { memo } from "react";
import { toggleDualPane } from "../features/navigation/panes";
import type { Density } from "../ipc";
import { useCommandLabel } from "../stores/keymap";
import { setSetting } from "../stores/settings";
import { strings } from "../strings";
import { IconButton } from "../ui/IconButton";
import { ComfortableIcon, CompactIcon, DualPaneIcon, SinglePaneIcon } from "../ui/icons";

/**
 * The 28 px status bar on Mica. The item and selection counts and the drive's free space arrive
 * with #41; the switches are here already: one pane or two (the same as F3) and the row density
 * (the same as Ctrl+Alt+D).
 */
export const StatusBar = memo(function StatusBar({
  density,
  dual,
}: {
  density: Density;
  dual: boolean;
}) {
  const densityLabel = useCommandLabel("toggleDensity");
  const panesLabel = useCommandLabel("dualPane");
  return (
    <footer className="flex h-statusbar shrink-0 items-center gap-4 border-stroke border-t pr-2 pl-4 text-fg-2 text-small">
      <span className="flex-1" />
      <fieldset aria-label={panesLabel} className="flex gap-0.5">
        <IconButton
          label={strings.status.singlePane}
          pressed={!dual}
          onClick={() => dual && toggleDualPane()}
          className="h-5.5 w-6 rounded-kbd"
        >
          <SinglePaneIcon />
        </IconButton>
        <IconButton
          label={strings.status.dualPane}
          pressed={dual}
          onClick={() => !dual && toggleDualPane()}
          className="h-5.5 w-6 rounded-kbd"
        >
          <DualPaneIcon />
        </IconButton>
      </fieldset>
      <fieldset aria-label={densityLabel} className="flex gap-0.5">
        <IconButton
          label={strings.status.comfortable}
          pressed={density === "comfortable"}
          onClick={() => void setSetting("density", "comfortable")}
          className="h-5.5 w-6 rounded-kbd"
        >
          <ComfortableIcon />
        </IconButton>
        <IconButton
          label={strings.status.compact}
          pressed={density === "compact"}
          onClick={() => void setSetting("density", "compact")}
          className="h-5.5 w-6 rounded-kbd"
        >
          <CompactIcon />
        </IconButton>
      </fieldset>
    </footer>
  );
});

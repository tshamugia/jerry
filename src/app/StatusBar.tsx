import { memo } from "react";
import type { Density } from "../ipc";
import { setSetting } from "../stores/settings";
import { strings } from "../strings";
import { IconButton } from "../ui/IconButton";
import { ComfortableIcon, CompactIcon } from "../ui/icons";

/**
 * The 28 px status bar on Mica. The item and selection counts and the drive's free space arrive
 * with the file list; the row density switch is here already (the same as Ctrl+Alt+D).
 */
export const StatusBar = memo(function StatusBar({ density }: { density: Density }) {
  return (
    <footer className="flex h-statusbar shrink-0 items-center gap-4 border-stroke border-t pr-2 pl-4 text-fg-2 text-small">
      <span className="flex-1" />
      <fieldset aria-label={strings.status.density} className="flex gap-0.5">
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

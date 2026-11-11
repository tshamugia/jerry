import { memo, useMemo } from "react";
import { useShownRows } from "../features/file-list/rows";
import { toggleDualPane } from "../features/navigation/panes";
import { TransferIndicator } from "../features/transfers/TransferIndicator";
import type { Density } from "../ipc";
import { type Folder, useFolder } from "../stores/folder";
import { useCommandLabel } from "../stores/keymap";
import { setSetting } from "../stores/settings";
import { useActiveTab } from "../stores/tabs";
import { useWindow } from "../stores/window";
import { sizeText, strings } from "../strings";
import { IconButton } from "../ui/IconButton";
import { ComfortableIcon, CompactIcon, DualPaneIcon, SinglePaneIcon } from "../ui/icons";
import { selectionSize, useFolderSpace } from "./status";

const t = strings.status;

/**
 * The 28 px status bar on Mica, for the active pane's folder: its item count ("12 of 340 items"
 * while the quick filter is on), the selection's count and size, and the free space on its
 * drive; then the switches: one pane or two (the same as F3) and the row density (the same as
 * Ctrl+Alt+D). Each part follows its own slice of the folder, so a selection change repaints
 * only the selection.
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
  const pane = useWindow((state) => state.activePane);
  const folder = useActiveTab(pane)?.folder;
  return (
    <footer className="flex h-statusbar shrink-0 items-center gap-4 border-stroke border-t pr-2 pl-4 text-fg-2 text-small tabular-nums">
      {folder && <Items folder={folder} />}
      {folder && <Selected folder={folder} />}
      <span className="flex-1" />
      <TransferIndicator />
      {folder && <FreeSpace folder={folder} />}
      <fieldset aria-label={panesLabel} className="flex shrink-0 gap-0.5">
        <IconButton
          label={t.singlePane}
          pressed={!dual}
          onClick={() => dual && toggleDualPane()}
          className="h-5.5 w-6 rounded-kbd"
        >
          <SinglePaneIcon />
        </IconButton>
        <IconButton
          label={t.dualPane}
          pressed={dual}
          onClick={() => !dual && toggleDualPane()}
          className="h-5.5 w-6 rounded-kbd"
        >
          <DualPaneIcon />
        </IconButton>
      </fieldset>
      <fieldset aria-label={densityLabel} className="flex shrink-0 gap-0.5">
        <IconButton
          label={t.comfortable}
          pressed={density === "comfortable"}
          onClick={() => void setSetting("density", "comfortable")}
          className="h-5.5 w-6 rounded-kbd"
        >
          <ComfortableIcon />
        </IconButton>
        <IconButton
          label={t.compact}
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

/** The 1 × 12 px rule between the status bar's parts. */
function Divider() {
  return <span aria-hidden="true" className="h-3 w-px shrink-0 bg-stroke" />;
}

/** "340 items", or "12 of 340 items" while the quick filter is on: what the list shows. */
const Items = memo(function Items({ folder }: { folder: Folder }) {
  const status = useFolder(folder, (state) => state.status);
  const filtered = useFolder(folder, (state) => state.filter !== "");
  const { rows, total } = useShownRows(folder);
  if (status === "none" || status === "failed") {
    return null;
  }
  return <span className="truncate">{t.items(rows.length, total, filtered)}</span>;
});

/** "3 selected · 7.5 MB", once something is selected. */
const Selected = memo(function Selected({ folder }: { folder: Folder }) {
  const selected = useFolder(folder, (state) => state.selected);
  const entries = useFolder(folder, (state) => state.entries);
  const version = useFolder(folder, (state) => state.version);
  // `entries` changes in place (a diff can change a selected file's size): `version` says when.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `version` is an input of `selectionSize`.
  const size = useMemo(() => selectionSize(entries, selected), [entries, selected, version]);
  if (size.count === 0) {
    return null;
  }
  return (
    <>
      <Divider />
      <span
        className="truncate text-fg"
        title={size.files > 0 && size.folders > 0 ? t.filesOnly : undefined}
      >
        {t.selected(size.count, size.files > 0 ? sizeText(size.bytes) : undefined)}
      </span>
    </>
  );
});

/** "D: 612 GB free of 1.8 TB", once the drive's space is known. */
const FreeSpace = memo(function FreeSpace({ folder }: { folder: Folder }) {
  const found = useFolderSpace(folder);
  if (!found) {
    return null;
  }
  const { volume, space } = found;
  return (
    <>
      <span className="truncate" title={volume.root}>
        {t.free(volume.name, sizeText(space.free), sizeText(space.total))}
      </span>
      <Divider />
    </>
  );
});

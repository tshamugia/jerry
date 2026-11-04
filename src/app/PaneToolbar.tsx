import { memo } from "react";
import { FilterField } from "../features/filter/FilterField";
import type { Folder } from "../stores/folder";
import { useCommandLabel } from "../stores/keymap";
import type { PaneSide } from "../stores/window";
import { strings } from "../strings";
import { IconButton } from "../ui/IconButton";
import {
  BackIcon,
  DetailsIcon,
  ForwardIcon,
  GridIcon,
  ListIcon,
  PaletteIcon,
  RefreshIcon,
  UpIcon,
} from "../ui/icons";

const t = strings.toolbar;

/**
 * A pane's toolbar, laid out as the design: Back, Forward, Up, the address bar and the command
 * palette button; under them the quick filter of `folder` and the view switcher. Controls stay
 * disabled until their features arrive (navigation, address bar, palette, List and Grid views).
 * Its buttons are commands, so their tooltips show the shortcuts in force.
 */
export const PaneToolbar = memo(function PaneToolbar({
  folder,
  pane,
}: {
  folder: Folder;
  pane: PaneSide;
}) {
  const labels = {
    back: useCommandLabel("back"),
    forward: useCommandLabel("forward"),
    up: useCommandLabel("up"),
    refresh: useCommandLabel("refresh"),
    palette: useCommandLabel("palette"),
    detailsView: useCommandLabel("detailsView"),
    listView: useCommandLabel("listView"),
    gridView: useCommandLabel("gridView"),
  };
  return (
    <div className="flex shrink-0 flex-col gap-1.5 bg-bar px-2 pt-1.5 pb-2">
      <div className="flex items-center gap-0.5">
        <IconButton label={labels.back} disabled className="size-toolbar-btn rounded-ctrl">
          <BackIcon />
        </IconButton>
        <IconButton label={labels.forward} disabled className="size-toolbar-btn rounded-ctrl">
          <ForwardIcon />
        </IconButton>
        <IconButton label={labels.up} disabled className="size-toolbar-btn rounded-ctrl">
          <UpIcon />
        </IconButton>
        <div className="ml-1 flex h-address min-w-0 flex-1 items-center rounded-ctrl border border-ctrl-border bg-ctrl pr-1 pl-2">
          <span className="flex-1" />
          <IconButton label={labels.refresh} disabled className="size-6 rounded-kbd text-fg-2">
            <RefreshIcon />
          </IconButton>
        </div>
        <IconButton label={labels.palette} disabled className="ml-1 size-toolbar-btn rounded-ctrl">
          <PaletteIcon />
        </IconButton>
      </div>
      <div className="flex items-center gap-2">
        <FilterField folder={folder} pane={pane} />
        <fieldset
          aria-label={t.view}
          className="flex gap-0.5 rounded-ctrl border border-ctrl-border bg-ctrl p-0.5"
        >
          <IconButton label={labels.detailsView} pressed className="h-5.5 w-6.5 rounded-kbd">
            <DetailsIcon />
          </IconButton>
          <IconButton label={labels.listView} disabled className="h-5.5 w-6.5 rounded-kbd">
            <ListIcon />
          </IconButton>
          <IconButton label={labels.gridView} disabled className="h-5.5 w-6.5 rounded-kbd">
            <GridIcon />
          </IconButton>
        </fieldset>
      </div>
    </div>
  );
});

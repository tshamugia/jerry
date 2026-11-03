import { memo } from "react";
import { strings } from "../strings";
import { IconButton } from "../ui/IconButton";
import {
  BackIcon,
  DetailsIcon,
  FilterIcon,
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
 * palette button; under them the quick filter and the view switcher. A skeleton for now: its
 * controls stay disabled until their features arrive (navigation, address bar, filter, palette,
 * List and Grid views).
 */
export const PaneToolbar = memo(function PaneToolbar() {
  return (
    <div className="flex shrink-0 flex-col gap-1.5 bg-bar px-2 pt-1.5 pb-2">
      <div className="flex items-center gap-0.5">
        <IconButton label={t.back} disabled className="size-toolbar-btn rounded-ctrl">
          <BackIcon />
        </IconButton>
        <IconButton label={t.forward} disabled className="size-toolbar-btn rounded-ctrl">
          <ForwardIcon />
        </IconButton>
        <IconButton label={t.up} disabled className="size-toolbar-btn rounded-ctrl">
          <UpIcon />
        </IconButton>
        <div className="ml-1 flex h-address min-w-0 flex-1 items-center rounded-ctrl border border-ctrl-border bg-ctrl pr-1 pl-2">
          <span className="flex-1" />
          <IconButton label={t.refresh} disabled className="size-6 rounded-kbd text-fg-2">
            <RefreshIcon />
          </IconButton>
        </div>
        <IconButton label={t.palette} disabled className="ml-1 size-toolbar-btn rounded-ctrl">
          <PaletteIcon />
        </IconButton>
      </div>
      <div className="flex items-center gap-2">
        <label className="flex h-filter min-w-0 flex-1 items-center gap-2 rounded-ctrl border border-ctrl-border bg-ctrl px-2 text-fg-2">
          <FilterIcon />
          <input
            type="text"
            disabled
            aria-label={t.filter}
            placeholder={t.filterPlaceholder}
            className="min-w-0 flex-1 bg-transparent text-fg text-small outline-none placeholder:text-fg-3"
          />
          <kbd className="rounded-kbd border border-ctrl-border px-1.25 font-sans text-caption text-fg-3">
            {t.filterKey}
          </kbd>
        </label>
        <fieldset
          aria-label={t.view}
          className="flex gap-0.5 rounded-ctrl border border-ctrl-border bg-ctrl p-0.5"
        >
          <IconButton label={t.details} pressed className="h-5.5 w-6.5 rounded-kbd">
            <DetailsIcon />
          </IconButton>
          <IconButton label={t.list} disabled className="h-5.5 w-6.5 rounded-kbd">
            <ListIcon />
          </IconButton>
          <IconButton label={t.grid} disabled className="h-5.5 w-6.5 rounded-kbd">
            <GridIcon />
          </IconButton>
        </fieldset>
      </div>
    </div>
  );
});

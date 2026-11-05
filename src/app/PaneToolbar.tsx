import { memo } from "react";
import { useStore } from "zustand";
import { FilterField } from "../features/filter/FilterField";
import { AddressBar } from "../features/navigation/AddressBar";
import { useCommandLabel } from "../stores/keymap";
import { type Tab, useActiveTab } from "../stores/tabs";
import type { PaneSide } from "../stores/window";
import { strings } from "../strings";
import { ContextMenu, MenuItem } from "../ui/ContextMenu";
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

/** Folders a Back or Forward history menu lists, nearest first. */
const HISTORY_MENU = 15;

const NAV_BUTTON = "size-toolbar-btn rounded-ctrl";

/**
 * A pane's toolbar, laid out as the design: Back, Forward, Up, the address bar and the command
 * palette button; under them the quick filter and the view switcher. It acts on the tab the pane
 * shows; right-clicking Back or Forward lists the tab's folders that way. The address bar shows
 * the folder as segments, a text field on a click or Ctrl+L (`AddressBar`). Controls stay disabled until their
 * features arrive (palette, List and Grid views). Its buttons are commands, so their tooltips
 * show the shortcuts in force.
 */
export const PaneToolbar = memo(function PaneToolbar({ pane }: { pane: PaneSide }) {
  const tab = useActiveTab(pane);
  const labels = {
    palette: useCommandLabel("palette"),
    detailsView: useCommandLabel("detailsView"),
    listView: useCommandLabel("listView"),
    gridView: useCommandLabel("gridView"),
  };
  return (
    <div className="flex shrink-0 flex-col gap-1.5 bg-bar px-2 pt-1.5 pb-2">
      <div className="flex items-center gap-0.5">
        {tab ? <Navigation tab={tab} pane={pane} /> : <NoNavigation />}
        <IconButton label={labels.palette} disabled className="ml-1 size-toolbar-btn rounded-ctrl">
          <PaletteIcon />
        </IconButton>
      </div>
      <div className="flex items-center gap-2">
        {tab && <FilterField folder={tab.folder} pane={pane} />}
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

/** Back, Forward, Up and the address bar with Refresh, for `tab`. */
function Navigation({ tab, pane }: { tab: Tab; pane: PaneSide }) {
  const history = useStore(tab.store, (state) => state.history);
  const index = useStore(tab.store, (state) => state.index);
  const labels = {
    back: useCommandLabel("back"),
    forward: useCommandLabel("forward"),
    up: useCommandLabel("up"),
    refresh: useCommandLabel("refresh"),
  };
  const current = history[index];
  const behind = history.slice(Math.max(0, index - HISTORY_MENU), Math.max(0, index));
  const ahead = history.slice(index + 1, index + 1 + HISTORY_MENU);
  const historyMenu = (from: number, entries: typeof history, nearestFirst: boolean) =>
    (nearestFirst ? [...entries.keys()].reverse() : [...entries.keys()]).map((i) => (
      <MenuItem key={from + i} onSelect={() => tab.go(from + i)}>
        <span className="truncate">{entries[i]?.name}</span>
      </MenuItem>
    ));

  return (
    <>
      <ContextMenu label={t.backHistory} menu={historyMenu(index - behind.length, behind, true)}>
        <IconButton
          label={labels.back}
          disabled={index <= 0}
          onClick={() => tab.back()}
          className={NAV_BUTTON}
        >
          <BackIcon />
        </IconButton>
      </ContextMenu>
      <ContextMenu label={t.forwardHistory} menu={historyMenu(index + 1, ahead, false)}>
        <IconButton
          label={labels.forward}
          disabled={index >= history.length - 1}
          onClick={() => tab.forward()}
          className={NAV_BUTTON}
        >
          <ForwardIcon />
        </IconButton>
      </ContextMenu>
      <IconButton
        label={labels.up}
        disabled={current?.parent == null}
        onClick={() => tab.up()}
        className={NAV_BUTTON}
      >
        <UpIcon />
      </IconButton>
      <fieldset
        aria-label={t.address}
        className="ml-1 flex h-address min-w-0 flex-1 items-center gap-1 rounded-ctrl border border-ctrl-border bg-ctrl pr-1 pl-1 focus-within:outline-2 focus-within:outline-focus focus-within:-outline-offset-2"
      >
        <AddressBar key={tab.id} tab={tab} pane={pane} />
        <IconButton
          label={labels.refresh}
          onClick={() => tab.refresh()}
          className="size-6 rounded-kbd text-fg-2"
        >
          <RefreshIcon />
        </IconButton>
      </fieldset>
    </>
  );
}

/** The navigation controls before the pane has a tab. */
function NoNavigation() {
  const labels = {
    back: useCommandLabel("back"),
    forward: useCommandLabel("forward"),
    up: useCommandLabel("up"),
    refresh: useCommandLabel("refresh"),
  };
  return (
    <>
      <IconButton label={labels.back} disabled className={NAV_BUTTON}>
        <BackIcon />
      </IconButton>
      <IconButton label={labels.forward} disabled className={NAV_BUTTON}>
        <ForwardIcon />
      </IconButton>
      <IconButton label={labels.up} disabled className={NAV_BUTTON}>
        <UpIcon />
      </IconButton>
      <div className="ml-1 flex h-address min-w-0 flex-1 items-center rounded-ctrl border border-ctrl-border bg-ctrl pr-1 pl-2">
        <span className="flex-1" />
        <IconButton label={labels.refresh} disabled className="size-6 rounded-kbd text-fg-2">
          <RefreshIcon />
        </IconButton>
      </div>
    </>
  );
}

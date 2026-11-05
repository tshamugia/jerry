import {
  type CSSProperties,
  lazy,
  type MouseEvent,
  memo,
  type PointerEvent,
  type ReactNode,
  type Ref,
  Suspense,
} from "react";
import { useStore } from "zustand";
import { commands } from "../../ipc";
import { useCommandLabel } from "../../stores/keymap";
import { useSetting } from "../../stores/settings";
import {
  activateTab,
  closeOtherTabs,
  closeTab,
  closeTabsToRight,
  duplicateTab,
  type PaneTab,
  pinTab,
  type Tab,
  tabById,
  useTabs,
} from "../../stores/tabs";
import { activatePane, type PaneSide, useWindow } from "../../stores/window";
import { strings } from "../../strings";
import { ContextMenu, MenuItem, MenuSeparator } from "../../ui/ContextMenu";
import { IconButton } from "../../ui/IconButton";
import { CloseIcon, FolderIcon, PinIcon, PlusIcon } from "../../ui/icons";
import { focusListSoon } from "../filter/focus";
import { openInOtherPane } from "../navigation/panes";
import { newTab } from "./commands";

const t = strings.titleBar;

/** dnd-kit, for dragging tabs: loaded after the first paint (Tech PRD «წარმადობა»). */
const SortableTabs = lazy(() => import("./SortableTabs"));

/** What dragging adds to a tab: dnd-kit's node, its moving style and its press. */
export interface TabDrag {
  ref: Ref<HTMLDivElement>;
  style: CSSProperties;
  onPointerDown: ((event: PointerEvent) => void) | undefined;
  dragging: boolean;
}

/** Draws tab `tab` (at `index` in the strip); `drag` once dragging is loaded. */
export type RenderTab = (tab: PaneTab, index: number, drag?: TabDrag) => ReactNode;

interface Props {
  pane: PaneSide;
  /** The strip's own width class and padding, from the title bar. */
  className?: string;
}

/**
 * One pane's tabs in the title bar, right above that pane: pinned tabs (40 px, icon only) first,
 * a separator, the other tabs (168 px for the shown one, 140 px for the rest, narrower when
 * many) and the new-tab button. A tab shows its folder's name, with the full path as its tooltip.
 * Pressing a tab shows it; dragging it moves it within its group; a middle click closes it; its
 * context menu duplicates, pins, closes others or those to the right, and copies its path. The
 * strip's empty space stays a drag region of the title bar; tabs and buttons opt out.
 */
export const TabStrip = memo(function TabStrip({ pane, className = "" }: Props) {
  const { tabs, active } = useTabs((state) => state[pane]);
  const activePane = useWindow((state) => state.activePane === pane);
  const newTabLabel = useCommandLabel("newTab");
  const name = pane === "left" ? strings.frame.leftPane : strings.frame.rightPane;

  const show = (id: number) => {
    activateTab(pane, id);
    activatePane(pane);
    focusListSoon(pane);
  };
  const render: RenderTab = (tab, index, drag) => {
    const shown = tab.id === active;
    return (
      <TabItem
        key={tab.id}
        pane={pane}
        tab={tab}
        shown={shown}
        activePane={activePane}
        // A separator ends the pinned group.
        separator={tab.pinned && tabs[index + 1]?.pinned === false}
        closable={!tab.pinned && tabs.length > 1}
        last={index === tabs.length - 1}
        onShow={show}
        drag={drag}
      />
    );
  };

  return (
    <div
      role="tablist"
      aria-label={t.tabs(name)}
      className={`flex min-w-0 items-end gap-0.5 overflow-hidden px-1.5 ${className}`}
    >
      <Suspense fallback={tabs.map((tab, index) => render(tab, index))}>
        <SortableTabs pane={pane} tabs={tabs} render={render} />
      </Suspense>
      <IconButton
        label={newTabLabel}
        onClick={() => newTab(pane)}
        className="mb-0.5 size-7 rounded-ctrl"
      >
        <PlusIcon />
      </IconButton>
    </div>
  );
});

interface TabProps {
  pane: PaneSide;
  tab: PaneTab;
  /** The tab its pane shows: wider, on the toolbar's color, with its close button. */
  shown: boolean;
  /** Its pane is the active one: the shown tab's name gets the primary text color. */
  activePane: boolean;
  separator: boolean;
  /** It can close: not pinned, and not its pane's last tab. */
  closable: boolean;
  /** The strip's last tab: nothing to close to its right. */
  last: boolean;
  onShow: (id: number) => void;
  drag: TabDrag | undefined;
}

function TabItem(props: TabProps) {
  const tab = tabById(props.tab.id);
  // A tab closing goes before its strip re-renders.
  return tab ? <TabView {...props} model={tab} /> : null;
}

function TabView({
  pane,
  tab,
  model,
  shown,
  activePane,
  separator,
  closable,
  last,
  onShow,
  drag,
}: TabProps & { model: Tab }) {
  const entry = useStore(model.store, (state) => state.history[state.index]);
  const dual = useSetting("dualPane") ?? true;
  const label = entry?.name ?? t.noFolder;
  const close = () => closeTab(pane, tab.id);

  // Tabs switch on press, as in browsers; the keyboard uses Enter or Space.
  const press = (event: PointerEvent) => {
    drag?.onPointerDown?.(event);
    if (event.button === 0) {
      onShow(tab.id);
    }
  };
  const keys = (event: { key: string; preventDefault(): void }) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onShow(tab.id);
    }
  };
  // The keyboard stays with the list (the tab hands it there), and a middle press doesn't
  // autoscroll.
  const mouseDown = (event: MouseEvent) => event.preventDefault();
  const auxClick = (event: MouseEvent) => {
    if (event.button === 1 && closable) {
      close();
    }
  };
  const look = `${shown ? "bg-bar" : "hover:bg-subtle-hover"} ${drag?.dragging ? "z-10" : ""}`;

  const menu = (
    <>
      <MenuItem onSelect={() => duplicateTab(pane, tab.id)} disabled={!entry}>
        {t.duplicate}
      </MenuItem>
      <MenuItem onSelect={() => pinTab(pane, tab.id, !tab.pinned)}>
        {tab.pinned ? t.unpin : t.pin}
      </MenuItem>
      <MenuSeparator />
      <MenuItem onSelect={close} disabled={!closable}>
        {t.closeTab}
      </MenuItem>
      <MenuItem onSelect={() => closeOtherTabs(pane, tab.id)}>{t.closeOthers}</MenuItem>
      <MenuItem onSelect={() => closeTabsToRight(pane, tab.id)} disabled={last}>
        {t.closeToRight}
      </MenuItem>
      <MenuSeparator />
      <MenuItem onSelect={() => openInOtherPane(pane, model)} disabled={!entry || !dual}>
        {t.openInOtherPane}
      </MenuItem>
      <MenuItem
        onSelect={() => entry && void commands.folderCopyPath(entry.folder)}
        disabled={!entry}
      >
        {t.copyPath}
      </MenuItem>
    </>
  );

  const common = {
    ref: drag?.ref,
    style: drag?.style,
    title: entry?.path,
    onPointerDown: press,
    onMouseDown: mouseDown,
    onAuxClick: auxClick,
    onKeyDown: keys,
  } as const;

  if (tab.pinned) {
    return (
      <>
        <ContextMenu label={t.tabMenu} menu={menu}>
          <div
            {...common}
            role="tab"
            aria-selected={shown}
            tabIndex={shown ? 0 : -1}
            aria-label={t.pinnedTab(label)}
            className={`no-drag relative flex h-tab w-10 shrink-0 items-center justify-center rounded-tab ${look}`}
          >
            <FolderIcon />
            <PinIcon className="absolute top-1.75 right-1.75 text-fg-2" />
          </div>
        </ContextMenu>
        {separator && (
          <div role="presentation" className="mt-2 h-4 w-px shrink-0 self-center bg-tab-sep" />
        )}
      </>
    );
  }

  return (
    <ContextMenu label={t.tabMenu} menu={menu}>
      <div
        {...common}
        role="tab"
        aria-selected={shown}
        tabIndex={shown ? 0 : -1}
        className={`group no-drag flex h-tab shrink items-center gap-2 rounded-tab pr-1.5 pl-2.5 ${
          shown ? "w-tab-active min-w-16" : "w-tab-inactive min-w-10"
        } ${look}`}
      >
        <FolderIcon />
        <span
          className={`min-w-0 flex-1 truncate text-small ${shown && activePane ? "text-fg" : "text-fg-2"}`}
        >
          {label}
        </span>
        {closable && (
          <button
            type="button"
            aria-label={t.closeTab}
            tabIndex={-1}
            // Closing isn't showing or dragging: the press stays on the button.
            onPointerDown={(event) => event.stopPropagation()}
            onClick={close}
            className={`size-6 shrink-0 items-center justify-center rounded-ctrl text-fg-2 hover:bg-subtle-hover active:bg-subtle-pressed ${
              shown ? "flex" : "hidden group-hover:flex"
            }`}
          >
            <CloseIcon />
          </button>
        )}
      </div>
    </ContextMenu>
  );
}

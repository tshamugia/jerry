import { memo, type PointerEvent } from "react";
import { useCommandLabel } from "../../stores/keymap";
import { activateTab, closeTab, type TabView, useTabs } from "../../stores/tabs";
import { activatePane, type PaneSide, useWindow } from "../../stores/window";
import { strings } from "../../strings";
import { IconButton } from "../../ui/IconButton";
import { CloseIcon, FolderIcon, PinIcon, PlusIcon } from "../../ui/icons";

interface Props {
  pane: PaneSide;
  /** The strip's own width class and padding, from the title bar. */
  className?: string;
}

/**
 * One pane's tabs in the title bar, right above that pane: pinned tabs (40 px, icon only) first,
 * a separator, the other tabs (168 px for the shown one, 140 px for the rest, narrower when
 * many) and the new-tab button. The strip's empty space stays a drag region of the title bar;
 * tabs and buttons opt out.
 */
export const TabStrip = memo(function TabStrip({ pane, className = "" }: Props) {
  const { tabs, active } = useTabs((state) => state[pane]);
  const activePane = useWindow((state) => state.activePane === pane);
  const newTab = useCommandLabel("newTab");
  const name = pane === "left" ? strings.frame.leftPane : strings.frame.rightPane;

  const show = (id: number) => {
    activateTab(pane, id);
    activatePane(pane);
  };

  return (
    <div
      role="tablist"
      aria-label={strings.titleBar.tabs(name)}
      className={`flex min-w-0 items-end gap-0.5 overflow-hidden px-1.5 ${className}`}
    >
      {tabs.map((tab, index) => (
        <Tab
          key={tab.id}
          tab={tab}
          shown={tab.id === active}
          activePane={activePane}
          // A separator ends the pinned group.
          separator={tab.pinned && tabs[index + 1]?.pinned === false}
          onShow={show}
          onClose={(id) => closeTab(pane, id)}
        />
      ))}
      {/* Opens a tab with #31. */}
      <IconButton label={newTab} disabled className="mb-0.5 size-7 rounded-ctrl">
        <PlusIcon />
      </IconButton>
    </div>
  );
});

interface TabProps {
  tab: TabView;
  /** The tab its pane shows: wider, on the toolbar's color, with its close button. */
  shown: boolean;
  /** Its pane is the active one: the shown tab's name gets the primary text color. */
  activePane: boolean;
  separator: boolean;
  onShow: (id: number) => void;
  onClose: (id: number) => void;
}

function Tab({ tab, shown, activePane, separator, onShow, onClose }: TabProps) {
  // Tabs switch on press, as in browsers; the keyboard uses Enter or Space.
  const press = (event: PointerEvent) => {
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
  const look = shown ? "bg-bar" : "hover:bg-subtle-hover";

  if (tab.pinned) {
    return (
      <>
        <div
          role="tab"
          aria-selected={shown}
          aria-label={strings.titleBar.pinnedTab(tab.name)}
          tabIndex={shown ? 0 : -1}
          onPointerDown={press}
          onKeyDown={keys}
          className={`no-drag relative flex h-tab w-10 shrink-0 items-center justify-center rounded-tab ${look}`}
        >
          <FolderIcon />
          <PinIcon className="absolute top-1.75 right-1.75 text-fg-2" />
        </div>
        {separator && (
          <div role="presentation" className="mt-2 h-4 w-px shrink-0 self-center bg-tab-sep" />
        )}
      </>
    );
  }

  return (
    <div
      role="tab"
      aria-selected={shown}
      tabIndex={shown ? 0 : -1}
      onPointerDown={press}
      onKeyDown={keys}
      className={`group no-drag flex h-tab shrink items-center gap-2 rounded-tab pr-1.5 pl-2.5 ${
        shown ? "w-tab-active min-w-16" : "w-tab-inactive min-w-10"
      } ${look}`}
    >
      <FolderIcon />
      <span
        className={`min-w-0 flex-1 truncate text-small ${shown && activePane ? "text-fg" : "text-fg-2"}`}
      >
        {tab.name}
      </span>
      <button
        type="button"
        aria-label={strings.titleBar.closeTab}
        tabIndex={-1}
        // Closing isn't showing: the press stays on the button.
        onPointerDown={(event) => event.stopPropagation()}
        onClick={() => onClose(tab.id)}
        className={`size-6 shrink-0 items-center justify-center rounded-ctrl text-fg-2 hover:bg-subtle-hover active:bg-subtle-pressed ${
          shown ? "flex" : "hidden group-hover:flex"
        }`}
      >
        <CloseIcon />
      </button>
    </div>
  );
}

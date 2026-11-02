import { strings } from "../strings";
import type { MockTab, PaneTabs } from "./mockTabs";

type Props = {
  label: string;
  tabs: PaneTabs;
  /** The pane is the active one: its active tab gets the primary text color. */
  activePane: boolean;
  onActivate: (id: number) => void;
  onClose: (id: number) => void;
  onNew: () => void;
};

/**
 * One pane's tabs, sitting above that pane. The strip itself is a drag region (empty space moves
 * the window); tabs and buttons opt out.
 */
export function TabStrip({ label, tabs, activePane, onActivate, onClose, onNew }: Props) {
  return (
    <div
      role="tablist"
      aria-label={label}
      className="flex min-w-0 items-end gap-0.5 overflow-hidden px-1.5"
    >
      {tabs.tabs.map((tab, index) => {
        const next = tabs.tabs[index + 1];
        // A separator ends the pinned group, as in the design.
        const separator = tab.pinned && next && !next.pinned;
        return (
          <Tab
            key={tab.id}
            tab={tab}
            active={tab.id === tabs.activeId}
            activePane={activePane}
            separator={Boolean(separator)}
            onActivate={onActivate}
            onClose={onClose}
          />
        );
      })}
      <button
        type="button"
        aria-label={strings.titleBar.newTab}
        onClick={onNew}
        className="no-drag mb-0.5 flex size-7 shrink-0 items-center justify-center rounded-ctrl text-fg-2 hover:bg-subtle-hover active:bg-subtle-pressed"
      >
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" aria-hidden="true">
          <path d="M8 3v10M3 8h10" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" />
        </svg>
      </button>
    </div>
  );
}

type TabProps = {
  tab: MockTab;
  active: boolean;
  activePane: boolean;
  separator: boolean;
  onActivate: (id: number) => void;
  onClose: (id: number) => void;
};

function Tab({ tab, active, activePane, separator, onActivate, onClose }: TabProps) {
  if (tab.pinned) {
    return (
      <>
        <button
          type="button"
          role="tab"
          aria-selected={active}
          aria-label={strings.titleBar.pinnedTab(tab.name)}
          onClick={() => onActivate(tab.id)}
          className={`no-drag relative flex h-tab w-10 shrink-0 items-center justify-center rounded-tab ${
            active ? "bg-bar" : "hover:bg-subtle-hover"
          }`}
        >
          <TabIcon icon={tab.icon} />
          <svg
            width="9"
            height="9"
            viewBox="0 0 16 16"
            className="absolute top-1.75 right-1.75 fill-fg-2"
            aria-hidden="true"
          >
            <path d="M10 1l5 5-2.2.6-2.6 2.6.4 3.3-1.6 1.6-3.2-3.2L2 15l-1-1 4.1-3.8L1.9 7l1.6-1.6 3.3.4 2.6-2.6z" />
          </svg>
        </button>
        {separator && <div className="mt-2 h-4 w-px shrink-0 self-center bg-tab-sep" />}
      </>
    );
  }

  return (
    // A div with role="tab", not a <button>: the close button sits inside it.
    <div
      role="tab"
      tabIndex={0}
      aria-selected={active}
      onClick={() => onActivate(tab.id)}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") onActivate(tab.id);
      }}
      className={`group no-drag flex h-tab min-w-10 shrink items-center gap-2 rounded-tab pr-1.5 pl-2.5 ${
        active ? "w-tab-active bg-bar" : "w-tab-inactive hover:bg-subtle-hover"
      }`}
    >
      <TabIcon icon={tab.icon} />
      <span
        data-tab-label
        className={`min-w-0 flex-1 truncate text-small ${active && activePane ? "text-fg" : "text-fg-2"}`}
      >
        {tab.name}
      </span>
      <button
        type="button"
        aria-label={strings.titleBar.closeTab}
        onClick={(e) => {
          e.stopPropagation();
          onClose(tab.id);
        }}
        className={`flex size-6 shrink-0 items-center justify-center rounded-ctrl text-fg-2 hover:bg-subtle-hover active:bg-subtle-pressed ${
          active ? "" : "invisible group-hover:visible"
        }`}
      >
        <svg width="10" height="10" viewBox="0 0 16 16" fill="none" aria-hidden="true">
          <path
            d="M3 3l10 10M13 3L3 13"
            stroke="currentColor"
            strokeWidth="1.6"
            strokeLinecap="round"
          />
        </svg>
      </button>
    </div>
  );
}

function TabIcon({ icon }: { icon: MockTab["icon"] }) {
  if (icon === "download") {
    return (
      <svg
        width="16"
        height="16"
        viewBox="0 0 16 16"
        fill="none"
        className="shrink-0"
        aria-hidden="true"
      >
        <path
          d="M8 2v8.5M4.5 7L8 10.5 11.5 7M3 13.5h10"
          className="stroke-fg-2"
          strokeWidth="1.2"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      </svg>
    );
  }
  return (
    <svg width="16" height="16" viewBox="0 0 16 16" className="shrink-0" aria-hidden="true">
      <path
        d="M1.5 3.5a1 1 0 0 1 1-1h3.6l1.5 1.5h5.9a1 1 0 0 1 1 1v7.5a1 1 0 0 1-1 1h-11a1 1 0 0 1-1-1z"
        className="fill-folder-back"
      />
      <rect x="1.5" y="5.5" width="13" height="8" rx="1" className="fill-folder-front" />
    </svg>
  );
}

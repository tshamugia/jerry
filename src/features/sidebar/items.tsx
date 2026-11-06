import type { ComponentProps, KeyboardEvent, MouseEvent, ReactNode } from "react";
import { createStore, useStore } from "zustand";
import { type TabState, useActiveTab } from "../../stores/tabs";
import { useWindow } from "../../stores/window";
import { strings } from "../../strings";
import { SectionIcon } from "../../ui/icons";
import { openPlace, type Section, toggleSection } from "./store";

/** The sidebar's building blocks: items, captions, the arrow keys. */

const t = strings.sidebar;

/** Every item that opens something, for the arrow keys. */
export const ITEM = "data-nav-item";

/** An item's look; its padding comes with it (`px-2`, or `pr-2 pl-6.5` under a caption). */
export const ITEM_LOOK =
  "relative flex w-full min-w-0 items-center gap-2.5 rounded-ctrl text-left hover:bg-hover";

export const BUTTON =
  "h-8 rounded-ctrl border border-ctrl-border bg-ctrl px-3 text-fg hover:bg-hover disabled:text-disabled";

/** A path as the sidebar compares folders: case-blind, without a trailing backslash. */
export function folderKey(path: string | undefined): string | undefined {
  return path?.replace(/\\+$/, "").toLowerCase();
}

const NO_TAB = createStore<TabState>(() => ({ history: [], index: -1 }));

/** The folder the active pane's tab shows, as display text. */
export function useCurrentPath(): string | undefined {
  const pane = useWindow((state) => state.activePane);
  const tab = useActiveTab(pane);
  return useStore(tab?.store ?? NO_TAB, (state) => state.history[state.index]?.path);
}

/**
 * Opens on a click: in the tab, or with Ctrl in a new one. A middle click opens a new tab, and so
 * does Ctrl+Enter. Up and Down move between the items.
 */
export function openHandlers(open: (newTab: boolean) => void) {
  return {
    onClick: (event: MouseEvent) => open(event.ctrlKey),
    // No autoscroll on the middle button.
    onMouseDown: (event: MouseEvent) => event.button === 1 && event.preventDefault(),
    onAuxClick: (event: MouseEvent) => event.button === 1 && open(true),
    onKeyDown: (event: KeyboardEvent<HTMLElement>) => {
      if (event.key === "Enter" && event.ctrlKey) {
        event.preventDefault();
        open(true);
      } else {
        moveFocus(event);
      }
    },
  };
}

/**
 * An item of one line. Other props (a context menu's trigger props, dragging's) go to the button;
 * its own `onKeyDown` runs first, and one that prevents the default keeps the item's keys out.
 */
export function NavItem({
  path,
  icon,
  label,
  detail,
  title,
  selected,
  indent = false,
  unavailable = false,
  onOpen,
  onKeyDown,
  ...rest
}: Omit<ComponentProps<"button">, "title"> & {
  path: string;
  icon: ReactNode;
  label: string;
  /** Small text at the end: a network location's server. */
  detail?: string;
  title?: string;
  selected: boolean;
  /** Under a caption's text: favorites, recent folders. */
  indent?: boolean;
  /** Its folder isn't there: grey. */
  unavailable?: boolean;
  /** Opens it; by default `path`, in the tab or a new one. */
  onOpen?: (newTab: boolean) => void;
}) {
  const handlers = openHandlers(onOpen ?? ((newTab) => openPlace(path, newTab)));
  return (
    <button
      {...rest}
      type="button"
      {...{ [ITEM]: "" }}
      title={title ?? path}
      aria-current={selected ? "location" : undefined}
      className={`${ITEM_LOOK} h-nav shrink-0 ${indent ? "pr-2 pl-6.5" : "px-2"} ${
        selected ? "bg-nav-sel" : ""
      } ${unavailable ? "text-disabled" : ""}`}
      {...handlers}
      onKeyDown={(event) => {
        onKeyDown?.(event);
        if (!event.defaultPrevented) {
          handlers.onKeyDown(event);
        }
      }}
    >
      {selected && <Pill />}
      <span className={`flex shrink-0 ${unavailable ? "opacity-50 grayscale" : ""}`}>{icon}</span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {detail && <span className="shrink-0 text-caption text-fg-3">{detail}</span>}
    </button>
  );
}

/** The 3 × 16 px accent pill on the selected item's left edge. */
export function Pill() {
  return (
    <span className="absolute top-1/2 left-0 z-10 h-4 w-0.75 -translate-y-1/2 rounded-bar bg-accent" />
  );
}

/** An `icon://` icon. */
export function ItemIcon({ src }: { src: string }) {
  return (
    <img src={src} alt="" width={16} height={16} draggable={false} className="size-4 shrink-0" />
  );
}

export function Separator() {
  return <div className="mx-2 my-1 h-px shrink-0 bg-stroke" />;
}

/**
 * A section's caption: a click folds or unfolds it. Other props (a context menu's trigger props)
 * go to its button.
 */
export function Caption({
  section,
  label,
  folded,
  action,
  ...rest
}: Omit<ComponentProps<"button">, "children"> & {
  section: Section;
  label: string;
  folded: boolean;
  action?: ReactNode;
}) {
  return (
    <div className="flex h-section shrink-0 items-center gap-1">
      <button
        {...rest}
        type="button"
        aria-expanded={!folded}
        aria-label={folded ? t.showSection(label) : t.hideSection(label)}
        onClick={() => toggleSection(section)}
        className="flex h-full min-w-0 flex-1 items-center gap-1.5 rounded-ctrl px-2 font-semibold text-fg-2 text-small hover:bg-hover"
      >
        <SectionIcon folded={folded} />
        <span className="truncate">{label}</span>
      </button>
      {action}
    </div>
  );
}

/** Up and Down (Home, End) on an item move the focus between the sidebar's items. */
export function moveFocus(event: KeyboardEvent<HTMLElement>): void {
  const step = { ArrowDown: 1, ArrowUp: -1 }[event.key];
  const edge = { Home: 0, End: -1 }[event.key];
  const list = event.currentTarget.closest("[data-nav-items]");
  if ((step === undefined && edge === undefined) || !list) {
    return;
  }
  const items = [...list.querySelectorAll<HTMLElement>(`[${ITEM}]`)];
  const at = items.indexOf(event.currentTarget);
  if (at < 0) {
    return;
  }
  event.preventDefault();
  const next = step !== undefined ? at + step : edge === 0 ? 0 : items.length - 1;
  items[Math.max(0, Math.min(items.length - 1, next))]?.focus();
}

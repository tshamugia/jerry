import { commands, type HistoryFolder, type SessionRestore } from "../../ipc";
import { lookOf } from "../../stores/folder";
import { addTab, type HistoryEntry, type Tab, tabById, useTabs } from "../../stores/tabs";
import { showToast } from "../../stores/toasts";
import { activatePane, type PaneSide } from "../../stores/window";
import { strings } from "../../strings";

const PANES: readonly PaneSide[] = ["left", "right"];

const entryOf = ({ focus, ...folder }: HistoryFolder): HistoryEntry =>
  focus === null ? folder : { ...folder, focus };

/**
 * Gives the window its tabs at start (#42): the session the core read before the window was
 * created, or the start folder from the settings, or the user's own folder at a first start.
 * Each pane's shown tab opens its folder at once (its listing started with the process); the
 * other tabs open theirs when they first show. If the core can't say, each pane opens the
 * user's own folder.
 */
export async function restoreSession(): Promise<void> {
  const result = await commands.sessionRestore();
  if (result.status === "error") {
    startAtHome();
    return;
  }
  restoreTabs(result.data);
}

/** Adds `session`'s tabs to the panes, as `restoreSession` does. */
export function restoreTabs(session: SessionRestore): void {
  const waiting = new Set<number>();
  for (const pane of PANES) {
    const { tabs, active } = session[pane];
    const shown = tabs[active] ?? tabs[0];
    for (const restored of tabs) {
      const tab = addTab(pane, {
        id: restored.id,
        pinned: restored.pinned,
        activate: restored === shown,
        from: { history: restored.history.map(entryOf), index: restored.index },
      });
      const look = lookOf(restored.look);
      if (look) {
        tab.folder.applyLook(look);
      }
      if (restored === shown) {
        reopen(tab);
      } else {
        waiting.add(tab.id);
      }
    }
  }
  activatePane(session.activePane);
  if (waiting.size > 0) {
    const stop = useTabs.subscribe((state) => {
      for (const pane of PANES) {
        const id = state[pane].active;
        if (id !== undefined && waiting.delete(id)) {
          const tab = tabById(id);
          if (tab) reopen(tab);
        }
      }
      for (const id of waiting) {
        if (!tabById(id)) waiting.delete(id);
      }
      if (waiting.size === 0) stop();
    });
  }
}

/** Opens a restored tab's folder; a folder that went away opens its nearest parent, with a note. */
function reopen(tab: Tab): void {
  tab.restore((gone, shown) =>
    showToast({
      title: strings.session.folderGone(gone.name),
      detail: strings.session.nearestOpened(shown.name),
    }),
  );
}

/** Each pane opens the user's own folder: the core couldn't say what to restore. */
function startAtHome(): void {
  for (const pane of PANES) {
    if (useTabs.getState()[pane].tabs.length === 0) {
      void addTab(pane, { activate: true }).open({ kind: "home" });
    }
  }
}

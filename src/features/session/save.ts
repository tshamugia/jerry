import { commands, type PaneSave, type SessionSave } from "../../ipc";
import { tabById, useTabs } from "../../stores/tabs";
import { type PaneSide, useWindow } from "../../stores/window";

/**
 * Sends this window's panes and tabs to the core whenever they change (#42): each pane's tabs,
 * pinned or not, the shown one, the active pane, and each tab's history and look. The core
 * writes them a second after the last change and at exit, so the next start, after a crash too,
 * opens them again. Changes made together go as one message, and only when something the session
 * keeps changed. Call once the tabs are restored; returns a function that stops it.
 */
export function saveSession(): () => void {
  let sent = "";
  let queued = false;
  const send = () => {
    queued = false;
    const session = snapshot();
    const json = JSON.stringify(session);
    if (json !== sent) {
      sent = json;
      void commands.sessionSave(session);
    }
  };
  const queue = () => {
    if (!queued) {
      queued = true;
      queueMicrotask(send);
    }
  };

  // Every tab's history, and its folder's sort and columns (not every batch it receives).
  const watched = new Map<number, () => void>();
  const watchTabs = () => {
    const { left, right } = useTabs.getState();
    const open = new Set([...left.tabs, ...right.tabs].map((tab) => tab.id));
    for (const id of open) {
      const tab = tabById(id);
      if (tab && !watched.has(id)) {
        const stopHistory = tab.store.subscribe(queue);
        const stopLook = tab.folder.store.subscribe((state, before) => {
          if (state.sort !== before.sort || state.columns !== before.columns) queue();
        });
        watched.set(id, () => {
          stopHistory();
          stopLook();
        });
      }
    }
    for (const [id, stop] of watched) {
      if (!open.has(id)) {
        stop();
        watched.delete(id);
      }
    }
  };

  const stopTabs = useTabs.subscribe(() => {
    watchTabs();
    queue();
  });
  const stopWindow = useWindow.subscribe(queue);
  watchTabs();
  queue();
  return () => {
    stopTabs();
    stopWindow();
    for (const stop of watched.values()) stop();
    watched.clear();
  };
}

/** The window's panes and tabs as the core keeps them. Tabs before their first folder are left out. */
export function snapshot(): SessionSave {
  const pane = (side: PaneSide): PaneSave => {
    const { tabs, active } = useTabs.getState()[side];
    const saved: PaneSave["tabs"] = [];
    let shown = 0;
    for (const { id, pinned } of tabs) {
      const tab = tabById(id);
      const { history, index } = tab?.store.getState() ?? { history: [], index: -1 };
      if (!tab || history.length === 0) {
        continue;
      }
      if (id === active) {
        shown = saved.length;
      }
      const { sort, columns } = tab.folder.store.getState();
      saved.push({
        pinned,
        history: history.map(({ folder, focus }) => ({ folder, focus: focus ?? null })),
        index: Math.max(index, 0),
        // v0.1 shows only Details (List and Grid come in v0.3).
        view: {
          view: "details",
          sort,
          columns: {
            order: [...columns.order],
            hidden: [...columns.hidden],
            widths: { ...columns.widths },
          },
        },
      });
    }
    return { tabs: saved, active: shown };
  };
  return {
    activePane: useWindow.getState().activePane,
    left: pane("left"),
    right: pane("right"),
  };
}

import { type CSSProperties, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type Command, useKeymap } from "../commands/registry";
import { useListing } from "../features/file-list/store";
import { installGeorgianSpike } from "../features/georgian/spike";
import { useGeorgian } from "../features/georgian/store";
import { commands } from "../ipc";
import { strings } from "../strings";
import { initialTabs, type PaneTabs } from "./mockTabs";
import { type Pane, TitleBar } from "./TitleBar";
import { useFrame } from "./useFrame";
import { Workspace } from "./Workspace";

/**
 * The window frame for spike #18: the title bar with split tabs (#17) over two panes listing a
 * folder of Georgian names (#12's list), and shortcuts matched by physical key.
 */
export function App() {
  // The window is created hidden; show it once the backdrop is set and a frame is painted
  // (no white flash).
  const showWindow = useCallback(() => void commands.appReady(), []);
  const frame = useFrame(showWindow);
  const [tabs, setTabs] = useState(initialTabs);
  const [activePane, setActivePane] = useState<Pane>("left");
  const [split, setSplit] = useState(0.5);
  const [path, setPath] = useState("");
  let nextId = Math.max(...[...tabs.left.tabs, ...tabs.right.tabs].map((t) => t.id)) + 1;

  useEffect(() => {
    installGeorgianSpike();
    void useListing.getState().open("georgian");
    void commands.listingGeorgianPath().then(setPath);
  }, []);

  // The native title (taskbar, Alt+Tab) follows the active tab, as Explorer's does.
  const activeName = activeTab(tabs[activePane]);
  useEffect(() => {
    void commands
      .windowSetTitle(`${activeName} — ${strings.appName}`)
      .then((title) => useGeorgian.getState().setWindowTitle(title));
  }, [activeName]);

  const filters = useRef<Record<Pane, HTMLInputElement | null>>({ left: null, right: null });
  const active = useRef(activePane);
  useEffect(() => {
    active.current = activePane;
  }, [activePane]);

  const keymap = useMemo<Command[]>(
    () => [
      // Copy, paste and new folder only log in the spike; the real handlers come with v0.1.
      { id: "copy", title: strings.commands.copy, shortcut: "Ctrl+C", context: "list", run() {} },
      { id: "paste", title: strings.commands.paste, shortcut: "Ctrl+V", context: "list", run() {} },
      {
        id: "filter",
        title: strings.commands.filter,
        shortcut: "Ctrl+F",
        context: "global",
        run: () => {
          const input = filters.current[active.current];
          input?.focus();
          input?.select();
        },
      },
      {
        id: "newFolder",
        title: strings.commands.newFolder,
        shortcut: "Ctrl+Shift+N",
        context: "list",
        run() {},
      },
    ],
    [],
  );
  const logRun = useCallback((command: Command, event: KeyboardEvent) => {
    const { id, title, shortcut } = command;
    useGeorgian.getState().log({ id, title, shortcut, key: event.key, code: event.code });
  }, []);
  useKeymap(keymap, logRun);

  const update = (pane: Pane, change: (current: PaneTabs) => PaneTabs) => {
    setTabs((all) => ({ ...all, [pane]: change(all[pane]) }));
    setActivePane(pane);
  };

  const columns = {
    "--frame-left": `${split}fr`,
    "--frame-right": `${1 - split}fr`,
  } as CSSProperties;

  return (
    <div className="flex h-full flex-col text-fg" style={columns}>
      <TitleBar
        frame={frame}
        tabs={tabs}
        activePane={activePane}
        onActivate={(pane, id) => update(pane, (p) => ({ ...p, activeId: id }))}
        onClose={(pane, id) =>
          update(pane, (p) => {
            const rest = p.tabs.filter((t) => t.id !== id);
            if (rest.length === 0) return p;
            const activeId = p.activeId === id ? (rest.at(-1)?.id ?? p.activeId) : p.activeId;
            return { tabs: rest, activeId };
          })
        }
        onNew={(pane) =>
          update(pane, (p) => {
            const id = nextId++;
            const tab = { id, name: strings.titleBar.newTabName, icon: "folder" as const };
            return { tabs: [...p.tabs, tab], activeId: id };
          })
        }
      />
      <Workspace
        path={path}
        activePane={activePane}
        onActivatePane={setActivePane}
        onSplit={setSplit}
        filterRef={(pane) => (input) => {
          filters.current[pane] = input;
        }}
      />
    </div>
  );
}

function activeTab(pane: PaneTabs): string {
  return pane.tabs.find((t) => t.id === pane.activeId)?.name ?? "";
}

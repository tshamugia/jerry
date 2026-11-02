import { type CSSProperties, useCallback, useState } from "react";
import { commands } from "../ipc";
import { strings } from "../strings";
import { initialTabs, type PaneTabs } from "./mockTabs";
import { type Pane, TitleBar } from "./TitleBar";
import { useFrame } from "./useFrame";
import { Workspace } from "./Workspace";

/**
 * The window frame: the title bar with the split tab strip over a mocked body (spike #17).
 * The real zones arrive with #22 and #23.
 */
export function App() {
  // The window is created hidden; show it once the backdrop is set and a frame is painted
  // (no white flash).
  const showWindow = useCallback(() => void commands.appReady(), []);
  const frame = useFrame(showWindow);
  const [tabs, setTabs] = useState(initialTabs);
  const [activePane, setActivePane] = useState<Pane>("left");
  const [split, setSplit] = useState(0.5);
  let nextId = Math.max(...[...tabs.left.tabs, ...tabs.right.tabs].map((t) => t.id)) + 1;

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
        frame={frame}
        tabs={tabs}
        activePane={activePane}
        onActivatePane={setActivePane}
        onSplit={setSplit}
      />
    </div>
  );
}

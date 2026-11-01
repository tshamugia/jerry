import {
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  commands,
  type DndDone,
  type Effect,
  type InstallReport,
  type LabEntry,
  type LabFolder,
} from "../../ipc";
import { strings } from "../../strings";
import type { DropHit } from "./target";
import { useOleDrop } from "./useOleDrop";

/**
 * Spike #14: a stand-in for Jerry's window with every kind of drop target the PRD names: tabs,
 * a breadcrumb, sidebar favorites, folder rows and the list itself. Drops come in through
 * Rust's OLE drop target (see `useOleDrop`); rows also drag out, as in spike #13, so drags
 * inside Jerry go through the same OLE path.
 */

/** Windows' default drag threshold (SM_CXDRAG / SM_CYDRAG) is 4 px; #58 reads the metric. */
const DRAG_THRESHOLD = 4;

type Tab = { folder: number };
type Pending = { x: number; y: number; mask: number; ids: number[] };

const sizeFormat = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });

function formatSize(bytes: number | null) {
  if (bytes === null) return "";
  return bytes < 1024 ? `${bytes} B` : `${sizeFormat.format(bytes / 1024)} KB`;
}

/** Highlight of the element a drop would land on. */
function dropClass(hover: DropHit | null, key: string) {
  return hover?.key === key ? "bg-sel outline-2 -outline-offset-2 outline-focus" : "";
}

export function DropLab() {
  const [folders, setFolders] = useState<LabFolder[]>([]);
  const [tabs, setTabs] = useState<Tab[]>([]);
  const [active, setActive] = useState(0);
  const [entries, setEntries] = useState<LabEntry[]>([]);
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const [install, setInstall] = useState<InstallReport | null>(null);
  const [status, setStatus] = useState<string>("");
  const selectedRef = useRef(selected);
  const pending = useRef<Pending | null>(null);

  const byId = useMemo(() => new Map(folders.map((f) => [f.id, f])), [folders]);
  const current = tabs[active]?.folder;

  const loadFolders = useCallback(async () => {
    const result = await commands.labFolders();
    if (result.status === "error") {
      setStatus(strings.spike.error(result.error));
      return;
    }
    setFolders(result.data);
    setTabs((open) => {
      if (open.length > 0) return open;
      const named = (name: string) => result.data.find((f) => f.name === name)?.id;
      return [result.data[0]?.id, named("Drawings"), named("Outlook")]
        .filter((id): id is number => id !== undefined)
        .map((folder) => ({ folder }));
    });
  }, []);

  const loadEntries = useCallback(async (folder: number | undefined) => {
    if (folder === undefined) return;
    const result = await commands.labList(folder);
    if (result.status === "ok") setEntries(result.data);
    else setStatus(strings.spike.error(result.error));
  }, []);

  const select = useCallback((next: ReadonlySet<number>) => {
    selectedRef.current = next;
    setSelected(next);
  }, []);

  const open = useCallback(
    (folder: number) => {
      setTabs((list) => list.map((tab, index) => (index === active ? { folder } : tab)));
      select(new Set());
    },
    [active, select],
  );

  // Resting on a tab opens it; resting on a folder row opens the folder (spring-loaded).
  const onDwell = useCallback(
    (hit: DropHit) => {
      if (hit.kind === "tab") {
        const index = tabs.findIndex((_, i) => `tab:${i}` === hit.key);
        if (index >= 0) setActive(index);
      } else if (hit.kind === "row") {
        open(hit.folder);
      }
    },
    [tabs, open],
  );

  const onDone = useCallback(
    (_done: DndDone) => {
      void loadFolders();
      void loadEntries(current);
    },
    [loadFolders, loadEntries, current],
  );

  const { drag, hover, keys, stats, log, menu, choose, domDrags } = useOleDrop(onDwell, onDone);

  useEffect(() => {
    void loadFolders();
    void commands.dndInstall().then((result) => {
      if (result.status === "ok") setInstall(result.data);
      else setStatus(strings.spike.error(result.error));
    });
  }, [loadFolders]);

  useEffect(() => {
    void loadEntries(current);
  }, [loadEntries, current]);

  // Tell Rust where the drop targets are, for the probe that drives test drags.
  useEffect(() => {
    let sent = "";
    const report = () => {
      const rects = [...document.querySelectorAll<HTMLElement>("[data-drop-key]")].map((el) => {
        const r = el.getBoundingClientRect();
        return { key: el.dataset.dropKey ?? "", x: r.x, y: r.y, w: r.width, h: r.height };
      });
      const text = JSON.stringify(rects);
      if (text === sent) return;
      sent = text;
      void commands.labLayout(window.devicePixelRatio, rects);
    };
    const frame = requestAnimationFrame(report);
    window.addEventListener("resize", report);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", report);
    };
  });

  const startDrag = useCallback(
    async (ids: number[]) => {
      const uiTime = performance.timeOrigin + performance.now();
      const result = await commands.dragStart(ids, uiTime);
      if (result.status === "error") setStatus(strings.spike.error(result.error));
      else setStatus(strings.spike.report(result.data));
      void loadEntries(current);
    },
    [loadEntries, current],
  );

  useEffect(() => {
    const onMove = (event: PointerEvent) => {
      const start = pending.current;
      if (!start) return;
      if ((event.buttons & start.mask) === 0) {
        pending.current = null;
        return;
      }
      const dx = Math.abs(event.clientX - start.x);
      const dy = Math.abs(event.clientY - start.y);
      if (dx < DRAG_THRESHOLD && dy < DRAG_THRESHOLD) return;
      pending.current = null;
      void startDrag(start.ids);
    };
    window.addEventListener("pointermove", onMove);
    return () => window.removeEventListener("pointermove", onMove);
  }, [startDrag]);

  const onRowPointerDown = (event: ReactPointerEvent, id: number) => {
    if (event.button !== 0 && event.button !== 2) return;
    const was = selectedRef.current;
    let next: ReadonlySet<number>;
    if (event.ctrlKey && event.button === 0) {
      const toggled = new Set(was);
      if (!toggled.delete(id)) toggled.add(id);
      next = toggled;
    } else {
      next = was.has(id) ? was : new Set([id]);
    }
    select(next);
    pending.current = next.has(id)
      ? { x: event.clientX, y: event.clientY, mask: event.button === 0 ? 1 : 2, ids: [...next] }
      : null;
  };

  const crumbs = useMemo(() => {
    const chain: LabFolder[] = [];
    let folder = current === undefined ? undefined : byId.get(current);
    while (folder) {
      chain.unshift(folder);
      folder = folder.parent == null ? undefined : byId.get(folder.parent);
    }
    return chain;
  }, [current, byId]);

  const currentName = current === undefined ? "" : (byId.get(current)?.name ?? "");

  return (
    <div className="relative flex h-full min-h-0 flex-col">
      {/* Tabs: a drop target each; resting on one opens it. */}
      <div className="flex h-tab shrink-0 items-end gap-1 bg-mica px-2" data-drop="none">
        {tabs.map((tab, index) => {
          const folder = byId.get(tab.folder);
          const key = `tab:${index}`;
          return (
            <button
              type="button"
              key={key}
              data-drop="tab"
              data-drop-key={key}
              data-folder={tab.folder}
              data-name={folder?.name}
              className={`h-full w-40 truncate rounded-tab px-3 text-left ${
                index === active ? "bg-bar" : "hover:bg-hover"
              } ${dropClass(hover, key)}`}
              onClick={() => setActive(index)}
            >
              {folder?.name}
            </button>
          );
        })}
      </div>

      <div className="flex min-h-0 flex-1">
        {/* Sidebar: every lab folder is a favorite; "Network" refuses drops. */}
        <nav className="w-sidebar shrink-0 overflow-auto bg-mica p-2" data-drop="none">
          <p className="px-2 pb-1 text-caption text-fg-3">{strings.drop.favorites}</p>
          {folders.map((folder) => {
            const key = `side:${folder.id}`;
            return (
              <button
                type="button"
                key={key}
                data-drop="sidebar"
                data-drop-key={key}
                data-folder={folder.id}
                data-name={folder.name}
                style={{ paddingLeft: `${8 + folder.depth * 12}px` }}
                className={`flex h-nav w-full items-center truncate rounded-ctrl pr-2 text-left hover:bg-hover ${
                  folder.id === current ? "bg-nav-sel" : ""
                } ${dropClass(hover, key)}`}
                onClick={() => open(folder.id)}
              >
                {folder.name}
              </button>
            );
          })}
          <div className="flex h-nav items-center px-2 text-fg-3" data-drop-key="side:network">
            {strings.drop.network}
          </div>
        </nav>

        <section className="flex min-w-0 flex-1 flex-col bg-pane">
          {/* Breadcrumb: each segment is a drop target for that folder. */}
          <div className="flex h-address shrink-0 items-center gap-1 bg-bar px-2" data-drop="none">
            {crumbs.map((folder, index) => {
              const key = `crumb:${folder.id}`;
              return (
                <span key={key} className="flex items-center gap-1">
                  {index > 0 && <span className="text-fg-3">›</span>}
                  <button
                    type="button"
                    data-drop="crumb"
                    data-drop-key={key}
                    data-folder={folder.id}
                    data-name={folder.name}
                    className={`rounded-ctrl px-2 py-0.5 hover:bg-hover ${
                      index === crumbs.length - 1 ? "font-semibold" : ""
                    } ${dropClass(hover, key)}`}
                    onClick={() => open(folder.id)}
                  >
                    {folder.name}
                  </button>
                </span>
              );
            })}
          </div>

          {/* The list: folder rows take drops; anything else drops into the current folder. */}
          <div
            role="listbox"
            aria-multiselectable
            data-drop="pane"
            data-drop-key="pane"
            data-folder={current}
            data-name={currentName}
            className={`min-h-0 flex-1 overflow-auto p-1 ${
              hover?.key === "pane" ? "outline-2 -outline-offset-2 outline-focus" : ""
            }`}
            onContextMenu={(event) => event.preventDefault()}
            onDragStart={(event) => event.preventDefault()}
          >
            {entries.length === 0 && (
              <p className="p-4 text-center text-fg-3">{strings.drop.empty}</p>
            )}
            {entries.map((entry) => {
              const key = `row:${entry.id}`;
              const target = entry.isDir
                ? {
                    "data-drop": "row",
                    "data-drop-key": key,
                    "data-folder": entry.id,
                    "data-name": entry.name,
                  }
                : { "data-drop-key": key };
              return (
                <div
                  key={key}
                  role="option"
                  tabIndex={-1}
                  aria-selected={selected.has(entry.id)}
                  {...target}
                  className={`flex h-row items-center gap-3 rounded-ctrl px-3 ${
                    selected.has(entry.id) ? "bg-sel" : "hover:bg-hover"
                  } ${dropClass(hover, key)}`}
                  onPointerDown={(event) => onRowPointerDown(event, entry.id)}
                  onPointerUp={(event) => {
                    if (pending.current && !event.ctrlKey) select(new Set([entry.id]));
                    pending.current = null;
                  }}
                  onDoubleClick={() => entry.isDir && open(entry.id)}
                >
                  <span className="flex-1 truncate">{entry.name}</span>
                  <span className="w-24 text-right text-fg-2 tabular-nums">
                    {entry.isDir ? strings.spike.folder : formatSize(entry.size)}
                  </span>
                </div>
              );
            })}
          </div>

          <StatusPanel
            install={install}
            status={status}
            drag={drag}
            hoverName={hover ? `${hover.kind} · ${hover.name}` : null}
            keys={keys ? strings.drop.keys(keys) : null}
            stats={stats}
            log={log}
            domDrags={domDrags}
          />
        </section>
      </div>

      {menu && <DropMenuView menu={menu} onChoose={choose} />}
    </div>
  );
}

function DropMenuView({
  menu,
  onChoose,
}: {
  menu: { x: number; y: number };
  onChoose: (choice: Effect | null) => void;
}) {
  const dpr = window.devicePixelRatio || 1;
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.code === "Escape") onChoose(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onChoose]);
  const item = "flex h-row w-full items-center rounded-ctrl px-3 text-left hover:bg-hover";
  return (
    <div
      role="menu"
      className="absolute z-10 w-44 rounded-flyout border border-flyout-border bg-flyout p-1 shadow-flyout"
      style={{ left: menu.x / dpr, top: menu.y / dpr - 40 }}
    >
      <button type="button" role="menuitem" className={item} onClick={() => onChoose("copy")}>
        {strings.drop.copyHere}
      </button>
      <button type="button" role="menuitem" className={item} onClick={() => onChoose("move")}>
        {strings.drop.moveHere}
      </button>
      <button type="button" role="menuitem" className={item} onClick={() => onChoose(null)}>
        {strings.drop.cancel}
      </button>
    </div>
  );
}

function StatusPanel(props: {
  install: InstallReport | null;
  status: string;
  drag: ReturnType<typeof useOleDrop>["drag"];
  hoverName: string | null;
  keys: string | null;
  stats: ReturnType<typeof useOleDrop>["stats"];
  log: ReturnType<typeof useOleDrop>["log"];
  domDrags: number;
}) {
  const { install, status, drag, hoverName, keys, stats, log, domDrags } = props;
  return (
    <div
      className="max-h-64 shrink-0 overflow-auto border-t border-stroke bg-bar p-3 text-small text-fg-2 tabular-nums"
      data-testid="drop-status"
    >
      <p>{install ? strings.drop.install(install) : strings.drop.installing}</p>
      {domDrags > 0 && <p className="text-red">{strings.drop.domDrags(domDrags)}</p>}
      {status && <p>{status}</p>}
      {drag && <p>{strings.drop.drag(drag)}</p>}
      {hoverName && (
        <p className="text-fg">
          {strings.drop.over(hoverName)}
          {keys && ` · ${keys}`}
        </p>
      )}
      {stats && <p>{strings.drop.stats(stats)}</p>}
      <ol>
        {log.map((entry) => (
          <li key={entry.dropId}>{strings.drop.logEntry(entry)}</li>
        ))}
      </ol>
      {install && (
        <details>
          <summary>{strings.drop.windows}</summary>
          <pre className="text-caption">{install.before.join("\n")}</pre>
          <pre className="text-caption">{install.after.join("\n")}</pre>
        </details>
      )}
    </div>
  );
}

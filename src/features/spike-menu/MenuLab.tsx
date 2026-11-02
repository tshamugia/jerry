import { type KeyboardEvent, type MouseEvent, useEffect, useState } from "react";
import { commands, events, type LabItem, type MenuReport, type SpikeError } from "../../ipc";
import { strings } from "../../strings";

const t = strings.spikeMenu;

/** The ids each scripted case opens Windows' menu for. */
export function caseIds(items: LabItem[]): Record<keyof typeof t.cases, number[]> {
  const files = items.filter((item) => item.kind === "file").map((item) => item.id);
  const ofKind = (kind: LabItem["kind"]) =>
    items.filter((item) => item.kind === kind).map((item) => item.id);
  return {
    oneFile: files.slice(0, 1),
    severalFiles: files,
    folder: ofKind("folder"),
    longPath: ofKind("long"),
    mixedFolders: [...files.slice(0, 1), ...ofKind("elsewhere")],
  };
}

/**
 * Spike #16 lab: Windows' own context menu ("Windows options…") for real files. Right-click a
 * row (Shift for extended verbs), press the Menu key or Shift+F10 on one, or use a case button
 * (the E2E script presses those through UI Automation).
 */
export function MenuLab() {
  const [items, setItems] = useState<LabItem[]>([]);
  const [selected, setSelected] = useState<number[]>([]);
  const [report, setReport] = useState<MenuReport>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    void commands.labItems().then((result) => {
      if (result.status === "ok") setItems(result.data);
      else setError(errorText(result.error));
    });
    // Answer while the menu is still up: proves Rust → page events flow during its loop.
    const unlisten = events.menuOpened.listen(({ payload }) => {
      void commands.menuAck(payload.seq);
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  }, []);

  const open = async (ids: number[], x: number, y: number, extended: boolean) => {
    if (ids.length === 0) return;
    setBusy(true);
    setError(undefined);
    const result = await commands.windowsMenu(ids, Math.round(x), Math.round(y), extended);
    setBusy(false);
    if (result.status === "ok") setReport(result.data);
    else setError(errorText(result.error));
  };

  const onRowContextMenu = (event: MouseEvent, id: number) => {
    event.preventDefault();
    const ids = selected.includes(id) ? selected : [id];
    setSelected(ids);
    void open(ids, event.clientX, event.clientY, event.shiftKey);
  };

  const onRowClick = (event: MouseEvent, id: number) => {
    if (event.ctrlKey) {
      setSelected((current) =>
        current.includes(id) ? current.filter((other) => other !== id) : [...current, id],
      );
    } else {
      setSelected([id]);
    }
  };

  // The Menu key and Shift+F10 open the menu at the focused row, as in Explorer.
  const onRowKeyDown = (event: KeyboardEvent<HTMLButtonElement>, id: number) => {
    if (event.code !== "ContextMenu" && !(event.code === "F10" && event.shiftKey)) return;
    event.preventDefault();
    const ids = selected.includes(id) ? selected : [id];
    setSelected(ids);
    const rect = event.currentTarget.getBoundingClientRect();
    void open(ids, rect.left + 24, rect.bottom, false);
  };

  const cases = caseIds(items);

  return (
    <div className="flex flex-1 flex-col gap-4 overflow-auto p-4">
      <p className="text-fg-2">{t.hint}</p>
      <ul className="flex flex-col" aria-label={t.items}>
        {items.map((item) => (
          <li key={item.id}>
            <button
              type="button"
              aria-pressed={selected.includes(item.id)}
              className="h-row w-full truncate rounded-ctrl px-2 text-left hover:bg-hover aria-pressed:bg-sel"
              onClick={(event) => onRowClick(event, item.id)}
              onContextMenu={(event) => onRowContextMenu(event, item.id)}
              onKeyDown={(event) => onRowKeyDown(event, item.id)}
            >
              {item.name}
            </button>
          </li>
        ))}
      </ul>
      <div className="flex flex-wrap gap-2">
        {(Object.keys(t.cases) as (keyof typeof t.cases)[]).map((key) => (
          <button
            key={key}
            type="button"
            disabled={busy || cases[key].length === 0}
            className="h-row rounded-ctrl bg-ctrl px-3 hover:bg-hover disabled:text-disabled"
            onClick={(event) => {
              const rect = event.currentTarget.getBoundingClientRect();
              void open(cases[key], rect.left + rect.width / 2, rect.bottom, false);
            }}
          >
            {t.cases[key]}
          </button>
        ))}
      </div>
      {error && <p role="alert">{error}</p>}
      {report && (
        <section aria-label={t.lastMenu} className="flex flex-col gap-1">
          <p>{t.summary(report.chosen ?? null, report.invoked, report.ackDuringMenu)}</p>
          <pre className="text-small text-fg-2 tabular-nums">{JSON.stringify(report, null, 2)}</pre>
        </section>
      )}
    </div>
  );
}

function errorText(error: SpikeError): string {
  return t.errors[error.code];
}

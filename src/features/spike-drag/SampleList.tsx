import {
  type PointerEvent as ReactPointerEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import { commands, type SampleEntry } from "../../ipc";
import { strings } from "../../strings";

/**
 * Spike #13: a plain list of the sample folder that drags files out with Windows OLE.
 *
 * The UI only notices the gesture: a pointer move past the threshold with a button held. Rust
 * then runs the whole OLE drag on the main thread (`drag_start`), and the promise resolves on
 * drop or cancel. Rows are referred to by id; paths never leave Rust.
 */

/** Windows' default drag threshold (SM_CXDRAG / SM_CYDRAG) is 4 px; #58 reads the metric. */
const DRAG_THRESHOLD = 4;

/** Keep logging page events this long after a drag ends, to catch late pointerup and friends. */
const LOG_AFTER_DRAG_MS = 1500;

const LOGGED_EVENTS = [
  "pointerdown",
  "pointermove",
  "pointerup",
  "pointercancel",
  "lostpointercapture",
  "mousedown",
  "mouseup",
  "click",
  "contextmenu",
  "dragstart",
  "blur",
  "focus",
] as const;

type Pending = { x: number; y: number; mask: number; ids: number[] };
type PageEvent = { type: string; at: number; buttons: number; count: number };

const sizeFormat = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });

function formatSize(bytes: number | null) {
  if (bytes === null) return "";
  return bytes < 1024 ? `${bytes} B` : `${sizeFormat.format(bytes / 1024)} KB`;
}

export function SampleList() {
  const [entries, setEntries] = useState<SampleEntry[]>([]);
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const [status, setStatus] = useState<string>(strings.spike.noDrag);
  const [capture, setCapture] = useState<string>();
  const [events, setEvents] = useState<PageEvent[]>([]);
  const selectedRef = useRef(selected);
  const pending = useRef<Pending | null>(null);
  // performance.now() when the current drag started, and until when to keep logging.
  const logFrom = useRef(0);
  const logUntil = useRef(0);

  // The ref lets pointer handlers read the selection before React re-renders.
  const select = useCallback((next: ReadonlySet<number>) => {
    selectedRef.current = next;
    setSelected(next);
  }, []);

  const refresh = useCallback(async () => {
    const result = await commands.sampleList();
    if (result.status === "ok") {
      setEntries(result.data);
      select(new Set());
    } else {
      setStatus(strings.spike.error(result.error));
    }
  }, [select]);

  const startDrag = useCallback(
    async (ids: number[]) => {
      const uiTime = performance.timeOrigin + performance.now();
      logFrom.current = performance.now();
      logUntil.current = Number.POSITIVE_INFINITY;
      setEvents([]);
      setStatus(strings.spike.dragging);

      const result = await commands.dragStart(ids, uiTime);
      logUntil.current = performance.now() + LOG_AFTER_DRAG_MS;
      if (result.status === "error") {
        setStatus(strings.spike.error(result.error));
        return;
      }
      setStatus(strings.spike.report(result.data));
      setCapture(strings.spike.capture(result.data.capture));
      if (result.data.effect === "move") {
        // The target moved the files out of the folder.
        void refresh();
      }
    },
    [refresh],
  );

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Past the threshold with the button still held: hand the drag to Windows.
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

  // What the page hears around an OLE drag (the spike's "pointer capture" question).
  useEffect(() => {
    const log = (event: Event) => {
      const now = performance.now();
      if (now < logFrom.current || now > logUntil.current) return;
      const buttons = event instanceof MouseEvent ? event.buttons : -1;
      setEvents((list) => {
        const last = list.at(-1);
        if (last && last.type === event.type && last.buttons === buttons) {
          return [...list.slice(0, -1), { ...last, count: last.count + 1 }];
        }
        return [...list, { type: event.type, at: now - logFrom.current, buttons, count: 1 }];
      });
    };
    for (const type of LOGGED_EVENTS) window.addEventListener(type, log, true);
    return () => {
      for (const type of LOGGED_EVENTS) window.removeEventListener(type, log, true);
    };
  }, []);

  // Chromium's own drop target is still on the WebView (#14 replaces it). Refuse drops on the
  // page, so a file dropped back on Jerry doesn't make WebView2 open it.
  useEffect(() => {
    const refuse = (event: DragEvent) => {
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "none";
    };
    document.addEventListener("dragover", refuse);
    document.addEventListener("drop", refuse);
    return () => {
      document.removeEventListener("dragover", refuse);
      document.removeEventListener("drop", refuse);
    };
  }, []);

  const onRowPointerDown = (event: ReactPointerEvent, id: number) => {
    if (event.button !== 0 && event.button !== 2) return;
    const current = selectedRef.current;
    let next: ReadonlySet<number>;
    if (event.ctrlKey && event.button === 0) {
      const toggled = new Set(current);
      if (!toggled.delete(id)) toggled.add(id);
      next = toggled;
    } else {
      // Pressing on a selected row keeps the selection, so several files drag together.
      next = current.has(id) ? current : new Set([id]);
    }
    select(next);
    pending.current = next.has(id)
      ? { x: event.clientX, y: event.clientY, mask: event.button === 0 ? 1 : 2, ids: [...next] }
      : null;
  };

  const onRowPointerUp = (event: ReactPointerEvent, id: number) => {
    // A click without a drag on one of several selected rows selects just that row.
    if (pending.current && !event.ctrlKey) select(new Set([id]));
    pending.current = null;
  };

  return (
    <div className="flex h-full flex-col gap-3 p-4">
      <div className="flex items-center gap-3">
        <h1 className="text-title">{strings.spike.title}</h1>
        <button
          type="button"
          className="h-row rounded-ctrl border border-ctrl-border bg-ctrl px-3 hover:bg-hover"
          onClick={() => void refresh()}
        >
          {strings.spike.refresh}
        </button>
      </div>
      <p className="text-fg-2">{strings.spike.hint}</p>

      <div
        role="listbox"
        aria-multiselectable
        className="rounded-ctrl border border-stroke bg-pane"
        onContextMenu={(event) => event.preventDefault()}
        onDragStart={(event) => event.preventDefault()}
      >
        {entries.map((entry) => (
          <div
            key={entry.id}
            role="option"
            tabIndex={-1}
            aria-selected={selected.has(entry.id)}
            data-id={entry.id}
            className={`flex h-row items-center gap-3 px-3 ${
              selected.has(entry.id) ? "bg-sel" : "hover:bg-hover"
            }`}
            onPointerDown={(event) => onRowPointerDown(event, entry.id)}
            onPointerUp={(event) => onRowPointerUp(event, entry.id)}
          >
            <span className="flex-1 truncate">{entry.name}</span>
            <span className="w-24 text-right text-fg-2 tabular-nums">
              {entry.isDir ? strings.spike.folder : formatSize(entry.size)}
            </span>
          </div>
        ))}
      </div>

      <output className="block" data-testid="drag-status">
        {status}
      </output>
      {capture && <p className="text-small text-fg-2">{capture}</p>}

      {events.length > 0 && (
        <section className="min-h-0 flex-1 overflow-auto text-small text-fg-2 tabular-nums">
          <h2 className="text-fg">{strings.spike.events}</h2>
          <ol data-testid="drag-events">
            {events.map((event) => (
              <li key={`${event.type}-${event.at}`}>
                +{event.at.toFixed(0)} ms {event.type}
                {event.count > 1 ? ` ×${event.count}` : ""} (buttons {event.buttons})
              </li>
            ))}
          </ol>
        </section>
      )}
    </div>
  );
}

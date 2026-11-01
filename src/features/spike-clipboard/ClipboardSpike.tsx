import {
  type MouseEvent as ReactMouseEvent,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import {
  Channel,
  type ClipboardState,
  type ClipboardState_Deserialize,
  commands,
  type SampleEntry,
} from "../../ipc";
import { strings } from "../../strings";

/**
 * Spike #15: a plain list of the sample folder with Copy, Cut and Paste through the Windows
 * clipboard, the way Explorer does them.
 *
 * Rows are referred to by id; paths never leave Rust. Shortcuts match `KeyboardEvent.code`, so
 * they work on the Georgian layout too. Rust sends the clipboard state on every change, and cut
 * rows stay dimmed only while the clipboard still holds Jerry's cut.
 */

const sizeFormat = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1 });

function formatSize(bytes: number | null) {
  if (bytes === null) return "";
  return bytes < 1024 ? `${bytes} B` : `${sizeFormat.format(bytes / 1024)} KB`;
}

export function ClipboardSpike() {
  const [entries, setEntries] = useState<SampleEntry[]>([]);
  const [selected, setSelected] = useState<ReadonlySet<number>>(new Set());
  const [clip, setClip] = useState<ClipboardState>();
  const [status, setStatus] = useState<string>(strings.spike.ready);
  const selectedRef = useRef(selected);
  const entriesRef = useRef<SampleEntry[]>([]);
  const anchor = useRef<number | null>(null);

  const select = useCallback((next: ReadonlySet<number>) => {
    selectedRef.current = next;
    setSelected(next);
  }, []);

  // Ids are positions in a listing, so the selection survives a new listing by name.
  const refresh = useCallback(async () => {
    const result = await commands.sampleList();
    if (result.status === "error") {
      setStatus(strings.spike.error(result.error));
      return;
    }
    // Read the selection only now: it may have changed while the listing ran.
    const keep = new Set(
      entriesRef.current.filter((entry) => selectedRef.current.has(entry.id)).map((e) => e.name),
    );
    entriesRef.current = result.data;
    setEntries(result.data);
    select(new Set(result.data.filter((entry) => keep.has(entry.name)).map((e) => e.id)));
    const state = await commands.clipboardState();
    if (state.status === "ok") setClip(state.data);
  }, [select]);

  const copy = useCallback(async (cut: boolean) => {
    const ids = [...selectedRef.current];
    if (ids.length === 0) {
      setStatus(strings.spike.nothingSelected);
      return;
    }
    const result = await commands.clipboardCopy(ids, cut);
    setStatus(
      result.status === "ok"
        ? strings.spike.copied(result.data)
        : strings.spike.error(result.error),
    );
  }, []);

  const paste = useCallback(async () => {
    setStatus(strings.spike.pasting);
    const result = await commands.clipboardPaste();
    if (result.status === "error") {
      setStatus(strings.spike.error(result.error));
      return;
    }
    setStatus(strings.spike.pasted(result.data));
    await refresh();
  }, [refresh]);

  useEffect(() => {
    void refresh();
    let hadCut = false;
    const channel = new Channel<ClipboardState_Deserialize>();
    channel.onmessage = (state) => {
      setClip(state);
      // Jerry's cut left the clipboard: pasted elsewhere (the files moved away) or replaced.
      if (hadCut && state.cutIds.length === 0) void refresh();
      hadCut = state.cutIds.length > 0;
    };
    void commands.clipboardWatch(channel);
  }, [refresh]);

  // Coming back from Explorer: the folder may have changed (a paste there moved files away).
  useEffect(() => {
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refresh]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (!event.ctrlKey || event.altKey || event.shiftKey || event.metaKey) return;
      switch (event.code) {
        case "KeyC":
          void copy(false);
          break;
        case "KeyX":
          void copy(true);
          break;
        case "KeyV":
          void paste();
          break;
        case "KeyA":
          select(new Set(entries.map((entry) => entry.id)));
          break;
        default:
          return;
      }
      event.preventDefault();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [copy, paste, select, entries]);

  const onRowClick = (event: ReactMouseEvent, id: number) => {
    const current = selectedRef.current;
    if (event.shiftKey && anchor.current !== null) {
      const ids = entries.map((entry) => entry.id);
      const [a, b] = [ids.indexOf(anchor.current), ids.indexOf(id)];
      select(new Set(ids.slice(Math.min(a, b), Math.max(a, b) + 1)));
      return;
    }
    anchor.current = id;
    if (event.ctrlKey) {
      const toggled = new Set(current);
      if (!toggled.delete(id)) toggled.add(id);
      select(toggled);
    } else {
      select(new Set([id]));
    }
  };

  const cut = new Set(clip?.cutIds ?? []);
  const button = "h-row rounded-ctrl border border-ctrl-border bg-ctrl px-3 hover:bg-hover";

  return (
    <div className="flex h-full flex-col bg-mica text-fg">
      <header
        className="flex h-titlebar shrink-0 items-center justify-between pl-4 text-small text-fg-2"
        onPointerDown={(event) => {
          if (event.button === 0 && event.target === event.currentTarget) {
            void commands.spikeWindow("move");
          }
        }}
      >
        {strings.spike.windowTitle}
        <button
          type="button"
          aria-label={strings.spike.close}
          className="h-titlebar w-caption-btn hover:bg-red hover:text-on-accent"
          onClick={() => void commands.spikeWindow("close")}
        >
          ✕
        </button>
      </header>

      <main className="flex min-h-0 flex-1 flex-col gap-3 bg-pane p-4">
        <div className="flex items-center gap-2">
          <h1 className="mr-2 text-title">{strings.spike.title}</h1>
          <button type="button" className={button} onClick={() => void copy(false)}>
            {strings.spike.copy}
          </button>
          <button type="button" className={button} onClick={() => void copy(true)}>
            {strings.spike.cut}
          </button>
          <button type="button" className={button} onClick={() => void paste()}>
            {strings.spike.paste}
          </button>
          <button
            type="button"
            className={button}
            onClick={() => select(new Set(entries.map((entry) => entry.id)))}
          >
            {strings.spike.selectAll}
          </button>
          <button type="button" className={button} onClick={() => void refresh()}>
            {strings.spike.refresh}
          </button>
        </div>
        <p className="text-fg-2">{strings.spike.hint}</p>

        <div
          role="listbox"
          aria-multiselectable
          className="min-h-0 overflow-auto rounded-ctrl border border-stroke"
        >
          {entries.map((entry) => (
            <div
              key={entry.id}
              role="option"
              tabIndex={-1}
              aria-selected={selected.has(entry.id)}
              aria-label={entry.name}
              data-cut={cut.has(entry.id) || undefined}
              className={`flex h-row items-center gap-3 px-3 ${
                selected.has(entry.id) ? "bg-sel" : "hover:bg-hover"
              } ${cut.has(entry.id) ? "opacity-50" : ""}`}
              onClick={(event) => onRowClick(event, entry.id)}
              onKeyDown={(event) => {
                if (event.key === "Enter" || event.key === " ") select(new Set([entry.id]));
              }}
            >
              <span className="flex-1 truncate">{entry.name}</span>
              <span className="w-24 text-right text-fg-2 tabular-nums">
                {entry.isDir ? strings.spike.folder : formatSize(entry.size)}
              </span>
            </div>
          ))}
        </div>

        <output className="block" data-testid="clip-status">
          {status}
        </output>
        {clip && (
          <section className="text-small text-fg-2" data-testid="clip-state">
            <p>{strings.spike.clipboard(clip)}</p>
            {clip.formats.length > 0 && <p>{strings.spike.formats(clip.formats)}</p>}
          </section>
        )}
      </main>
    </div>
  );
}

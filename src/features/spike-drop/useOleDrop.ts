import { useCallback, useEffect, useRef, useState } from "react";
import {
  commands,
  type DndDone,
  type DndEnter,
  type DndKeys,
  type DndStats,
  type Effect,
  events,
} from "../../ipc";
import { strings } from "../../strings";
import { type DropHit, dropHitAt } from "./target";

/**
 * Spike #14: the UI half of an OLE drop. Rust's drop target can't see the page, and the page
 * gets no mouse events during an OLE drag, so Rust sends each new point (`dnd:over`) and this
 * hook answers with the drop target under it (`dnd_answer`). Rust returns the effect for the
 * last answer to OLE synchronously, a frame behind the cursor.
 *
 * The same events drive the UI-only behavior: highlighting, and opening a tab or folder after
 * the cursor rests on it (PRD: 0.7 s).
 */

/** PRD «Drag & Drop»: rest on a tab or folder this long to open it. */
export const DWELL_MS = 700;

type Point = { session: number; seq: number; x: number; y: number; keys: DndKeys };

export type DropLogEntry = {
  dropId: number;
  requested: Effect;
  items: number;
  readMs: number;
  takeMs: number;
  state: "copying" | "refused" | "menu" | "cancelled" | "done";
  done?: DndDone;
};

export type DropMenu = { dropId: number; x: number; y: number };

export function useOleDrop(onDwell: (hit: DropHit) => void, onDone: (done: DndDone) => void) {
  const [drag, setDrag] = useState<DndEnter | null>(null);
  const [hover, setHover] = useState<DropHit | null>(null);
  const [keys, setKeys] = useState<DndKeys | null>(null);
  const [stats, setStats] = useState<DndStats | null>(null);
  const [log, setLog] = useState<DropLogEntry[]>([]);
  const [menu, setMenu] = useState<DropMenu | null>(null);
  const menuRef = useRef<DropMenu | null>(null);
  const [domDrags, setDomDrags] = useState(0);
  const last = useRef<Point | null>(null);
  const itemCount = useRef(0);
  const hoverKey = useRef<string | null>(null);
  const dwell = useRef<number | undefined>(undefined);
  const callbacks = useRef({ onDwell, onDone });
  callbacks.current = { onDwell, onDone };

  const stopDwell = useCallback(() => {
    window.clearTimeout(dwell.current);
    dwell.current = undefined;
  }, []);

  /** Hit-tests the last point and tells Rust what is there. */
  const answer = useCallback(() => {
    const point = last.current;
    if (!point) return;
    const hit = dropHitAt(point.x, point.y);
    if ((hit?.key ?? null) !== hoverKey.current) {
      hoverKey.current = hit?.key ?? null;
      setHover(hit);
      stopDwell();
      if (hit && (hit.kind === "tab" || hit.kind === "row")) {
        dwell.current = window.setTimeout(() => {
          callbacks.current.onDwell(hit);
          // The DOM under the cursor changed; answer again without waiting for a mouse move.
          requestAnimationFrame(() => requestAnimationFrame(() => answerRef.current()));
        }, DWELL_MS);
      }
    }
    void commands.dndAnswer({
      session: point.session,
      seq: point.seq,
      target: hit ? { id: hit.folder, kind: hit.kind, insert: hit.name } : null,
      labels: strings.drop.labels(itemCount.current),
    });
  }, [stopDwell]);
  const answerRef = useRef(answer);
  answerRef.current = answer;

  const end = useCallback(() => {
    last.current = null;
    hoverKey.current = null;
    stopDwell();
    setHover(null);
    setKeys(null);
  }, [stopDwell]);

  useEffect(() => {
    const listeners = [
      events.dndEnter.listen(({ payload }) => {
        // Explorer offers CF_HDROP and FileGroupDescriptorW for the same files: count one.
        itemCount.current = payload.files > 0 ? payload.files : payload.virtualFiles;
        last.current = payload;
        setDrag(payload);
        setKeys(payload.keys);
        answerRef.current();
      }),
      events.dndOver.listen(({ payload }) => {
        if (last.current && payload.session !== last.current.session) return;
        last.current = payload;
        setKeys(payload.keys);
        answerRef.current();
      }),
      events.dndLeave.listen(() => end()),
      events.dndDrop.listen(({ payload }) => {
        end();
        const state = payload.target === null ? "refused" : payload.menu ? "menu" : "copying";
        setLog((list) => [
          {
            dropId: payload.dropId,
            requested: payload.requested,
            items: payload.items,
            readMs: payload.readMs ?? 0,
            takeMs: payload.takeMs ?? 0,
            state,
          },
          ...list.slice(0, 19),
        ]);
        if (payload.menu) {
          menuRef.current = { dropId: payload.dropId, x: payload.x, y: payload.y };
          setMenu(menuRef.current);
        }
      }),
      events.dndDone.listen(({ payload }) => {
        setLog((list) =>
          list.map((entry) =>
            entry.dropId === payload.dropId ? { ...entry, state: "done", done: payload } : entry,
          ),
        );
        callbacks.current.onDone(payload);
      }),
      events.dndStats.listen(({ payload }) => setStats(payload)),
    ];
    return () => {
      for (const listener of listeners) void listener.then((unlisten) => unlisten());
    };
  }, [end]);

  // Diagnostics: HTML5 drag events reach the page only if WebView2's own drop target is still
  // registered, which is what `dnd_install` removes.
  useEffect(() => {
    const count = (event: DragEvent) => {
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "none";
      setDomDrags((n) => n + 1);
    };
    document.addEventListener("dragover", count);
    document.addEventListener("drop", count);
    return () => {
      document.removeEventListener("dragover", count);
      document.removeEventListener("drop", count);
    };
  }, []);

  const choose = useCallback((choice: Effect | null) => {
    const current = menuRef.current;
    if (!current) return;
    menuRef.current = null;
    setMenu(null);
    void commands.dndChoose(current.dropId, choice);
    setLog((list) =>
      list.map((entry) =>
        entry.dropId === current.dropId
          ? { ...entry, state: choice ? "copying" : "cancelled" }
          : entry,
      ),
    );
  }, []);

  return { drag, hover, keys, stats, log, menu, choose, domDrags };
}

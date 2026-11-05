import {
  type MouseEvent,
  type PointerEvent,
  type RefObject,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";
import type { Folder, Selection } from "../../stores/folder";
import { selectionOf } from "./commands";
import {
  clear,
  clickRow,
  indexOf,
  type MarqueeMode,
  type Modifiers,
  marquee,
  marqueeMode,
  NO_MODIFIERS,
  rowsCrossed,
} from "./model";

/** Moving this far with the button down makes a press a marquee (Windows' SM_CXDRAG). */
const DRAG_PX = 4;
/** Auto-scroll per frame: this much of the distance past the edge, at least 1 px, at most the max. */
const SCROLL_RATE = 0.4;
const SCROLL_MAX_PX = 48;

/** The marquee as drawn, in CSS px from the first row's top left. */
export interface MarqueeRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

interface Press {
  pointer: number;
  /** Where it started, on the screen and from the first row's top left. */
  clientX: number;
  clientY: number;
  x: number;
  y: number;
  modifiers: Modifiers;
  /** The row clicked when the button comes up without a drag. */
  clickOnUp: number | undefined;
  canMarquee: boolean;
  dragging: boolean;
  /** The selection the marquee started from, and how it changes it. */
  initial: Selection;
  mode: MarqueeMode;
  /** The rows the marquee crosses, as `[first, last]` indexes. */
  crossed: [number, number];
}

interface Options {
  folder: Folder;
  /** The rows the list shows. */
  rows: Uint32Array;
  rowHeight: number;
  /** The list's scroll element (the grid) and the element the rows are positioned in. */
  scroller: RefObject<HTMLElement | null>;
  body: RefObject<HTMLElement | null>;
  /** From the scroll element's top to the first row's: the column header and the gap under it. */
  margin: number;
}

/**
 * Mouse selection as in Explorer's Details view:
 * - A press on a row's icon or name selects it at once (Ctrl toggles, Shift selects the range);
 *   on a row already selected the click lands on release, so a drag can take the selection.
 * - A press anywhere else, in a row's other columns or below the rows, can become a marquee: it
 *   selects the rows it crosses (Ctrl toggles them, Shift adds them) and scrolls the list when
 *   it is dragged past the top or bottom. Released without a drag, it clicks the row, or on empty
 *   space clears the selection.
 * - A right press on a row not selected selects it alone.
 */
export function usePointerSelection({ folder, rows, rowHeight, scroller, body, margin }: Options) {
  const [rect, setRect] = useState<MarqueeRect>();
  const press = useRef<Press | undefined>(undefined);
  const pointer = useRef({ x: 0, y: 0 });
  /** A drag just ended: the click that follows isn't one. */
  const dragged = useRef(false);
  const latest = useRef({ rows, rowHeight, margin });
  useEffect(() => {
    latest.current = { rows, rowHeight, margin };
  });

  /** Follows the pointer: the rows crossed and the drawn rectangle. */
  const update = useCallback(() => {
    const p = press.current;
    const view = scroller.current;
    const area = body.current?.getBoundingClientRect();
    if (!p?.dragging || !view || !area) {
      return;
    }
    const { rows, rowHeight, margin } = latest.current;
    const x = pointer.current.x - area.left;
    const y = pointer.current.y - area.top;
    const crossed = rowsCrossed(p.y, y, rowHeight, rows.length);
    if (crossed[0] !== p.crossed[0] || crossed[1] !== p.crossed[1]) {
      p.crossed = crossed;
      folder.setSelection(marquee(p.initial, rows, crossed[0], crossed[1], p.mode));
    }
    // Drawn inside the scrolled content only, so it never makes the list longer or wider.
    const bottom = Math.max(rows.length * rowHeight, view.clientHeight - margin);
    const right = body.current?.offsetWidth ?? 0;
    const clamp = (value: number, max: number) => Math.min(Math.max(value, 0), max);
    const left = clamp(Math.min(p.x, x), right);
    const top = clamp(Math.min(p.y, y), bottom);
    setRect({
      left,
      top,
      width: clamp(Math.max(p.x, x), right) - left,
      height: clamp(Math.max(p.y, y), bottom) - top,
    });
  }, [folder, scroller, body]);

  // While a marquee is dragged, the list scrolls when the pointer is past the top or bottom of
  // the rows in view, faster the further past.
  const dragging = rect !== undefined;
  useEffect(() => {
    if (!dragging) {
      return;
    }
    let frame = requestAnimationFrame(function tick() {
      const view = scroller.current;
      if (view) {
        const box = view.getBoundingClientRect();
        const top = box.top + latest.current.margin;
        const bottom = box.top + view.clientHeight;
        const y = pointer.current.y;
        const past = y < top ? y - top : y > bottom ? y - bottom : 0;
        const step = Math.min(Math.ceil(Math.abs(past) * SCROLL_RATE), SCROLL_MAX_PX);
        const before = view.scrollTop;
        view.scrollTop += Math.sign(past) * step;
        if (view.scrollTop !== before) {
          update();
        }
      }
      frame = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(frame);
  }, [dragging, scroller, update]);

  const end = useCallback(() => {
    press.current = undefined;
    setRect(undefined);
  }, []);

  const onPointerDown = (event: PointerEvent<HTMLElement>) => {
    const target = event.target as Element;
    const area = body.current;
    // The scroll bars are the grid itself; the column header has its own clicks.
    if (target === event.currentTarget || !area || isHeader(target)) {
      return;
    }
    const modifiers = { ctrl: event.ctrlKey, shift: event.shiftKey };
    const selection = selectionOf(folder.store.getState());
    const row = target.closest<HTMLElement>("[data-id]");
    const id = row ? Number(row.dataset.id) : undefined;
    const index = indexOf(rows, id);

    if (event.button === 2) {
      if (index !== undefined && id !== undefined && !selection.selected.has(id)) {
        folder.setSelection(clickRow(selection, rows, index, NO_MODIFIERS));
      } else if (index === undefined && !modifiers.ctrl && !modifiers.shift) {
        folder.setSelection(clear(selection));
      }
      return;
    }
    if (event.button !== 0) {
      return;
    }

    const onItem = index !== undefined && target.closest("[data-item]") !== null;
    let clickOnUp: number | undefined;
    if (index !== undefined && id !== undefined) {
      const selected = selection.selected.has(id);
      if (onItem && !(selected && !modifiers.shift)) {
        folder.setSelection(clickRow(selection, rows, index, modifiers));
      } else {
        clickOnUp = id;
      }
    } else if (!modifiers.ctrl && !modifiers.shift) {
      folder.setSelection(clear(selection));
    }

    const box = area.getBoundingClientRect();
    press.current = {
      pointer: event.pointerId,
      clientX: event.clientX,
      clientY: event.clientY,
      x: event.clientX - box.left,
      y: event.clientY - box.top,
      modifiers,
      clickOnUp,
      canMarquee: !onItem,
      dragging: false,
      initial: selection,
      mode: marqueeMode(modifiers),
      crossed: [0, -1],
    };
    pointer.current = { x: event.clientX, y: event.clientY };
    dragged.current = false;
  };

  const onPointerMove = (event: PointerEvent<HTMLElement>) => {
    const p = press.current;
    if (!p || event.pointerId !== p.pointer) {
      return;
    }
    pointer.current = { x: event.clientX, y: event.clientY };
    if (!p.dragging) {
      const far =
        Math.abs(event.clientX - p.clientX) >= DRAG_PX ||
        Math.abs(event.clientY - p.clientY) >= DRAG_PX;
      if (!p.canMarquee || !far) {
        return;
      }
      p.dragging = true;
      p.initial = selectionOf(folder.store.getState());
      // Only now: a captured pointer's click and double click go to the list, not to the row.
      event.currentTarget.setPointerCapture?.(event.pointerId);
    }
    update();
  };

  const onPointerUp = (event: PointerEvent<HTMLElement>) => {
    const p = press.current;
    if (!p || event.pointerId !== p.pointer) {
      return;
    }
    end();
    dragged.current = p.dragging;
    if (!p.dragging && p.clickOnUp !== undefined) {
      const index = indexOf(latest.current.rows, p.clickOnUp);
      if (index !== undefined) {
        const selection = selectionOf(folder.store.getState());
        folder.setSelection(clickRow(selection, latest.current.rows, index, p.modifiers));
      }
    }
  };

  /** Whether a click is the end of a marquee, not a click on a row. */
  const wasDrag = (event: MouseEvent) => {
    const was = dragged.current;
    dragged.current = false;
    return was || event.button !== 0;
  };

  return {
    rect,
    wasDrag,
    handlers: {
      onPointerDown,
      onPointerMove,
      onPointerUp,
      onPointerCancel: end,
      onLostPointerCapture: (event: PointerEvent<HTMLElement>) => {
        if (press.current?.pointer === event.pointerId) {
          end();
        }
      },
      /** Wheel scrolling while dragging moves the marquee's end with the rows. */
      onScroll: update,
    },
  };
}

function isHeader(target: Element): boolean {
  const row = target.closest('[role="row"]');
  return row !== null && !row.hasAttribute("data-id");
}

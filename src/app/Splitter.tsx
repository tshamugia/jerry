import { type PointerEvent, useRef } from "react";

interface Props {
  label: string;
  /** Where the handle sits in its zone, e.g. `-left-1` to center it on the zone's left edge. */
  className: string;
  /** The pointer moved to `x` (client pixels) while dragging. */
  onDrag: (x: number) => void;
  /** The drag ended: save the size. */
  onDrop: () => void;
}

/**
 * The invisible, 9 px wide handle on a zone's edge that resizes it by drag. It holds the pointer
 * while dragging, so the drag keeps going over the panes and outside the window.
 */
export function Splitter({ label, className, onDrag, onDrop }: Props) {
  const dragging = useRef(false);

  const end = (event: PointerEvent<HTMLDivElement>) => {
    if (!dragging.current) {
      return;
    }
    dragging.current = false;
    event.currentTarget.releasePointerCapture?.(event.pointerId);
    onDrop();
  };

  return (
    // biome-ignore lint/a11y/useSemanticElements: a draggable window splitter; <hr> can't be one.
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      className={`no-drag absolute inset-y-0 z-10 w-2.25 cursor-col-resize ${className}`}
      onPointerDown={(event) => {
        if (event.button !== 0) {
          return;
        }
        // A drag on the left pane's edge doesn't make it the active pane.
        event.stopPropagation();
        event.preventDefault();
        dragging.current = true;
        event.currentTarget.setPointerCapture?.(event.pointerId);
      }}
      onPointerMove={(event) => {
        if (dragging.current) {
          onDrag(event.clientX);
        }
      }}
      onPointerUp={end}
      onPointerCancel={end}
    />
  );
}

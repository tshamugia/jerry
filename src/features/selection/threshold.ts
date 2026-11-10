import { commands, type DragThreshold } from "../../ipc";

/** Windows' default (SM_CXDRAG, SM_CYDRAG), until the core tells the user's own. */
const DEFAULT: DragThreshold = { x: 4, y: 4 };

let threshold = DEFAULT;
let asked = false;

/**
 * Whether a press has moved far enough with the button down to be a drag, not a click: a marquee,
 * or files dragged out of Jerry. Windows' drag threshold, in CSS px, which the core reads once
 * (`drag_threshold`); 4 px until it answers.
 */
export function pastDragThreshold(dx: number, dy: number): boolean {
  if (!asked) {
    asked = true;
    void readThreshold();
  }
  return Math.abs(dx) >= threshold.x || Math.abs(dy) >= threshold.y;
}

async function readThreshold(): Promise<void> {
  try {
    const read = await commands.dragThreshold();
    if (read && read.x > 0 && read.y > 0) {
      threshold = read;
    }
  } catch {
    // Keeps Windows' default.
  }
}

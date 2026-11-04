import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  type CaptionButton,
  type CaptionButtonRect,
  commands,
  type FrameState,
  stream,
} from "../ipc";
import { strings } from "../strings";

const buttons: readonly CaptionButton[] = ["minimize", "maximize", "close"];

/** Until the core reports: a restored, active window with nothing under the mouse. */
const restored: FrameState = { hovered: null, pressed: null, maximized: false, active: true };

/** How long the mouse rests on a button before its tooltip shows, as for Windows' own. */
export const TOOLTIP_DELAY_MS = 500;

/**
 * Minimize, maximize (restore when maximized) and close, 46×40 at the title bar's top right
 * (ADR-0004). The mouse doesn't reach these elements: a native input window lies over them,
 * because Snap Layouts need `HTMAXBUTTON` from a real hit-test. It runs the clicks and reports
 * hover and pressed state, which is drawn here. The buttons are still real buttons with names,
 * out of the Tab order, so screen readers can press them; that goes through `titleBarPress`.
 * Without the input window (a window with Windows' own frame), the page gets the mouse and the
 * same buttons work, with CSS hover.
 */
export const CaptionButtons = memo(function CaptionButtons() {
  const state = useFrameState();
  const ref = useReportedButtons();
  const tooltip = useTooltip(state);

  return (
    <div ref={ref} className="no-drag absolute top-0 right-0 flex">
      {buttons.map((button) => (
        <button
          key={button}
          type="button"
          tabIndex={-1}
          data-caption={button}
          data-look={look(button, state)}
          aria-label={label(button, state.maximized)}
          onClick={() => void commands.titleBarPress(button)}
          className={`relative flex h-titlebar w-caption-btn items-center justify-center text-fg data-[look=inactive]:text-disabled ${
            button === "close"
              ? "hover:bg-caption-close hover:text-on-caption-close active:bg-caption-close-pressed data-[look=hover]:bg-caption-close data-[look=hover]:text-on-caption-close data-[look=pressed]:bg-caption-close-pressed data-[look=pressed]:text-on-caption-close"
              : "hover:bg-subtle-hover active:bg-subtle-pressed data-[look=hover]:bg-subtle-hover data-[look=pressed]:bg-subtle-pressed"
          }`}
        >
          <span aria-hidden="true" className="font-glyph text-glyph leading-none">
            {glyph(button, state.maximized)}
          </span>
          {tooltip === button && (
            // The button's name says the same, so screen readers skip it.
            <span
              aria-hidden="true"
              data-tooltip
              className="pointer-events-none absolute top-full right-0 z-10 mt-1 whitespace-nowrap rounded-ctrl border border-flyout-border bg-flyout px-2 py-1 text-fg text-small shadow-flyout"
            >
              {label(button, state.maximized)}
            </span>
          )}
        </button>
      ))}
    </div>
  );
});

type Look = "rest" | "inactive" | "hover" | "pressed";

/** How a button looks, as Windows draws its own. */
export function look(button: CaptionButton, state: FrameState): Look {
  if (state.pressed === button) {
    // Held down and dragged off: back to rest until the mouse returns.
    return state.hovered === button ? "pressed" : "rest";
  }
  // While one button is held, the others don't light up.
  if (state.hovered === button && !state.pressed) {
    return "hover";
  }
  return state.active ? "rest" : "inactive";
}

function label(button: CaptionButton, maximized: boolean): string {
  if (button === "minimize") {
    return strings.titleBar.minimize;
  }
  if (button === "close") {
    return strings.titleBar.close;
  }
  return maximized ? strings.titleBar.restore : strings.titleBar.maximize;
}

/** Windows' own caption glyphs (Segoe Fluent Icons; Segoe MDL2 Assets on Windows 10). */
function glyph(button: CaptionButton, maximized: boolean): string {
  if (button === "minimize") {
    return "";
  }
  if (button === "close") {
    return "";
  }
  return maximized ? "" : "";
}

/** The window's frame state, streamed from the core. */
function useFrameState(): FrameState {
  const [state, setState] = useState(restored);
  useEffect(() => {
    let live = true;
    const updates = stream<FrameState>((next) => {
      if (live) {
        setState(next);
      }
    });
    // A second attach (StrictMode, a reload) replaces the first one's stream.
    void commands.titleBarAttach(updates);
    return () => {
      live = false;
    };
  }, []);
  return state;
}

/**
 * Tells the native hit-test where the buttons are. They are right-anchored, so the core keeps
 * them in place when the window resizes and rescales them when the DPI changes; a report goes
 * out only when the page moves them.
 */
function useReportedButtons() {
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const group = ref.current;
    if (!group) {
      return;
    }
    let last = "";
    let frame = 0;
    const report = () => {
      frame = 0;
      const viewport = document.documentElement.clientWidth;
      const rects = Array.from(group.querySelectorAll<HTMLElement>("[data-caption]"), (element) => {
        const rect = element.getBoundingClientRect();
        return {
          button: element.dataset.caption as CaptionButton,
          right: viewport - rect.right,
          top: rect.top,
          width: rect.width,
          height: rect.height,
        } satisfies CaptionButtonRect;
      });
      const key = JSON.stringify(rects);
      if (key !== last) {
        last = key;
        void commands.titleBarSetButtons(rects);
      }
    };
    const schedule = () => {
      if (!frame) {
        frame = requestAnimationFrame(report);
      }
    };
    report();
    window.addEventListener("resize", schedule);
    return () => {
      window.removeEventListener("resize", schedule);
      cancelAnimationFrame(frame);
    };
  }, []);
  return ref;
}

/** The button whose tooltip shows: the hovered one, once the mouse rested on it. */
function useTooltip(state: FrameState): CaptionButton | null {
  const [shown, setShown] = useState<CaptionButton | null>(null);
  const resting = state.pressed ? null : state.hovered;
  useEffect(() => {
    setShown(null);
    if (!resting) {
      return;
    }
    const timer = setTimeout(() => setShown(resting), TOOLTIP_DELAY_MS);
    return () => clearTimeout(timer);
  }, [resting]);
  return shown;
}

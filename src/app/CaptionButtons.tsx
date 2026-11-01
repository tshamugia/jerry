import { useLayoutEffect, useRef } from "react";
import { type CaptionButton, type CaptionButtonRect, commands, type FrameState } from "../ipc";
import { strings } from "../strings";

const buttons: readonly CaptionButton[] = ["minimize", "maximize", "close"];

/**
 * Minimize, maximize and close, 46×40 at the right of the title bar. The mouse never reaches
 * these elements: a native input window sits on top of them (Snap Layouts need `HTMAXBUTTON`
 * from a real hit-test), runs the clicks and reports hover and pressed state, which is drawn
 * here. So they are not buttons to the DOM; the native window owns the interaction.
 */
export function CaptionButtons({ state }: { state: FrameState }) {
  const ref = useRef<HTMLDivElement>(null);

  // Report where the buttons are. They are right-anchored, so the native side keeps them in
  // place during a resize and rescales them on a DPI change; a report is needed only when the
  // layout itself moves them.
  useLayoutEffect(() => {
    const group = ref.current;
    if (!group) return;
    let last = "";
    let frame = 0;
    const report = () => {
      frame = 0;
      const viewport = document.documentElement.clientWidth;
      const rects = Array.from(group.children, (child): CaptionButtonRect => {
        const r = child.getBoundingClientRect();
        const button = (child as HTMLElement).dataset.caption as CaptionButton;
        return { button, right: viewport - r.right, top: r.top, width: r.width, height: r.height };
      });
      const key = JSON.stringify(rects);
      if (key === last) return;
      last = key;
      void commands.titleBarSetButtons(rects);
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(report);
    };
    report();
    const observer = new ResizeObserver(schedule);
    observer.observe(group);
    observer.observe(document.documentElement);
    return () => {
      observer.disconnect();
      cancelAnimationFrame(frame);
    };
  }, []);

  return (
    <div ref={ref} className="no-drag flex h-titlebar shrink-0" aria-hidden="true">
      {buttons.map((button) => (
        <div
          key={button}
          data-caption={button}
          data-look={look(button, state)}
          data-label={label(button, state.maximized)}
          className={
            button === "close"
              ? "flex h-titlebar w-caption-btn items-center justify-center data-[look=hover]:bg-caption-close data-[look=pressed]:bg-caption-close-pressed data-[look=hover]:text-on-caption-close data-[look=pressed]:text-on-caption-close data-[look=inactive]:text-disabled"
              : "flex h-titlebar w-caption-btn items-center justify-center data-[look=hover]:bg-subtle-hover data-[look=pressed]:bg-subtle-pressed data-[look=inactive]:text-disabled"
          }
        >
          <Glyph button={button} maximized={state.maximized} />
        </div>
      ))}
    </div>
  );
}

type Look = "rest" | "inactive" | "hover" | "pressed";

function look(button: CaptionButton, state: FrameState): Look {
  if (state.pressed === button) return state.hovered === button ? "pressed" : "rest";
  // While one button is held, the others don't light up (as in native windows).
  if (state.hovered === button && !state.pressed) return "hover";
  return state.active ? "rest" : "inactive";
}

/** The name a native tooltip or UI Automation would give the button (#23). */
function label(button: CaptionButton, maximized: boolean): string {
  if (button === "minimize") return strings.titleBar.minimize;
  if (button === "close") return strings.titleBar.close;
  return maximized ? strings.titleBar.restore : strings.titleBar.maximize;
}

/** Segoe Fluent-style 10×10 glyphs from the design, drawn with the text color. */
function Glyph({ button, maximized }: { button: CaptionButton; maximized: boolean }) {
  const common = {
    width: 10,
    height: 10,
    viewBox: "0 0 10 10",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1,
  } as const;
  if (button === "minimize") {
    return (
      <svg {...common} aria-hidden="true">
        <path d="M0 5.5h10" />
      </svg>
    );
  }
  if (button === "close") {
    return (
      <svg {...common} aria-hidden="true">
        <path d="M0.5 0.5l9 9M9.5 0.5l-9 9" />
      </svg>
    );
  }
  return maximized ? (
    <svg {...common} aria-hidden="true">
      <rect x="0.5" y="2.5" width="7" height="7" rx="1" />
      <path d="M2.5 2.5v-1a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-1" />
    </svg>
  ) : (
    <svg {...common} aria-hidden="true">
      <rect x="0.5" y="0.5" width="9" height="9" rx="1.5" />
    </svg>
  );
}

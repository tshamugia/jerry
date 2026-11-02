import { strings } from "../../strings";
import { FONT_STACKS, type FontChoice, useGeorgian } from "./store";

const FONTS = Object.keys(FONT_STACKS) as FontChoice[];

/** Spike #18 readout: the font stack in use, the native window title and the last shortcuts. */
export function Readout() {
  const s = strings.georgian;
  const font = useGeorgian((g) => g.font);
  const setFont = useGeorgian((g) => g.setFont);
  const windowTitle = useGeorgian((g) => g.windowTitle);
  const fired = useGeorgian((g) => g.fired);

  return (
    <>
      <h2 className="m-0 font-semibold text-title">{s.title}</h2>
      <section className="flex flex-col gap-1.5">
        <h3 className="m-0 font-normal text-fg-2 text-small">{s.font}</h3>
        <div className="flex flex-col gap-1">
          {FONTS.map((choice) => (
            <button
              key={choice}
              type="button"
              aria-pressed={font === choice}
              onClick={() => setFont(choice)}
              className={`h-toolbar-btn rounded-ctrl border border-ctrl-border px-2 text-left ${
                font === choice ? "bg-nav-sel" : "bg-ctrl hover:bg-hover"
              }`}
            >
              {s.fonts[choice]}
            </button>
          ))}
        </div>
      </section>
      <section className="flex flex-col gap-1">
        <h3 className="m-0 font-normal text-fg-2 text-small">{s.windowTitle}</h3>
        <p data-window-title className="m-0 truncate">
          {windowTitle ?? "…"}
        </p>
      </section>
      <section className="flex min-h-0 flex-col gap-1">
        <h3 className="m-0 font-normal text-fg-2 text-small">{s.lastKeys}</h3>
        {fired.length === 0 ? (
          <p className="m-0 text-fg-3 text-small">{s.noKeys}</p>
        ) : (
          <ol className="m-0 flex flex-col gap-0.5 p-0 text-small tabular-nums">
            {fired.slice(-8).map((f) => (
              <li key={f.seq} className="list-none">
                {s.fired(f.title, f.shortcut, f.key, f.code)}
              </li>
            ))}
          </ol>
        )}
      </section>
      <p className="m-0 text-fg-3 text-small">{s.hint}</p>
    </>
  );
}

import { useMemo } from "react";
import { useKeymap } from "../stores/keymap";
import { closeOverlay, useOverlay } from "../stores/overlay";
import { strings } from "../strings";
import { Dialog } from "../ui/Dialog";
import { Kbd } from "../ui/Kbd";
import { shortcutTable } from "./table";

/** The shortcut table (F1 or `?`): every command's shortcut, by group, from the registry. */
export function ShortcutTable() {
  const open = useOverlay((state) => state.overlay === "shortcuts");
  const keymap = useKeymap((state) => state.keymap);
  const groups = useMemo(() => shortcutTable(keymap), [keymap]);

  return (
    <Dialog
      open={open}
      onClose={closeOverlay}
      title={strings.shortcutTable.title}
      closeHint={strings.shortcutTable.close}
    >
      <div className="columns-2 gap-x-1.5 p-1.5">
        {groups.map((group) => (
          <section
            key={group.group}
            aria-labelledby={`shortcuts-${group.group}`}
            className="break-inside-avoid pb-1.5"
          >
            <h3
              id={`shortcuts-${group.group}`}
              className="flex h-6.5 items-center px-2.5 font-semibold text-caption text-fg-3"
            >
              {group.title}
            </h3>
            <dl>
              {group.rows.map((row) => (
                <div key={row.id} className="flex h-7 items-center gap-3 px-2.5">
                  <dt className="min-w-0 flex-1 truncate">{row.title}</dt>
                  <dd className="flex shrink-0 gap-1">
                    {row.keys.map((key) => (
                      <Kbd key={key}>{key}</Kbd>
                    ))}
                  </dd>
                </div>
              ))}
            </dl>
          </section>
        ))}
      </div>
    </Dialog>
  );
}

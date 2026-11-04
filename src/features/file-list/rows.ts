import { useMemo } from "react";
import { type Folder, type ShownRows, useFolder } from "../../stores/folder";
import { useSetting } from "../../stores/settings";

/**
 * The rows `folder`'s list shows (hidden entries and the quick filter applied) and how many there
 * would be without the filter, kept up to date. Components that call it share one pass over the
 * entries (`Folder.shown`).
 */
export function useShownRows(folder: Folder): ShownRows {
  const status = useFolder(folder, (state) => state.status);
  const entries = useFolder(folder, (state) => state.entries);
  const order = useFolder(folder, (state) => state.order);
  const version = useFolder(folder, (state) => state.version);
  const filter = useFolder(folder, (state) => state.filter);
  const showHidden = useSetting("showHidden") ?? false;
  // `entries` and `order` change in place: `version` says when. `shown` reads them all.
  // biome-ignore lint/correctness/useExhaustiveDependencies: each is an input of `shown`.
  return useMemo(
    () => folder.shown(showHidden),
    [folder, status, entries, order, version, filter, showHidden],
  );
}

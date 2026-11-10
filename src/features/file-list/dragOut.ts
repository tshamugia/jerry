import { commands } from "../../ipc";
import type { Folder } from "../../stores/folder";
import { showToast } from "../../stores/toasts";
import { errorText, strings } from "../../strings";

/**
 * Drags the selected rows of `folder` (those among `rows`, in display order) out of Jerry, as a
 * drag from Explorer (PRD «Drag & Drop», #58): Windows runs it with its own drag image, and the
 * program they are dropped on copies, moves, attaches or opens them. Resolves once they are
 * dropped or the drag is cancelled. A file moved away leaves the list through the folder's
 * watcher. A file gone since the folder was read says so in a toast.
 */
export async function dragOut(folder: Folder, rows: Uint32Array): Promise<void> {
  const { listing, selected } = folder.store.getState();
  const ids = Array.from(rows).filter((id) => selected.has(id));
  if (listing === undefined || ids.length === 0) {
    return;
  }
  const result = await commands.dragStart(listing, ids);
  if (result.status === "error") {
    showToast({ title: strings.fileList.dragFailed, detail: errorText(result.error) });
  }
}

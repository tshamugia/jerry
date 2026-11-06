import { commands, events, type LaunchOpen, subscribe } from "../../ipc";
import { openInNewTab } from "../../stores/tabs";
import { showToast } from "../../stores/toasts";
import { useWindow } from "../../stores/window";
import { errorText, strings } from "../../strings";

/**
 * Opens what starts of `jerry.exe <path>` leave for this window (#43): at start, and whenever
 * Jerry is started again while it runs. The core keeps them until the page takes them, so none
 * is lost while the page loads. Waits for `restored` (the session's tabs) first, so the new tabs
 * go after them. Returns a function that stops it.
 */
export function followLaunches(restored: Promise<unknown>): () => void {
  const take = () => {
    void restored.then(async () => {
      for (const open of await commands.launchTake()) {
        openLaunch(open);
      }
    });
  };
  return subscribe(events.launchOpened, take, take);
}

/**
 * "Exit Jerry": quits, even while Jerry keeps running in the background (ADR-0005), where closing
 * the last window only hides it. The core writes the session first.
 */
export function exitJerry(): void {
  void commands.appExit();
}

/**
 * A start's path in a new tab of the active pane, shown, with the file it named selected; a path
 * that leads nowhere says why.
 */
export function openLaunch(open: LaunchOpen): void {
  if (open.kind === "failed") {
    showToast({ title: strings.session.cantOpen(open.path), detail: errorText(open.error) });
    return;
  }
  openInNewTab(
    useWindow.getState().activePane,
    { kind: "folder", folder: open.folder },
    { restore: open.select === null ? undefined : { focus: open.select } },
  );
}

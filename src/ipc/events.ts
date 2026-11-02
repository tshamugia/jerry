/** An event from the core, as tauri-specta generates it (`events.settingsChanged`, ...). */
export interface CoreEvent<T> {
  listen(handler: (event: { payload: T }) => void): Promise<() => void>;
}

/**
 * Calls `handler` with the payload of every `event` the core emits, until the returned function
 * is called. Unlike `listen`, it stops right away even if the listener isn't registered yet, so
 * a React effect can return it as its cleanup.
 */
export function subscribe<T>(event: CoreEvent<T>, handler: (payload: T) => void): () => void {
  let stopped = false;
  let unlisten: (() => void) | undefined;
  void event
    .listen(({ payload }) => {
      if (!stopped) {
        handler(payload);
      }
    })
    .then((stop) => {
      if (stopped) {
        stop();
      } else {
        unlisten = stop;
      }
    });
  return () => {
    stopped = true;
    unlisten?.();
  };
}
